use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorCancellation, IssueCodeValidatorNotRun, IssueCodeValidatorRunOutcome,
    IssueCodeValidatorRunRequest, IssueCodeValidatorRunnerPort,
};

/*
 * 승인한 코드 검증기를 격리해 실행한다 (#2431).
 *
 * macOS는 sandbox-exec, Linux는 엄격한 native bubblewrap 프로필만 사용한다.
 * Windows는 검증된 headless PE32+ LPAC 프로필만 사용한다.
 * 지원되지 않는 플랫폼/프로필/이미지 형식은 실행하지 않는다.
 * 격리 없는 실행으로 바꾸는 길은 없다.
 * 사용자 취소는 모든 runner 가 제한 시간과 같은 자리에서 확인하고, 같은 정리 경로로 프로세스 트리를 끝낸다.
 *
 * macOS 에서의 실행:
 * - 승인한 해시와 같은 내용을 임시 작업 공간에 복사하고 그 복사본을 실행한다. 해시 확인 뒤에 원본을
 *   바꿔도 실행 내용은 같다.
 * - 환경 변수는 고정 `PATH` 하나이다. Vault 값, 계정 자격 증명, 상속 토큰이 없다.
 * - 프로필은 네트워크를 막고, 시스템 경로와 작업 공간만 읽게 하고, 작업 공간에만 쓰게 한다.
 */
#[derive(Debug, Default)]
pub struct SandboxedIssueCodeValidatorRunnerAdapter;

impl SandboxedIssueCodeValidatorRunnerAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl IssueCodeValidatorRunnerPort for SandboxedIssueCodeValidatorRunnerAdapter {
    fn run(
        &self,
        request: IssueCodeValidatorRunRequest,
        cancellation: &IssueCodeValidatorCancellation,
    ) -> IssueCodeValidatorRunOutcome {
        if cancellation.is_cancelled() {
            return IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Cancelled);
        }
        #[cfg(target_os = "macos")]
        {
            macos::run(&request, cancellation)
        }
        #[cfg(target_os = "linux")]
        {
            linux::run(&request, cancellation)
        }
        #[cfg(windows)]
        {
            windows::run(&request, cancellation)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let _ = (request, cancellation);
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::IsolationUnavailable)
        }
    }
}

/// Bound the final UTF-8 message, not just the captured raw bytes.
#[cfg(any(test, target_os = "macos", target_os = "linux", windows))]
fn bounded_rejection_message(bytes: &[u8], limit: usize) -> String {
    let mut value = String::from_utf8_lossy(&bytes[..bytes.len().min(limit)])
        .trim()
        .to_owned();
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

#[cfg(test)]
mod common_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn validator_cancelled_before_admission_never_reads_an_executable() {
        let cancellation = IssueCodeValidatorCancellation::new();
        assert!(cancellation.cancel());
        let result = SandboxedIssueCodeValidatorRunnerAdapter::new().run(
            IssueCodeValidatorRunRequest {
                command: String::new(),
                sha256: String::new(),
                timeout: Duration::from_secs(30),
                input_json: "{}".into(),
            },
            &cancellation,
        );
        assert_eq!(
            result,
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Cancelled)
        );
    }

    #[test]
    fn validator_rejection_message_bounds_lossy_utf8_on_character_boundary() {
        let limit = 4096;
        let malformed = bounded_rejection_message(&vec![0xff; limit], limit);
        assert_eq!(malformed.len(), limit - 1);
        assert!(malformed.chars().all(|ch| ch == '\u{fffd}'));
        let mut split = vec![b'x'; limit - 1];
        split.extend_from_slice("한".as_bytes());
        assert_eq!(
            bounded_rejection_message(&split, limit),
            "x".repeat(limit - 1)
        );
        assert_eq!(bounded_rejection_message(b"  reject  ", limit), "reject");
        assert_eq!(bounded_rejection_message(b"x", 0), "");
    }
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
mod macos {
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use sha2::{Digest, Sha256};

