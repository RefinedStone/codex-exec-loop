// parallel mode는 git worktree path, pool directory, lease files, temporary command output을 많이
// 다룬다. `Path`/`PathBuf`를 port 계약에 직접 사용해 문자열 경로 조작을 service 계층에
// 흩뿌리지 않고, filesystem 의미가 있는 값은 처음부터 path 타입으로 전달한다.
use std::path::{Path, PathBuf};

// `ParallelModeRuntimePort`는 parallel mode application service가 OS, git, gh, filesystem에
// 닿는 작은 capability를 묶은 runtime boundary이다. pool/readiness/distributor/slot lifecycle은
// 이 trait만 보고 실행 환경을 관찰하거나 파일을 수정하므로, 테스트에서는 fake runtime으로 command
// 결과와 filesystem 상태를 재현할 수 있다.
//
// 이 port는 일부러 low-level 함수들이 많다. parallel mode는 worktree 생성/정리, branch 검증,
// GitHub auth 확인, lease 파일 I/O처럼 순서가 중요한 작업을 조합하므로, 큰 "do everything"
// adapter보다 작은 primitive를 주입받는 편이 각 service의 정책을 application 계층에 남기기 쉽다.
// 즉 adapter는 명령 실행과 filesystem 호출을 맡고, 어떤 순서로 recovery/readiness/cleanup을
// 진행할지는 service가 결정한다.
pub trait ParallelModeRuntimePort: Send + Sync {
    // workspace가 속한 git repository root를 찾는다. pool service는 이 root를 기준으로 worktree
    // pool path와 baseline branch 상태를 계산하고, repo 밖에서 parallel mode가 켜지는 경우를 blocked로 돌린다.
    fn detect_git_repo_root(&self, workspace_dir: &str) -> Option<String>;

    // 출력이 필요 없는 command readiness probe이다. 예를 들어 `git`, `gh` 같은 도구가 특정
    // 인자로 성공하는지만 확인할 때 사용하며, stdout parsing이 필요 없는 capability check를 단순화한다.
    fn command_succeeds(&self, program: &str, args: &[&str]) -> bool;

    // stdout이 필요한 외부 명령을 실행한다. 실패, non-zero exit, invalid utf8 등은 service
    // 정책에서 쉽게 분기하도록 `None`으로 축약한다.
    fn run_command(
        &self,
        // 실행할 binary 이름이다. production adapter는 PATH에서 찾고, fake runtime은 이 값을 key로 삼는다.
        program: &str,
        // command arguments이다. slice로 받아 호출자가 임시 Vec 없이 static args를 넘길 수 있다.
        args: &[&str],
        // git command처럼 특정 repo root에서 실행해야 하는 경우에만 current_dir를 지정한다.
        current_dir: Option<&str>,
    ) -> Option<String>;

    // stdin을 요구하는 command 실행 primitive이다. GitHub fallback script처럼 token/credentials를
    // stdin으로 넘겨야 하는 흐름이 command line argument에 민감 정보를 남기지 않도록 이 경로를 쓴다.
    fn run_command_with_stdin(
        &self,
        // 실행할 binary 이름이다.
        program: &str,
        // command arguments이다.
        args: &[&str],
        // command stdin으로 전달할 본문이다.
        stdin_body: &str,
    ) -> Option<String>;

    // 특정 executable이 PATH나 adapter가 정한 탐색 경로에 있는지 찾는다. readiness projection은
    // 이 값으로 기능 가능/불가능을 설명하고, service는 binary 탐색 규칙을 직접 알 필요가 없다.
    fn find_executable(&self, program: &str) -> Option<PathBuf>;

    // GitHub CLI 인증이 현재 repo/root 문맥에서 유효한지 확인한다. distributor delivery와 review
    // polling은 gh auth가 없으면 진행할 수 없으므로 readiness가 이 primitive를 사용한다.
    fn gh_auth_status(&self, repo_root: Option<&str>) -> bool;

    // audit/log/lease timestamp에 쓸 현재 시간을 runtime에서 제공한다. 테스트 fake는 deterministic
    // timestamp를 돌려 snapshot과 persisted lease fixture를 안정화할 수 있다.
    fn current_timestamp(&self) -> String;

    // 아래부터는 filesystem primitive이다. parallel mode는 pool slot, lease, session detail 파일을
    // 조작하지만, service 정책과 실제 `std::fs` 호출을 분리하기 위해 모두 port 뒤로 둔다.

    // 가능한 경우 path를 canonicalize하고, 실패하면 합리적인 best-effort path를 반환한다. pool
    // 비교는 symlink/relative path 차이에 민감하므로 이 helper를 runtime 경계에 둔다.
    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf;

    // 파일이나 directory 존재 여부를 확인한다. pool reconcile과 lease cleanup에서 destructive
    // operation 전에 guard로 사용한다.
    fn path_exists(&self, path: &Path) -> bool;

    // pool/session detail directory를 생성한다. 실패는 caller가 사용자-facing error로 바꿔야 하므로
    // `std::io::Result`를 그대로 보존한다.
    fn ensure_directory_exists(&self, path: &Path) -> std::io::Result<()>;

    // directory entries를 path 목록으로 읽는다. pool board/reconcile은 이 목록을 domain slot 상태로 매핑한다.
    fn read_dir_paths(&self, path: &Path) -> std::io::Result<Vec<PathBuf>>;

    // lease/session detail 같은 UTF-8 text 파일을 읽는다.
    fn read_to_string(&self, path: &Path) -> std::io::Result<String>;

    // lease/session detail text 파일을 쓴다. atomic write가 필요한 caller는 임시 파일과 rename을 조합한다.
    fn write_string(&self, path: &Path, body: &str) -> std::io::Result<()>;

    // temporary file을 최종 path로 교체하거나 slot marker를 이동할 때 사용하는 filesystem rename이다.
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;

    // stale lease나 임시 파일을 삭제한다. 삭제 실패는 cleanup/reconcile 정책에서 판단해야 하므로
    // io error를 숨기지 않는다.
    fn remove_file(&self, path: &Path) -> std::io::Result<()>;
}
