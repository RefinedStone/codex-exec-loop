//! Real process/pipe/Job tests. Every dialog is a fake closure; no OS UI or key store is used.
use crate::adapter::inbound::vault_native_confirmation::run_with_pipes;
use crate::adapter::outbound::native_vault_confirmation::windows::exchange;
use crate::application::port::outbound::vault_native_confirmation_port::{
    VaultNativeConfirmationOutcome as Outcome, VaultNativeConfirmationPurpose as Purpose,
    VaultNativeConfirmationRequest,
    helper_protocol::{self as protocol, Decision},
};
use std::fs;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const FIXTURE: &str =
    "composition::desktop_restore::windows_tests::native_confirmation_process_fixture";
const ROLE: &str = "AKRA_TEST_VAULT_NATIVE_ROLE";
const READY: &str = "AKRA_TEST_VAULT_NATIVE_READY";
const RELEASE: &str = "AKRA_TEST_VAULT_NATIVE_RELEASE";

fn command(role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .env_clear()
        .args(["--exact", FIXTURE, "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .stdin(Stdio::piped())
        .stdout(Stdio::null()) // libtest output never shares the private fixture receipt pipe.
        .stderr(Stdio::piped());
    command
}

fn request(purpose: Purpose) -> protocol::Request {
    protocol::Request::from_target(
        &VaultNativeConfirmationRequest {
            issue_url: "https://github.com/acme/project/issues/1".into(),
            workspace_dir: r"\\?\C:\work\한글 project".into(),
            purpose,
        },
        format!("{:032x}", rand::random::<u128>()),
        10_000,
    )
}

struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-vault-native-process-{}-{}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn ready(&self) -> PathBuf {
        self.0.join("ready.json")
    }
    fn release(&self) -> PathBuf {
        self.0.join("release")
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture did not reach the barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn marker() {
    if let Some(path) = std::env::var_os(READY) {
        fs::write(path, b"ready").unwrap();
    }
}

fn park() -> ! {
    loop {
        std::thread::park_timeout(Duration::from_secs(1));
    }
}

fn read_wire() -> protocol::Request {
    let mut input = std::io::stdin();
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        input.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() <= protocol::MAX_REQUEST_BYTES);
        if byte[0] == b'\n' {
            break;
        }
    }
    protocol::decode_request(&bytes).unwrap()
}

fn emit(receipt: &protocol::Receipt) {
    let mut output = std::io::stderr();
    output
        .write_all(&protocol::encode(receipt, protocol::MAX_RESPONSE_BYTES).unwrap())
        .unwrap();
    output.flush().unwrap();
}

#[test]
fn native_confirmation_process_fixture() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    assert!(
        unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() }.is_null(),
        "the native helper must not inherit or create a console"
    );
    match role.as_str() {
        "confirmed" | "cancelled" | "waiting" | "late" | "subtree" => {
            let code = run_with_pipes(std::io::stdin(), std::io::stderr(), move |_| {
                if role != "subtree" {
                    marker();
                }
                match role.as_str() {
                    "confirmed" => Outcome::Confirmed,
                    "cancelled" => Outcome::Cancelled,
                    "late" => {
                        wait_file(Path::new(&std::env::var_os(RELEASE).unwrap()));
                        Outcome::Confirmed
                    }
                    "subtree" => {
                        // This child inherits the helper's Job; it does not create its own containment.
                        let descendant = Owner(
                            command("park")
                                .stdin(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn()
                                .unwrap(),
                        );
                        let ready = std::env::var_os(READY).unwrap();
                        let payload = serde_json::json!({
                            "helper": std::process::id(), "descendant": descendant.0.id(),
                        });
                        // Publish the complete subtree marker once. There is no earlier generic marker.
                        let ready = PathBuf::from(ready);
                        let pending = ready.with_extension("pending");
                        fs::write(&pending, serde_json::to_vec(&payload).unwrap()).unwrap();
                        fs::rename(pending, ready).unwrap();
                        let _descendant = descendant;
                        park()
                    }
                    _ => park(),
                }
            });
            std::process::exit(i32::from(code));
        }
        "owner" => {
            let mut child_command = command("subtree");
            child_command.env(READY, std::env::var_os(READY).unwrap());
            let mut child = crate::subprocess::spawn_without_console(&mut child_command).unwrap();
            let input = child.take_stdin().unwrap();
            let output = child.take_stderr().unwrap();
            let outcome = exchange(
                child,
                input,
                output,
                request(Purpose::ViewOriginal),
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(30),
            );
            assert_ne!(outcome, Outcome::Confirmed);
            std::process::exit(0);
        }
        "park" => park(),
        "neverRead" => {
            marker();
            park();
        }
        _ => {
            let wire = read_wire();
            let mut receipt = protocol::Receipt::new(&wire, Decision::Confirmed);
            match role.as_str() {
                "wrongId" => receipt.id.push('0'),
                "wrongPurpose" => receipt.purpose = Purpose::RewriteBinding,
                "oversize" => {
                    std::io::stderr()
                        .write_all(&vec![b'x'; protocol::MAX_RESPONSE_BYTES + 1])
                        .unwrap();
                    std::process::exit(0);
                }
                "truncated" => {
                    let bytes = protocol::encode(&receipt, protocol::MAX_RESPONSE_BYTES).unwrap();
                    std::io::stderr()
                        .write_all(&bytes[..bytes.len() - 1])
                        .unwrap();
                    std::process::exit(0);
                }
                "unknownField" => {
                    let mut value = serde_json::to_value(&receipt).unwrap();
                    value["approved"] = true.into();
                    std::io::stderr()
                        .write_all(&protocol::encode(&value, protocol::MAX_RESPONSE_BYTES).unwrap())
                        .unwrap();
                    std::process::exit(0);
                }
                "confirmedThenFail" | "confirmedThenWait" | "extra" => {}
                _ => panic!("unknown fixture role"),
            }
            emit(&receipt);
            match role.as_str() {
                "confirmedThenFail" => std::process::exit(7),
                "confirmedThenWait" => {
                    marker();
                    park();
                }
                "extra" => std::io::stderr().write_all(b"\n").unwrap(),
                _ => {}
            }
            std::process::exit(0);
        }
    }
}

