// 학습 주석: parallel mode는 git worktree path, pool directory, lease files, temporary command output을 많이 다룹니다.
// `Path`/`PathBuf`를 port 계약에 직접 사용해 문자열 경로 조작을 서비스 계층에 흩뿌리지 않게 합니다.
use std::path::{Path, PathBuf};

// 학습 주석: `ParallelModeRuntimePort`는 parallel mode application service가 OS, git, gh, filesystem에 닿는 모든
// 작은 capability를 묶은 runtime boundary입니다. pool/readiness/distributor/slot lifecycle은 이 trait만 보고
// 실행 환경을 관찰하거나 파일을 수정하므로, 테스트에서는 fake runtime으로 command 결과와 filesystem 상태를 재현할 수 있습니다.
//
// 학습 주석: 이 port는 일부러 low-level 함수들이 많습니다. parallel mode는 worktree 생성/정리, branch 검증,
// GitHub auth 확인, lease 파일 I/O처럼 순서가 중요한 작업을 조합하므로, 큰 "do everything" adapter보다 작은 primitive를
// 주입받는 편이 각 service의 정책을 application 계층에 남기기 쉽습니다.
pub trait ParallelModeRuntimePort: Send + Sync {
    // 학습 주석: workspace가 속한 git repository root를 찾습니다. pool service는 이 root를 기준으로
    // worktree pool path와 baseline branch 상태를 계산합니다.
    fn detect_git_repo_root(&self, workspace_dir: &str) -> Option<String>;

    // 학습 주석: 출력이 필요 없는 command readiness probe입니다. 예를 들어 `git`, `gh` 같은 도구가
    // 특정 인자로 성공하는지만 확인할 때 사용합니다.
    fn command_succeeds(&self, program: &str, args: &[&str]) -> bool;

    // 학습 주석: stdout이 필요한 외부 명령을 실행합니다. 실패, non-zero exit, invalid utf8 등은
    // service 정책에서 쉽게 분기하도록 `None`으로 축약합니다.
    fn run_command(
        &self,
        // 학습 주석: 실행할 binary 이름입니다. production adapter는 PATH에서 찾고, fake runtime은 이 값을 key로 삼습니다.
        program: &str,
        // 학습 주석: command arguments입니다. slice로 받아 호출자가 임시 Vec 없이 static args를 넘길 수 있습니다.
        args: &[&str],
        // 학습 주석: git command처럼 특정 repo root에서 실행해야 하는 경우에만 current_dir를 지정합니다.
        current_dir: Option<&str>,
    ) -> Option<String>;

    // 학습 주석: stdin을 요구하는 command 실행 primitive입니다. GitHub fallback script처럼 token/credentials를
    // stdin으로 넘겨야 하는 흐름이 command line argument에 민감 정보를 남기지 않도록 이 경로를 씁니다.
    fn run_command_with_stdin(
        &self,
        // 학습 주석: 실행할 binary 이름입니다.
        program: &str,
        // 학습 주석: command arguments입니다.
        args: &[&str],
        // 학습 주석: command stdin으로 전달할 본문입니다.
        stdin_body: &str,
    ) -> Option<String>;

    // 학습 주석: 특정 executable이 PATH나 adapter가 정한 탐색 경로에 있는지 찾습니다.
    // readiness projection은 이 값으로 기능 가능/불가능을 설명합니다.
    fn find_executable(&self, program: &str) -> Option<PathBuf>;

    // 학습 주석: GitHub CLI 인증이 현재 repo/root 문맥에서 유효한지 확인합니다. distributor delivery와 review polling은
    // gh auth가 없으면 진행할 수 없으므로 readiness가 이 primitive를 사용합니다.
    fn gh_auth_status(&self, repo_root: Option<&str>) -> bool;

    // 학습 주석: audit/log/lease timestamp에 쓸 현재 시간을 runtime에서 제공합니다.
    // 테스트 fake는 deterministic timestamp를 돌려 snapshot을 안정화할 수 있습니다.
    fn current_timestamp(&self) -> String;

    // 학습 주석: 아래부터는 filesystem primitive입니다. parallel mode는 pool slot, lease, session detail 파일을
    // 조작하지만, service 정책과 실제 `std::fs` 호출을 분리하기 위해 모두 port 뒤로 둡니다.

    // 학습 주석: 가능한 경우 path를 canonicalize하고, 실패하면 합리적인 best-effort path를 반환합니다.
    // pool 비교는 symlink/relative path 차이에 민감하므로 이 helper를 runtime 경계에 둡니다.
    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf;

    // 학습 주석: 파일이나 directory 존재 여부를 확인합니다. pool reconcile과 lease cleanup에서 guard로 사용합니다.
    fn path_exists(&self, path: &Path) -> bool;

    // 학습 주석: pool/session detail directory를 생성합니다. 실패는 caller가 사용자-facing error로 바꿔야 하므로
    // `std::io::Result`를 그대로 보존합니다.
    fn ensure_directory_exists(&self, path: &Path) -> std::io::Result<()>;

    // 학습 주석: directory entries를 path 목록으로 읽습니다. pool board/reconcile은 이 목록을 domain slot 상태로 매핑합니다.
    fn read_dir_paths(&self, path: &Path) -> std::io::Result<Vec<PathBuf>>;

    // 학습 주석: lease/session detail 같은 UTF-8 text 파일을 읽습니다.
    fn read_to_string(&self, path: &Path) -> std::io::Result<String>;

    // 학습 주석: lease/session detail text 파일을 씁니다. atomic write가 필요한 caller는 임시 파일과 rename을 조합합니다.
    fn write_string(&self, path: &Path, body: &str) -> std::io::Result<()>;

    // 학습 주석: temporary file을 최종 path로 교체하거나 slot marker를 이동할 때 사용하는 filesystem rename입니다.
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;

    // 학습 주석: stale lease나 임시 파일을 삭제합니다. 삭제 실패는 cleanup/reconcile 정책에서 판단해야 하므로
    // io error를 숨기지 않습니다.
    fn remove_file(&self, path: &Path) -> std::io::Result<()>;
}
