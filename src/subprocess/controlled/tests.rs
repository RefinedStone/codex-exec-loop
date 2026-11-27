use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

const FIXTURE_ENV: &str = "AKRA_TASK_VERIFICATION_PROCESS_FIXTURE";

#[test]
#[ignore = "child process fixture, invoked by bounded capture tests"]
fn controlled_child_fixture() {
    match std::env::var(FIXTURE_ENV).as_deref() {
        Ok("wait") => std::thread::sleep(Duration::from_secs(60)),
        Ok("output") => {
            let bytes = vec![b'x'; 1024 * 1024];
            std::io::stdout().write_all(&bytes).unwrap();
        }
        Ok("done") => {
            std::io::stdout().write_all(b"host-finished\n").unwrap();
        }
        _ => (),
    }
}

fn child(mode: &str) -> ManagedChild {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "subprocess::controlled::tests::controlled_child_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env(FIXTURE_ENV, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn(&mut command).unwrap()
}

#[test]
fn completed_capture_retains_exact_bytes_and_marks_both_streams_complete() {
    let result = wait_with_output_controlled(
        child("done"),
        Instant::now() + Duration::from_secs(10),
        &|| false,
        8192,
        8192,
    );
    assert!(
        matches!(result.termination, ControlledTermination::Exited(status) if status.success())
    );
    assert!(result.stdout_complete && result.stderr_complete);
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("host-finished\n")
    );
}

#[test]
fn cancellation_terminates_and_reaps_without_waiting_for_the_command_timeout() {
    let polls = AtomicUsize::new(0);
    let started = Instant::now();
    let result = wait_with_output_controlled(
        child("wait"),
        started + Duration::from_secs(10),
        &|| polls.fetch_add(1, Ordering::SeqCst) >= 2,
        8192,
        8192,
    );
    assert!(matches!(
        result.termination,
        ControlledTermination::Cancelled
    ));
    assert!(!result.stdout_complete && !result.stderr_complete);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(target_os = "macos")]
#[test]
fn cancellation_after_natural_exit_reaps_without_losing_the_cancelled_fact() {
    let child = child("done");
    let pid = libc::pid_t::try_from(child.id()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    // WNOWAIT fixes the disputed boundary: the native child has exited, but neither
    // ManagedChild nor the capture loop has reaped it. No sleep races against Git.
    loop {
        let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        };
        assert_eq!(result, 0, "waitid: {}", std::io::Error::last_os_error());
        if unsafe { info.si_pid() } == pid {
            assert_eq!(info.si_code, libc::CLD_EXITED);
            break;
        }
        assert!(Instant::now() < deadline, "native fixture did not exit");
        std::thread::sleep(POLL_INTERVAL);
    }
    assert!(!child.child_reaped);
    let result = wait_with_output_controlled(child, deadline, &|| true, 8192, 8192);
    assert!(matches!(
        result.termination,
        ControlledTermination::Cancelled
    ));
    assert!(!result.stdout_complete && !result.stderr_complete);
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD),
        "the original child must have been reaped"
    );
}

#[test]
fn deadline_stops_a_running_process_and_does_not_claim_complete_streams() {
    let started = Instant::now();
    let result = wait_with_output_controlled(
        child("wait"),
        started + Duration::from_millis(100),
        &|| false,
        8192,
        8192,
    );
    assert!(matches!(
        result.termination,
        ControlledTermination::TimedOut
    ));
    assert!(!result.stdout_complete && !result.stderr_complete);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn overflow_retains_only_an_actual_bounded_prefix_and_never_synthetic_padding() {
    let result = wait_with_output_controlled(
        child("output"),
        Instant::now() + Duration::from_secs(10),
        &|| false,
        4096,
        4096,
    );
    assert!(matches!(
        result.termination,
        ControlledTermination::OutputLimit
    ));
    assert_eq!(result.stdout.len(), 4096);
    assert!(!result.stdout.contains(&0));
    assert!(!result.stdout_complete);
}

#[cfg(windows)]
#[test]
fn cancellation_after_exit_signal_keeps_the_cancelled_fact() {
    let child = child("done");
    child.wait_for_progress(Duration::from_secs(10)).unwrap();
    assert_eq!(
        unsafe {
            windows_sys::Win32::System::Threading::WaitForSingleObject(
                child.child.as_raw_handle(),
                0,
            )
        },
        windows_sys::Win32::Foundation::WAIT_OBJECT_0,
        "the readiness wait must observe the real child exit"
    );
    assert!(
        !child.child_reaped,
        "waiting for readiness must not consume the capture loop's exit observation"
    );
    let result = wait_with_output_controlled(
        child,
        Instant::now() + Duration::from_secs(10),
        &|| true,
        8192,
        8192,
    );
    assert!(matches!(
        result.termination,
        ControlledTermination::Cancelled
    ));
    assert!(!result.stdout_complete && !result.stderr_complete);
}