struct ProcessProbe(OwnedHandle);
impl ProcessProbe {
    fn open(pid: u32) -> Self {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        assert!(!handle.is_null(), "could not pin the owned fixture process");
        Self(unsafe { OwnedHandle::from_raw_handle(handle) })
    }

    fn assert_exited(&self) {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        assert_eq!(
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 5_000) },
            WAIT_OBJECT_0,
            "owned native helper process did not exit"
        );
    }
}
impl Drop for ProcessProbe {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
        use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
        // Failure cleanup targets only the process handle pinned by this fixture, never a reused PID.
        unsafe {
            if WaitForSingleObject(self.0.as_raw_handle(), 0) == WAIT_TIMEOUT {
                TerminateProcess(self.0.as_raw_handle(), 1);
                WaitForSingleObject(self.0.as_raw_handle(), 5_000);
            }
        }
    }
}

fn run_exchange(role: &str, wire: protocol::Request, lifetime: Duration) -> Outcome {
    let mut child = crate::subprocess::spawn_without_console(&mut command(role)).unwrap();
    let probe = ProcessProbe::open(child.id());
    let input = child.take_stdin().unwrap();
    let output = child.take_stderr().unwrap();
    let outcome = exchange(
        child,
        input,
        output,
        wire,
        &AtomicBool::new(false),
        Instant::now() + lifetime,
    );
    probe.assert_exited();
    outcome
}

#[test]
fn windows_native_pipe_accepts_only_a_correlated_receipt_after_successful_exit() {
    for purpose in [Purpose::ViewOriginal, Purpose::RewriteBinding] {
        assert_eq!(
            run_exchange("confirmed", request(purpose), Duration::from_secs(5)),
            Outcome::Confirmed
        );
        assert_eq!(
            run_exchange("cancelled", request(purpose), Duration::from_secs(5)),
            Outcome::Cancelled
        );
    }
    for role in [
        "wrongId",
        "wrongPurpose",
        "oversize",
        "truncated",
        "unknownField",
        "extra",
        "confirmedThenFail",
    ] {
        assert_eq!(
            run_exchange(role, request(Purpose::ViewOriginal), Duration::from_secs(5)),
            Outcome::Unavailable,
            "{role}"
        );
    }
}

#[test]
fn windows_native_pipe_never_accepts_confirmation_from_a_process_that_stays_alive() {
    let folder = Folder::new();
    let mut cmd = command("confirmedThenWait");
    cmd.env(READY, folder.ready());
    let mut child = crate::subprocess::spawn_without_console(&mut cmd).unwrap();
    let probe = ProcessProbe::open(child.id());
    let input = child.take_stdin().unwrap();
    let output = child.take_stderr().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let worker = std::thread::spawn(move || {
        exchange(
            child,
            input,
            output,
            request(Purpose::ViewOriginal),
            &flag,
            Instant::now() + Duration::from_secs(10),
        )
    });
    wait_file(&folder.ready()); // The complete matching receipt has already been flushed.
    assert!(
        !worker.is_finished(),
        "a receipt alone must not approve a still-running helper"
    );
    cancelled.store(true, Ordering::SeqCst);
    assert_eq!(worker.join().unwrap(), Outcome::Cancelled);
    probe.assert_exited();
}

