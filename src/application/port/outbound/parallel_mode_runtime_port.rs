// parallel mode는 git worktree path, pool directory, lease files, temporary command output을 많이
// 다룬다. `Path`/`PathBuf`를 port 계약에 직접 사용해 문자열 경로 조작을 service 계층에
// 흩뿌리지 않고, filesystem 의미가 있는 값은 처음부터 path 타입으로 전달한다.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub trait ParallelPoolMutationPermit: Send {
    fn verify_pool_root(&self, pool_root: &Path) -> Result<(), String>;
}

pub trait ParallelPinnedDirectory: Send {
    fn verify(&self) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelCommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl ParallelCommandOutput {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelWorkerCommitDisposition {
    Created,
    Existing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelWorkerCommitOutcome {
    pub commit_sha: String,
    pub disposition: ParallelWorkerCommitDisposition,
}

impl ParallelWorkerCommitOutcome {
    pub fn created(commit_sha: impl Into<String>) -> Self {
        Self {
            commit_sha: commit_sha.into(),
            disposition: ParallelWorkerCommitDisposition::Created,
        }
    }

    pub fn existing(commit_sha: impl Into<String>) -> Self {
        Self {
            commit_sha: commit_sha.into(),
            disposition: ParallelWorkerCommitDisposition::Existing,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelWorkerCommitRequest<'a> {
    pub workspace_directory: &'a str,
    pub expected_branch_name: &'a str,
    pub expected_base_commit_sha: &'a str,
    pub expected_head_commit_sha: &'a str,
    pub commit_message: &'a str,
    pub commit_timestamp: &'a str,
}

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
    fn acquire_pool_mutation_permit(
        &self,
        _canonical_repo_root: &Path,
        _pool_root: &Path,
        _timeout: Duration,
        _retry_delay: Duration,
    ) -> Result<Box<dyn ParallelPoolMutationPermit>, String> {
        Err("parallel pool mutation locking is unavailable in this runtime".to_string())
    }

    fn try_acquire_pool_mutation_permit(
        &self,
        _pool_root: &Path,
    ) -> Result<Option<Box<dyn ParallelPoolMutationPermit>>, String> {
        Err("parallel pool mutation locking is unavailable in this runtime".to_string())
    }

    fn environment_variable(&self, _name: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn current_process_id(&self) -> u32 {
        0
    }

    fn required_process_start_identity(&self, _process_id: u32) -> Result<String, String> {
        Err("process start identity is unavailable in this runtime".to_string())
    }

    fn ensure_git_execution_safe(&self, _path: &Path) -> Result<(), String> {
        Err("guarded Git execution is unavailable in this runtime".to_string())
    }

    fn run_git_command(
        &self,
        _args: &[OsString],
        _stdin: Option<&[u8]>,
    ) -> Result<ParallelCommandOutput, String> {
        Err("raw Git command execution is unavailable in this runtime".to_string())
    }

    fn canonicalize_path(&self, _path: &Path) -> std::io::Result<PathBuf> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "path canonicalization is unavailable in this runtime",
        ))
    }

    fn read_directory_paths(&self, _path: &Path) -> std::io::Result<Vec<PathBuf>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "directory inspection is unavailable in this runtime",
        ))
    }

    fn path_is_symlink(&self, _path: &Path) -> std::io::Result<bool> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "path metadata inspection is unavailable in this runtime",
        ))
    }

    fn path_exists_checked(&self, path: &Path) -> std::io::Result<bool> {
        Ok(self.path_exists(path))
    }

    fn paths_match_securely(&self, _left: &Path, _right: &Path) -> bool {
        false
    }

    fn create_private_staging_directory(
        &self,
        _path: &Path,
    ) -> Result<Box<dyn ParallelPinnedDirectory>, String> {
        Err("private staging directories are unavailable in this runtime".to_string())
    }

    fn atomic_rename_noreplace(&self, _source: &Path, _destination: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "atomic no-replace rename is unavailable in this runtime",
        ))
    }

    fn read_bounded_unshared_regular_file(
        &self,
        _path: &Path,
        _max_bytes: usize,
    ) -> Option<Vec<u8>> {
        None
    }

    fn fill_secure_random(&self, _bytes: &mut [u8]) -> Result<(), String> {
        Err("secure randomness is unavailable in this runtime".to_string())
    }

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

    // A parallel model session runs without Git metadata write access. After a clean
    // TurnCompleted event, the host owns the bounded local-only staging and commit step.
    // Runtime adapters must fail closed unless they implement the full guarded operation.
    fn prepare_parallel_worker_commit(
        &self,
        _request: ParallelWorkerCommitRequest<'_>,
    ) -> Result<ParallelWorkerCommitOutcome, String> {
        Err("host-owned parallel worker commit is unavailable in this runtime".to_string())
    }

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

    // Runtime mirror를 실제 filesystem에 저장하는 adapter는 mirror root를 먼저 준비한다.
    // Windows native adapter처럼 mirror I/O가 authority-only no-op인 구현은 이 hook을
    // override해 보안 pool namespace를 일반 directory creation으로 선점하지 않는다.
    fn prepare_runtime_mirror_root(&self, path: &Path) -> std::io::Result<()> {
        self.ensure_directory_exists(path)
    }

    // Pool-local runtime mirrors are security-sensitive host metadata. The adapter must anchor
    // every relative path beneath the pinned pool root, reject links and shared objects, and
    // install a complete private file atomically. Callers never construct temporary paths.
    fn write_runtime_mirror_atomic(
        &self,
        pool_root: &Path,
        relative: &Path,
        body: &str,
    ) -> std::io::Result<()>;

    // Reads use the same pinned-root and object-identity checks as writes. A missing mirror is
    // represented explicitly; malformed or unsafe filesystem objects remain errors.
    fn read_runtime_mirror_optional(
        &self,
        pool_root: &Path,
        relative: &Path,
    ) -> std::io::Result<Option<String>>;

    // Queue recovery needs a bounded snapshot of one private mirror directory. Returned paths
    // are relative to that directory so no absolute attacker-controlled path crosses the port.
    fn read_runtime_mirror_directory(
        &self,
        pool_root: &Path,
        relative: &Path,
    ) -> std::io::Result<Vec<(PathBuf, String)>>;

    // Removal is idempotent for a missing mirror and never follows the leaf or any ancestor.
    fn remove_runtime_mirror_file(&self, pool_root: &Path, relative: &Path) -> std::io::Result<()>;

    // Lease cleanup may remove a missing mirror or the exact serialized generation it observed,
    // but must preserve a replacement body installed for a newer slot lease.
    fn remove_runtime_mirror_file_if_matches(
        &self,
        _pool_root: &Path,
        _relative: &Path,
        _expected_body: &str,
    ) -> std::io::Result<bool> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "runtime mirror compare-and-delete is unavailable",
        ))
    }

    // Recovery may recreate a missing non-authoritative mirror or advance the exact stale body it
    // observed. The adapter must perform the optional-body comparison and replacement atomically.
    fn compare_and_swap_runtime_mirror_file(
        &self,
        _pool_root: &Path,
        _relative: &Path,
        _expected_body: Option<&str>,
        _replacement_body: Option<&str>,
    ) -> std::io::Result<bool> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "runtime mirror optional compare-and-swap is unavailable",
        ))
    }

    // Lifecycle rollback may restore the previous serialized snapshot only when the mirror still
    // contains the exact next snapshot written by that transition.
    fn replace_runtime_mirror_file_if_matches(
        &self,
        _pool_root: &Path,
        _relative: &Path,
        _expected_body: &str,
        _replacement_body: &str,
    ) -> std::io::Result<bool> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "runtime mirror compare-and-replace is unavailable",
        ))
    }
}