    use crate::application::port::outbound::issue_code_validator_runner_port::{
        IssueCodeValidatorCancellation, IssueCodeValidatorNotRun, IssueCodeValidatorRunOutcome,
        IssueCodeValidatorRunRequest,
    };

    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
    /// 거부 글로 읽는 stdout 의 최대 바이트 수이다.
    const MESSAGE_LIMIT_BYTES: usize = 4096;

    // shortcut: 시스템 경로의 해석기만 읽을 수 있다 (`/bin/sh`, Command Line Tools 의 python3). 다른 자리의
    // 해석기가 필요하면 승인 항목에 읽기 경로를 더한다.
    const PROFILE: &str = r#"(version 1)
(deny default)
(allow process-exec)
(allow process-fork)
(allow sysctl-read)
(allow file-read-metadata)
(allow file-read*
  (literal "/")
  (subpath "/usr")
  (subpath "/bin")
  (subpath "/sbin")
  (subpath "/System")
  (subpath "/Library/Developer")
  (subpath "/private/etc")
  (subpath "/private/var/db/dyld")
  (subpath "/dev")
  (subpath (param "WORKSPACE")))
(allow file-write*
  (literal "/dev/null")
  (subpath (param "WORKSPACE")))
"#;

    pub(super) fn run(
        request: &IssueCodeValidatorRunRequest,
        cancellation: &IssueCodeValidatorCancellation,
    ) -> IssueCodeValidatorRunOutcome {
        let not_run = IssueCodeValidatorRunOutcome::NotRun;
        if cancellation.is_cancelled() {
            return not_run(IssueCodeValidatorNotRun::Cancelled);
        }
        let Ok(content) = fs::read(&request.command) else {
            return not_run(IssueCodeValidatorNotRun::Missing);
        };
        if format!("{:x}", Sha256::digest(&content)) != request.sha256 {
            return not_run(IssueCodeValidatorNotRun::HashMismatch);
        }
        if !Path::new(SANDBOX_EXEC).is_file() {
            return not_run(IssueCodeValidatorNotRun::IsolationUnavailable);
        }
        let Ok(workspace) = create_workspace() else {
            return not_run(IssueCodeValidatorNotRun::Crashed);
        };
        let outcome = run_in_workspace(&workspace, &content, request, cancellation)
            .unwrap_or(not_run(IssueCodeValidatorNotRun::Crashed));
        let _ = fs::remove_dir_all(&workspace);
        outcome
    }