#[test]
fn windows_native_cancel_fences_a_late_approval_and_reaps_the_helper() {
    let folder = Folder::new();
    let mut cmd = command("late");
    cmd.env(READY, folder.ready())
        .env(RELEASE, folder.release());
    let mut child = crate::subprocess::spawn_without_console(&mut cmd).unwrap();
    let probe = ProcessProbe::open(child.id());
    let input = child.take_stdin().unwrap();
    let output = child.take_stderr().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let worker = std::thread::spawn(move || {
        exchange(
            child,
            input,
            output,
            request(Purpose::ViewOriginal),
            &flag,
            Instant::now() + Duration::from_secs(10),
        )
    });
    wait_file(&folder.ready());
    cancelled.store(true, Ordering::SeqCst);
    fs::write(folder.release(), b"release").unwrap();
    assert_eq!(worker.join().unwrap(), Outcome::Cancelled);
    probe.assert_exited();
}

#[test]
fn windows_native_stuck_request_writer_is_bounded_by_the_parent_deadline() {
    let folder = Folder::new();
    let mut cmd = command("neverRead");
    cmd.env(READY, folder.ready());
    let mut child = crate::subprocess::spawn_without_console(&mut cmd).unwrap();
    let probe = ProcessProbe::open(child.id());
    let input = child.take_stdin().unwrap();
    let output = child.take_stderr().unwrap();
    wait_file(&folder.ready());
    let mut wire = request(Purpose::ViewOriginal);
    wire.workspace_dir = "x".repeat(4096);
    wire.issue_url = format!("https://github.com/{}", "x".repeat(2029));
    assert_eq!(
        exchange(
            child,
            input,
            output,
            wire,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_millis(400),
        ),
        Outcome::Cancelled
    );
    probe.assert_exited();
}

fn wait_child(child: &mut crate::subprocess::ManagedChild) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "native helper did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn windows_helper_parent_eof_and_own_deadline_close_a_pending_fake_dialog() {
    for eof in [true, false] {
        let folder = Folder::new();
        let mut cmd = command("waiting");
        cmd.env(READY, folder.ready());
        let mut child = crate::subprocess::spawn_without_console(&mut cmd).unwrap();
        let probe = ProcessProbe::open(child.id());
        let mut input = child.take_stdin().unwrap();
        let mut output = child.take_stderr().unwrap();
        let mut wire = request(Purpose::ViewOriginal);
        wire.lifetime_millis = if eof { 10_000 } else { 2_000 };
        input
            .write_all(&protocol::encode(&wire, protocol::MAX_REQUEST_BYTES).unwrap())
            .unwrap();
        input.flush().unwrap();
        wait_file(&folder.ready());
        let _held = if eof {
            drop(input);
            None
        } else {
            Some(input)
        };
        assert!(wait_child(&mut child).success());
        let mut receipt = Vec::new();
        output.read_to_end(&mut receipt).unwrap();
        assert_eq!(
            protocol::decode_receipt(&receipt, &wire),
            Some(Outcome::Cancelled)
        );
        probe.assert_exited();
    }
}

#[test]
fn windows_helper_rejects_override_and_pipelined_input_before_any_fake_dialog() {
    for extra in [false, true] {
        let folder = Folder::new();
        let mut cmd = command("confirmed");
        cmd.env(READY, folder.ready());
        let mut child = crate::subprocess::spawn_without_console(&mut cmd).unwrap();
        let mut input = child.take_stdin().unwrap();
        let wire = request(Purpose::ViewOriginal);
        let mut value = serde_json::to_value(&wire).unwrap();
        if !extra {
            value["approved"] = true.into();
        }
        let mut bytes = protocol::encode(&value, protocol::MAX_REQUEST_BYTES).unwrap();
        if extra {
            bytes.extend_from_slice(b"{}\n");
        }
        input.write_all(&bytes).unwrap();
        input.flush().unwrap();
        assert!(!wait_child(&mut child).success());
        assert!(!folder.ready().exists());
    }
}

struct Owner(Child);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn windows_native_parent_forced_termination_closes_the_job_and_reaps_descendants() {
    let folder = Folder::new();
    let mut cmd = command("owner");
    cmd.env(READY, folder.ready()).stderr(Stdio::null());
    // Deliberately no outer ManagedChild: killing only this owner must close its native helper Job.
    let mut owner = Owner(cmd.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let pids = loop {
        if let Ok(bytes) = fs::read(folder.ready())
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && value["helper"].is_u64()
            && value["descendant"].is_u64()
        {
            break value;
        }
        assert!(
            Instant::now() < deadline,
            "native helper subtree did not reach the barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let helper = ProcessProbe::open(pids["helper"].as_u64().unwrap() as u32);
    let descendant = ProcessProbe::open(pids["descendant"].as_u64().unwrap() as u32);
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    helper.assert_exited();
    descendant.assert_exited();
}

#[test]
fn windows_native_managed_child_drop_closes_a_pending_helper() {
    let mut child = crate::subprocess::spawn_without_console(&mut command("park")).unwrap();
    let probe = ProcessProbe::open(child.id());
    let input = child.take_stdin().unwrap();
    drop(input);
    drop(child);
    probe.assert_exited();
}

mod workspace;
