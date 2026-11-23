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
