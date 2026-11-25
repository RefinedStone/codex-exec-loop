use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use super::*;
use crate::application::port::outbound::issue_code_validator_runner_port::IssueCodeValidatorRunRequest;

pub(super) struct Fixture {
    root: PathBuf,
    image: PathBuf,
    digest: String,
}
impl Fixture {
    pub(super) fn create() -> Self {
        Self::with_source(include_str!("native_fixture.rs"))
    }
    fn with_source(content: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "akra-validator-test-{:032x}",
            rand::random::<u128>()
        ));
        fs::create_dir(&root).unwrap();
        let mut fixture = Self {
            image: root.join("fixture.exe"),
            root,
            digest: String::new(),
        };
        let root = &fixture.root;
        let source = root.join("fixture.rs");
        let image = root.join("fixture.exe");
        fs::write(&source, content).unwrap();
        let mut compiler = Command::new("rustc");
        compiler.args([
            "--edition=2024",
            "-C",
            "opt-level=1",
            "-C",
            "debuginfo=0",
            "-C",
            "target-feature=+crt-static",
            "-C",
            "panic=abort",
        ]);
        let output = compiler
            .arg(&source)
            .arg("-o")
            .arg(&image)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "native fixture compiler: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let digest = format!("{:x}", Sha256::digest(fs::read(&image).unwrap()));
        fixture.digest = digest;
        fixture
    }
    pub(super) fn request(&self, input: &str) -> IssueCodeValidatorRunRequest {
        IssueCodeValidatorRunRequest {
            command: self.image.to_str().unwrap().to_owned(),
            sha256: self.digest.clone(),
            timeout: Duration::from_secs(3),
            input_json: input.to_owned(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn native_windows_validator_passes_only_after_real_isolation_is_established() {
    let fixture = Fixture::create();
    assert_eq!(run(&fixture.request("pass")), Outcome::Passed);
}

#[test]
fn native_windows_validator_blocks_outside_files_network_and_child_creation() {
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let fixture = Fixture::create();
    let owner = crate::private_fs::windows_current_user_sid_string().unwrap();
    let private = fixture.root.join("synthetic-vault.txt");
    let app_readable = fixture.root.join("synthetic-aap-readable.txt");
    storage::write_new(
        &private,
        b"synthetic private data",
        &security::Descriptor::from_sddl(&format!("D:P(A;;FA;;;{owner})")).unwrap(),
    )
    .unwrap();
    storage::write_new(
        &app_readable,
        b"synthetic app-readable data",
        &security::Descriptor::from_sddl(&format!("D:P(A;;FA;;;{owner})(A;;GR;;;S-1-15-2-1)"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(fs::read(&private).unwrap(), b"synthetic private data");
    assert_eq!(
        fs::read(&app_readable).unwrap(),
        b"synthetic app-readable data"
    );
    let file_input = format!("files\n{}\n{}", private.display(), app_readable.display());
    assert_eq!(run(&fixture.request(&file_input)), Outcome::Passed);
    assert_eq!(fs::read(&private).unwrap(), b"synthetic private data");
    assert_eq!(
        fs::read(&app_readable).unwrap(),
        b"synthetic app-readable data"
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    // Positive control: the exact fixture can reach this listener outside the LPAC.
    let network_input = format!("network\n{}", listener.local_addr().unwrap());
    let mut control = Command::new(&fixture.image)
        .creation_flags(0x0800_0000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    control
        .stdin
        .take()
        .unwrap()
        .write_all(network_input.as_bytes())
        .unwrap();
    assert_eq!(control.wait_with_output().unwrap().status.code(), Some(0));
    let (connection, _) = listener
        .accept()
        .expect("positive network control connected");
    drop(connection);
    let result = run(&fixture.request(&network_input));
    eprintln!("native contained network result: {result:?}");
    assert!(
        matches!(result, Outcome::Rejected { ref message } if message == "network denied at WSAStartup: 10107" || message == "network denied at connect: Some(10013)")
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    // Host reachability remains true after the denied contained attempt.
    drop(TcpStream::connect(listener.local_addr().unwrap()).unwrap());
    drop(listener.accept().unwrap());

    for mode in ["child", "detached-child", "environment"] {
        assert_eq!(run(&fixture.request(mode)), Outcome::Passed, "{mode}");
    }
}

#[test]
fn native_windows_validator_bounds_runtime_output_and_reports_rejection_or_crash() {
    let fixture = Fixture::create();
    assert_eq!(
        run(&fixture.request("reject")),
        Outcome::Rejected {
            message: "synthetic validation failed".to_owned()
        }
    );
    assert_eq!(
        run(&fixture.request("crash")),
        Outcome::NotRun(NotRun::Crashed)
    );
    for mode in ["timeout", "flood"] {
        let mut request = fixture.request(mode);
        request.timeout = Duration::from_millis(200);
        let started = Instant::now();
        assert_eq!(run(&request), Outcome::NotRun(NotRun::TimedOut), "{mode}");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "bounded timeout and cleanup: {mode}"
        );
    }
    let Outcome::Rejected { message } = run(&fixture.request("long-reject")) else {
        panic!("output-capped rejection expected");
    };
    assert_eq!(message.len(), 4096);
    assert!(message.bytes().all(|value| value == b'x'));
    let Outcome::Rejected { message } = run(&fixture.request("invalid-utf8-reject")) else {
        panic!("bounded lossy output expected");
    };
    assert!(message.len() <= 4096);
    assert!(message.chars().all(|value| value == '\u{fffd}'));
}

#[test]
fn native_windows_validator_pins_approved_bytes_and_rejects_unsupported_images() {
    let fixture = Fixture::create();
    let request = fixture.request("pass");
    let approved = storage::approved_content(&request).unwrap();
    fs::write(&fixture.image, b"@echo off\r\nexit 0\r\n").unwrap();
    assert_eq!(run(&request), Outcome::NotRun(NotRun::HashMismatch));
    let identity = security::Identity::create().unwrap();
    // Simulate a source replacement after verification. Only the already approved bytes execute.
    assert_eq!(
        execute(&identity, &approved, &request).unwrap(),
        Outcome::Passed
    );
    let mut script = fixture.request("pass");
    script.sha256 = format!("{:x}", Sha256::digest(fs::read(&fixture.image).unwrap()));
    assert_eq!(run(&script), Outcome::NotRun(NotRun::IsolationUnavailable));

    let mut console = approved;
    let pe = u32::from_le_bytes(console[0x3c..0x40].try_into().unwrap()) as usize;
    console[pe + 24 + 68..pe + 24 + 70].copy_from_slice(&3u16.to_le_bytes());
    fs::write(&fixture.image, &console).unwrap();
    let mut request = fixture.request("pass");
    request.sha256 = format!("{:x}", Sha256::digest(&console));
    assert_eq!(run(&request), Outcome::NotRun(NotRun::IsolationUnavailable));
    request.command = fixture
        .root
        .join("missing.exe")
        .to_string_lossy()
        .into_owned();
    assert_eq!(run(&request), Outcome::NotRun(NotRun::Missing));
}

#[test]
fn native_windows_validator_observes_workspace_and_profile_removal_on_all_outcomes() {
    let fixture = Fixture::create();
    let local = crate::private_fs::windows_local_app_data_path().unwrap();
    for mode in ["pass", "reject", "crash", "timeout"] {
        let mut request = fixture.request(mode);
        request.timeout = Duration::from_millis(200);
        let approved = storage::approved_content(&request).unwrap();
        let identity = security::Identity::create().unwrap();
        let workspace_path = local.join("Temp").join(&identity.name);
        let profile_path = local.join("Packages").join(&identity.name);
        let outcome = execute(&identity, &approved, &request).unwrap();
        match mode {
            "pass" => assert_eq!(outcome, Outcome::Passed),
            "reject" => assert!(matches!(outcome, Outcome::Rejected { .. })),
            "crash" => assert_eq!(outcome, Outcome::NotRun(NotRun::Crashed)),
            "timeout" => assert_eq!(outcome, Outcome::NotRun(NotRun::TimedOut)),
            _ => unreachable!(),
        }
        assert!(!workspace_path.exists(), "owned stage removed after {mode}");
        assert!(!profile_path.exists(), "owned profile removed after {mode}");
    }
}

#[test]
fn native_windows_validator_removes_child_profile_payload_without_following_external_junction() {
    let fixture = Fixture::create();
    let external = fixture.root.join("external-disposable-target");
    fs::create_dir(&external).unwrap();
    let sentinel = external.join("keep.txt");
    fs::write(&sentinel, b"synthetic outside canary").unwrap();
    let before = file_identity(&sentinel);
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let control_link = fixture.root.join("host-junction-control");
    let mut control = Command::new(&fixture.image)
        .creation_flags(0x0800_0000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = format!(
        "junction-control\n{}\n{}",
        control_link.display(),
        external.display()
    );
    control
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let result = control.wait_with_output().unwrap();
    assert_eq!(
        result.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "junction control created"
    );
    // Remove only the newly created junction, never its external target.
    fs::remove_dir(&control_link).unwrap();
    assert_eq!(fs::read(&sentinel).unwrap(), b"synthetic outside canary");
    assert_eq!(file_identity(&sentinel), before);

    let identity = security::Identity::create().unwrap();
    let local = crate::private_fs::windows_local_app_data_path().unwrap();
    let profile = local.join("Packages").join(&identity.name);
    let request = fixture.request(&format!(
        "profile\n{}\n{}",
        external.display(),
        profile.display()
    ));
    let content = storage::approved_content(&request).unwrap();
    let result = execute(&identity, &content, &request).unwrap();
    eprintln!("native child profile cleanup result: {result:?}");
    let Outcome::Rejected { message } = result else {
        panic!("profile payload observation required");
    };
    assert!(
        message == "profile payload created; junction created"
            || message == "profile payload created; junction denied at open: Some(5)"
            || message
                == "profile payload created; junction denied at FSCTL_SET_REPARSE_POINT: Some(5)"
            || message
                == "profile payload created; junction denied at FSCTL_SET_REPARSE_POINT: Some(1314)",
        "{message}"
    );
    assert!(!profile.exists(), "the newly owned profile was removed");
    assert!(
        !local.join("Temp").join(&identity.name).exists(),
        "the stage was removed"
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"synthetic outside canary");
    assert_eq!(file_identity(&sentinel), before);
}

fn file_identity(path: &std::path::Path) -> [u32; 3] {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = fs::File::open(path).unwrap();
    // SAFETY: a live read handle and exactly sized writable information output.
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) },
        0
    );
    [
        information.dwVolumeSerialNumber,
        information.nFileIndexHigh,
        information.nFileIndexLow,
    ]
}

#[test]
fn native_windows_validator_never_deletes_an_existing_profile_on_name_collision() {
    let identity = security::Identity::create().unwrap();
    let local = crate::private_fs::windows_local_app_data_path().unwrap();
    let path = local.join("Packages").join(&identity.name);
    let mut owner = profile::Profile::create(&identity).unwrap();
    assert!(path.exists());
    assert!(profile::Profile::create(&identity).is_err());
    assert!(
        path.exists(),
        "a failed creator never owns the existing profile"
    );
    owner.cleanup().unwrap();
    assert!(!path.exists());
}
