use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::Utc;

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;

/*
 * GitParallelModeRuntimeAdapter는 parallel mode application service가 요청하는 낮은 수준의
 * runtime primitive를 실제 OS/Git/filesystem 호출로 연결하는 outbound adapter다.
 * pool, readiness, distributor, lease lifecycle은 모두 ParallelModeRuntimePort만 의존하므로,
 * 이 파일에 side effect를 모아 두면 service 계층에는 "언제 실행할지" 정책만 남는다.
 */
#[derive(Debug, Clone, Default)]
pub struct GitParallelModeRuntimeAdapter;

impl GitParallelModeRuntimeAdapter {
    pub fn new() -> Self {
        /*
         * adapter 자체는 설정을 들고 있지 않다. 매 호출마다 현재 process environment와 filesystem을
         * 관찰하므로, service가 Arc<dyn ParallelModeRuntimePort>로 공유해도 내부 상태 동기화가 필요 없다.
         */
        Self
    }
}

impl ParallelModeRuntimePort for GitParallelModeRuntimeAdapter {
    fn detect_git_repo_root(&self, workspace_dir: &str) -> Option<String> {
        /*
         * parallel pool은 사용자가 repo 하위 디렉터리나 slot worktree 안에서 실행해도 canonical repo 기준을
         * 찾아야 한다. 여기서는 git의 `rev-parse --show-toplevel` 결과만 돌려주고, linked worktree 보정 같은
         * 더 높은 정책은 application service 쪽 helper가 담당한다.
         */
        self.run_command(
            "git",
            &["-C", workspace_dir, "rev-parse", "--show-toplevel"],
            None,
        )
        .filter(|value| !value.is_empty())
    }

    fn command_succeeds(&self, program: &str, args: &[&str]) -> bool {
        /*
         * readiness probe는 stdout이 필요 없고 성공/실패만 중요하다.
         * stdout/stderr를 버려 TUI가 background capability check 중 터미널에 noise를 흘리지 않게 한다.
         */
        let mut command = Command::new(program);
        command.args(args);
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.status().is_ok_and(|status| status.success())
    }

    fn run_command(
        &self,
        program: &str,
        args: &[&str],
        current_dir: Option<&str>,
    ) -> Option<String> {
        /*
         * service 계층은 git/gh command 결과를 대부분 "사용 가능한 문자열인가"로 소비한다.
         * spawn 실패, non-zero exit, invalid utf8, empty stdout을 모두 None으로 축약해 caller가
         * capability degraded/blocker 같은 domain 상태로 바꾸기 쉽게 한다.
         */
        let mut command = Command::new(program);
        command.args(args);
        if let Some(current_dir) = current_dir {
            command.current_dir(current_dir);
        }
        command.stderr(Stdio::null());
        command.env("GIT_TERMINAL_PROMPT", "0");

        let output = command.output().ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let trimmed = stdout.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    fn run_command_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin_body: &str,
    ) -> Option<String> {
        /*
         * GitHub fallback script처럼 민감한 입력을 argv에 남기면 안 되는 경로가 이 primitive를 쓴다.
         * stdin을 명시적으로 닫은 뒤 wait해야 child process가 EOF를 보고 종료한다.
         */
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .spawn()
            .ok()?;

        let mut stdin = child.stdin.take()?;
        stdin.write_all(stdin_body.as_bytes()).ok()?;
        drop(stdin);

        let output = child.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let trimmed = stdout.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    fn find_executable(&self, program: &str) -> Option<PathBuf> {
        /*
         * readiness는 binary가 있는지와 어디에서 발견됐는지를 operator-facing detail로 보여 줄 수 있다.
         * which crate에 PATH 탐색 규칙을 맡겨 platform별 executable lookup 차이를 adapter 안에 가둔다.
         */
        which::which(program).ok()
    }

    fn gh_auth_status(&self, repo_root: Option<&str>) -> bool {
        /*
         * GitHub delivery/readiness는 `gh auth status`가 현재 repo context에서 성공하는지만 필요하다.
         * repo_root가 있으면 그 디렉터리에서 실행해 gh가 올바른 host/account configuration을 고르게 한다.
         */
        let mut command = Command::new("gh");
        command.args(["auth", "status"]);
        if let Some(repo_root) = repo_root {
            command.current_dir(repo_root);
        }
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.status().is_ok_and(|status| status.success())
    }

    fn current_timestamp(&self) -> String {
        /*
         * lease audit, distributor records, session detail events는 runtime time source를 통해 timestamp를 받는다.
         * production은 UTC RFC3339를 쓰고, tests는 fake runtime으로 deterministic value를 제공한다.
         */
        Utc::now().to_rfc3339()
    }

    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        /*
         * pool/lease 비교는 symlink와 relative path 차이에 민감하지만, cleanup 중에는 path가 아직 없을 수도 있다.
         * canonicalize가 실패해도 원래 path를 반환해 caller가 "없는 경로" 같은 상태를 계속 판정할 수 있게 한다.
         */
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn path_exists(&self, path: &Path) -> bool {
        /*
         * slot worktree, lease file, session detail directory가 실제로 남아 있는지 확인하는 가장 작은 guard다.
         * 존재 여부만 필요한 caller에게 metadata error의 세부를 노출하지 않는다.
         */
        path.exists()
    }

    fn ensure_directory_exists(&self, path: &Path) -> std::io::Result<()> {
        /*
         * pool root나 session detail directory 생성은 idempotent해야 한다.
         * 이미 있으면 성공으로 보고, 없을 때만 create_dir_all의 io error를 caller에게 보존한다.
         */
        if path.exists() {
            return Ok(());
        }

        std::fs::create_dir_all(path)
    }

    fn read_dir_paths(&self, path: &Path) -> std::io::Result<Vec<PathBuf>> {
        /*
         * pool reconciliation은 directory entry를 domain slot 후보로 다시 해석한다.
         * adapter는 path 목록만 반환하고, 어떤 entry가 slot인지/stale인지 판단하는 정책은 service에 남긴다.
         */
        std::fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect()
    }

    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        /*
         * lease와 session detail record는 service가 JSON/text schema를 해석한다.
         * adapter는 UTF-8 file read를 수행하고 io error를 숨기지 않는다.
         */
        std::fs::read_to_string(path)
    }

    fn write_string(&self, path: &Path, body: &str) -> std::io::Result<()> {
        /*
         * service가 만든 serialized lease/session detail body를 그대로 쓴다.
         * atomic 교체가 필요한 흐름은 write_string으로 temp file을 만들고 rename primitive를 이어서 사용한다.
         */
        std::fs::write(path, body)
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        /*
         * lease write의 final commit이나 temp marker 이동에 쓰이는 filesystem primitive다.
         * cross-device rename 실패 같은 세부는 caller의 recovery policy가 판단해야 하므로 그대로 반환한다.
         */
        std::fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> std::io::Result<()> {
        /*
         * stale lease/temp file cleanup은 성공/실패가 pool 상태에 영향을 준다.
         * 여기서 best-effort로 삼키지 않고 service가 blocker나 notice로 바꿀 수 있게 io::Result를 보존한다.
         */
        std::fs::remove_file(path)
    }
}
