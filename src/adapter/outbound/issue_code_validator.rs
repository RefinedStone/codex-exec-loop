use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorRunOutcome, IssueCodeValidatorRunRequest, IssueCodeValidatorRunnerPort,
};

/*
 * 승인한 코드 검증기를 격리해 실행한다 (#2431).
 *
 * 격리는 macOS 의 `/usr/bin/sandbox-exec` 뿐이다. 다른 플랫폼에는 격리가 없어서 실행하지 않는다.
 * 격리 없는 실행으로 바꾸는 길은 없다.
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
    fn run(&self, request: IssueCodeValidatorRunRequest) -> IssueCodeValidatorRunOutcome {
        #[cfg(target_os = "macos")]
        {
            macos::run(&request)
        }
        #[cfg(not(target_os = "macos"))]
        {
            use crate::application::port::outbound::issue_code_validator_runner_port::IssueCodeValidatorNotRun;

            let _ = request;
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::IsolationUnavailable)
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use sha2::{Digest, Sha256};

    use crate::application::port::outbound::issue_code_validator_runner_port::{
        IssueCodeValidatorNotRun, IssueCodeValidatorRunOutcome, IssueCodeValidatorRunRequest,
    };

    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
    /// 거부 글로 읽는 stdout 의 최대 바이트 수이다.
    const MESSAGE_LIMIT_BYTES: u64 = 4096;

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

    pub(super) fn run(request: &IssueCodeValidatorRunRequest) -> IssueCodeValidatorRunOutcome {
        let not_run = IssueCodeValidatorRunOutcome::NotRun;
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
        let outcome = run_in_workspace(&workspace, &content, request)
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
        let stdout = workspace.join("stdout.txt");

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
            .stdout(File::create(&stdout)?)
            .stderr(Stdio::null())
            // 제한 시간에 검증기와 그 자식을 함께 끝낸다.
            .process_group(0)
            .spawn()?;

        let deadline = Instant::now() + request.timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                break None;
            }
            thread::sleep(Duration::from_millis(20));
        };
        // SAFETY: kill has no memory preconditions. The negative ID names the process group that
        // `process_group(0)` created for this child.
        unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
        let Some(status) = status else {
            let _ = child.wait();
            return Ok(IssueCodeValidatorRunOutcome::NotRun(
                IssueCodeValidatorNotRun::TimedOut,
            ));
        };

        Ok(match status.code() {
            Some(0) => IssueCodeValidatorRunOutcome::Passed,
            Some(1) => {
                let mut message = Vec::new();
                File::open(&stdout)?
                    .take(MESSAGE_LIMIT_BYTES)
                    .read_to_end(&mut message)?;
                IssueCodeValidatorRunOutcome::Rejected {
                    message: String::from_utf8_lossy(&message).trim().to_string(),
                }
            }
            _ => IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Crashed),
        })
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
                run(&request(sha256, Duration::from_secs(20))),
                IssueCodeValidatorRunOutcome::Rejected {
                    message: "title is too short".to_string()
                }
            );
            assert_eq!(
                run(&request("0".repeat(64), Duration::from_secs(20))),
                IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::HashMismatch)
            );
            fs::remove_dir_all(directory).expect("cleanup");
        }
    }
}