    /// 현재 사용자만 쓰는 새 디렉터리이다. 이미 있는 경로는 쓰지 않는다.
    fn create_workspace() -> io::Result<PathBuf> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "akra-issue-validator-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new().mode(0o700).create(&path)?;
        // 프로필은 링크를 푼 경로로 비교한다 (`/var` 는 `/private/var` 의 링크이다).
        path.canonicalize()
    }

    fn run_in_workspace(
        workspace: &Path,
        content: &[u8],
        request: &IssueCodeValidatorRunRequest,
        cancellation: &IssueCodeValidatorCancellation,
    ) -> io::Result<IssueCodeValidatorRunOutcome> {
        let validator = workspace.join("validator");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&validator)?
            .write_all(content)?;
        let input = workspace.join("input.json");
        fs::write(&input, &request.input_json)?;

        if cancellation.is_cancelled() {
            return Ok(IssueCodeValidatorRunOutcome::NotRun(
                IssueCodeValidatorNotRun::Cancelled,
            ));
        }
        let mut child = Command::new(SANDBOX_EXEC)
            .arg("-D")
            .arg(format!("WORKSPACE={}", workspace.display()))
            .arg("-p")
            .arg(PROFILE)
            .arg(&validator)
            .current_dir(workspace)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(File::open(&input)?)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            // 제한 시간에 검증기와 그 자식을 함께 끝낸다.
            .process_group(0)
            .spawn()?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("validator stdout pipe is unavailable"))?;
        let mut message = Vec::new();
        let status =
            (|| -> io::Result<Result<std::process::ExitStatus, IssueCodeValidatorNotRun>> {
                // The parent keeps this pipe handle; the validator cannot substitute a file or FIFO.
                let fd = stdout.as_raw_fd();
                // SAFETY: fd belongs to the live ChildStdout. fcntl changes only its status flags.
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags < 0
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                {
                    return Err(io::Error::last_os_error());
                }
                let deadline = Instant::now() + request.timeout;
                loop {
                    drain_stdout(&mut stdout, &mut message)?;
                    if cancellation.is_cancelled() {
                        return Ok(Err(IssueCodeValidatorNotRun::Cancelled));
                    }
                    if let Some(status) = child.try_wait()? {
                        return Ok(Ok(status));
                    }
                    if Instant::now() >= deadline {
                        return Ok(Err(IssueCodeValidatorNotRun::TimedOut));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            })();
        // Always stop descendants, including when collection fails or a descendant keeps stdout open.
        // SAFETY: the negative PID identifies the process group created above.
        unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
        let _ = child.wait();
        let status = match status? {
            Ok(status) => status,
            Err(reason) => return Ok(IssueCodeValidatorRunOutcome::NotRun(reason)),
        };
        drain_stdout(&mut stdout, &mut message)?;

        Ok(match status.code() {
            Some(0) => IssueCodeValidatorRunOutcome::Passed,
            Some(1) => IssueCodeValidatorRunOutcome::Rejected {
                message: super::bounded_rejection_message(&message, MESSAGE_LIMIT_BYTES),
            },
            _ => IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Crashed),
        })
    }

    /// Drain a bounded slice per cycle so a continuously writing child cannot starve the deadline.
    fn drain_stdout(stdout: &mut impl Read, message: &mut Vec<u8>) -> io::Result<()> {
        let mut buffer = [0_u8; 8192];
        for _ in 0..8 {
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    let keep = length.min(MESSAGE_LIMIT_BYTES.saturating_sub(message.len()));
                    message.extend_from_slice(&buffer[..keep]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::os::unix::fs::PermissionsExt;

        use super::*;

        #[test]
        fn sandboxed_validator_gets_the_draft_without_environment_or_outside_files() {
            let directory = create_workspace().expect("fixture directory");
            let outside = directory.join("outside.txt");
            fs::write(&outside, "local only").expect("outside file");
            // 종료 코드: 3 = 상속한 환경 변수가 있다, 4 = 작업 공간 밖의 파일을 읽었다, 5 = stdin 이 초안이 아니다.
            let script = format!(
                "#!/bin/sh\n[ -z \"$HOME\" ] || exit 3\nif cat '{}' 2>/dev/null; then exit 4; fi\ngrep -q '\"title\"' || exit 5\necho 'title is too short'\nexit 1\n",
                outside.display()
            );
            let command = directory.join("validator.sh");
            fs::write(&command, &script).expect("validator script");
            fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).expect("mode");
            let request = |sha256: String, timeout: Duration| IssueCodeValidatorRunRequest {
                command: command.display().to_string(),
                sha256,
                timeout,
                input_json: r#"{"title":"T","body":"B"}"#.to_string(),
            };
            let sha256 = format!("{:x}", Sha256::digest(script.as_bytes()));

            assert_eq!(
                run(
                    &request(sha256, Duration::from_secs(20)),
                    &IssueCodeValidatorCancellation::new()
                ),
                IssueCodeValidatorRunOutcome::Rejected {
                    message: "title is too short".to_string()
                }
            );
            assert_eq!(
                run(
                    &request("0".repeat(64), Duration::from_secs(20)),
                    &IssueCodeValidatorCancellation::new()
                ),
                IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::HashMismatch)
            );
            fs::remove_dir_all(directory).expect("cleanup");
        }

        #[test]
        fn stdout_path_substitution_cannot_escape_the_pipe_capture() {
            let directory = create_workspace().unwrap();
            let outside = directory.join("outside.txt");
            fs::write(&outside, "synthetic outside secret").unwrap();
            for replacement in [
                format!(
                    "ln -s '{}' stdout.txt && [ -L stdout.txt ]",
                    outside.display()
                ),
                "mkfifo stdout.txt && [ -p stdout.txt ]".to_string(),
            ] {
                let script = format!(
                    "#!/bin/sh\nrm -f stdout.txt\n{replacement} || exit 2\nprintf 'safe result'\nexit 1\n"
                );
                let request = IssueCodeValidatorRunRequest {
                    command: String::new(),
                    sha256: String::new(),
                    timeout: Duration::from_secs(10),
                    input_json: "{}".to_string(),
                };
                let workspace = create_workspace().unwrap();
                assert_eq!(
                    run_in_workspace(
                        &workspace,
                        script.as_bytes(),
                        &request,
                        &IssueCodeValidatorCancellation::new()
                    )
                    .unwrap(),
                    IssueCodeValidatorRunOutcome::Rejected {
                        message: "safe result".to_string()
                    }
                );
                fs::remove_dir_all(workspace).unwrap();
            }
            fs::remove_dir_all(directory).unwrap();
        }

        #[test]
        fn large_output_is_drained_without_a_stdout_file_or_unbounded_retention() {
            let workspace = create_workspace().unwrap();
            let script = b"#!/bin/sh\nhead -c 2097152 /dev/zero | tr '\\000' x\nexit 1\n";
            let request = IssueCodeValidatorRunRequest {
                command: String::new(),
                sha256: String::new(),
                timeout: Duration::from_secs(20),
                input_json: "{}".to_string(),
            };
            assert_eq!(
                run_in_workspace(
                    &workspace,
                    script,
                    &request,
                    &IssueCodeValidatorCancellation::new()
                )
                .unwrap(),
                IssueCodeValidatorRunOutcome::Rejected {
                    message: "x".repeat(MESSAGE_LIMIT_BYTES)
                }
            );
            assert!(!workspace.join("stdout.txt").exists());
            fs::remove_dir_all(workspace).unwrap();
        }

        #[test]
        fn endless_output_still_obeys_the_deadline() {
            let workspace = create_workspace().unwrap();
            let request = IssueCodeValidatorRunRequest {
                command: String::new(),
                sha256: String::new(),
                timeout: Duration::from_millis(100),
                input_json: "{}".to_string(),
            };
            let started = Instant::now();
            assert_eq!(
                run_in_workspace(
                    &workspace,
                    b"#!/bin/sh\nexec yes x\n",
                    &request,
                    &IssueCodeValidatorCancellation::new()
                )
                .unwrap(),
                IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::TimedOut)
            );
            assert!(started.elapsed() < Duration::from_secs(5));
            fs::remove_dir_all(workspace).unwrap();
        }

        #[test]
        fn user_cancel_stops_the_running_validator_and_its_children() {
            let workspace = create_workspace().unwrap();
            let marker = workspace.join("still-running");
            // 자식이 살아 있으면 1 초 뒤에 표식을 만든다. 취소가 프로세스 그룹을 끝내면 표식이 없다.
            let script = format!(
                "#!/bin/sh\n(sleep 1; : > '{}') &\nexec sleep 30\n",
                marker.display()
            );
            let request = IssueCodeValidatorRunRequest {
                command: String::new(),
                sha256: String::new(),
                timeout: Duration::from_secs(30),
                input_json: "{}".to_string(),
            };
            let cancellation = IssueCodeValidatorCancellation::new();
            let started = Instant::now();
            let outcome = thread::scope(|scope| {
                scope.spawn(|| {
                    thread::sleep(Duration::from_millis(300));
                    assert!(cancellation.cancel());
                });
                run_in_workspace(&workspace, script.as_bytes(), &request, &cancellation).unwrap()
            });
            assert_eq!(
                outcome,
                IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Cancelled)
            );
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(1500));
            assert!(!marker.exists(), "a validator child survived the cancel");
            fs::remove_dir_all(workspace).unwrap();
        }
    }
}
