//! 저장소 커널 잠금이다 (plan 18 K3-4). 한 저장소의 결정 고리(턴 종료 평가, 자동 후속, 병렬 제어, PR 검증
//! 스케줄러)는 이 잠금을 쥔 프로세스 하나만 돌린다.
//!
//! 잠금의 단위는 planning 기준 데이터와 같은 저장소이다. 한 저장소의 모든 worktree 는 같은
//! `$AKRA_HOME/projects/<namespace>/runtime/` 에 닿으므로 같은 잠금 파일을 쓴다. git 이 아닌 폴더는 planning 이 그
//! 폴더에 쓰는 관리 디렉터리의 `runtime/` 을 쓴다. 폴더 안에는 아무것도 만들지 않는다. 잠금은 OS 파일 잠금이라
//! 프로세스가 끝나면 OS 가 푼다. 파일 안의 pid, 시작 시각, 폴더는 진단용이다.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::domain::planning::PlanningAuthorityLocation;

const KERNEL_LOCK_FILE_NAME: &str = "kernel.lock";
/// 진단 기록을 읽을 최대 크기이다. 기록은 세 줄이라 이것으로 충분하다.
const HOLDER_RECORD_MAX_BYTES: u64 = 4096;
const HOLDER_RECORD_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

/// 저장소 하나의 커널 잠금이다. 버리면 잠금이 풀린다.
#[derive(Debug)]
pub(crate) struct RepositoryKernelLock {
    _file: File,
    repository_key: String,
}

impl RepositoryKernelLock {
    /// 저장소 신원이다. 같은 저장소의 worktree 들은 같은 값을 갖는다. 저장소별 커널 목록의 키로 쓴다.
    pub(crate) fn repository_key(&self) -> &str {
        &self.repository_key
    }
}

/// 다른 프로세스가 이 저장소의 커널 잠금을 쥐고 있다. 호출자는 `downcast_ref` 로 이 경우를 가린다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryKernelLockHeld {
    pub(crate) repository_root: String,
    pub(crate) lock_path: PathBuf,
    pub(crate) holder: Option<String>,
}

impl fmt::Display for RepositoryKernelLockHeld {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "another Akra kernel already runs for repository `{}` (",
            self.repository_root
        )?;
        if let Some(holder) = &self.holder {
            write!(formatter, "{holder}; ")?;
        }
        write!(
            formatter,
            "lock `{}`). Only one Akra TUI or kernel daemon runs per repository, \
             including its worktrees. Close that process and try again",
            self.lock_path.display()
        )
    }
}

impl std::error::Error for RepositoryKernelLockHeld {}

pub(crate) struct RepositoryKernelLockAdapter;

impl RepositoryKernelLockAdapter {
    /// `location` 저장소의 커널 잠금을 잡는다. `location.runtime_dir` 은 이미 준비된 사용자 전용 디렉터리여야
    /// 한다. composition 이 planning 기준 데이터의 위치로 준비해서 넘긴다. 다른 프로세스가 쥐고 있으면
    /// `RepositoryKernelLockHeld` 오류를 돌려준다.
    pub(crate) fn acquire(
        location: PlanningAuthorityLocation,
        workspace_dir: &str,
    ) -> Result<RepositoryKernelLock> {
        let lock_path = Path::new(&location.runtime_dir).join(KERNEL_LOCK_FILE_NAME);
        let mut file = open_lock_file(&lock_path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(RepositoryKernelLockHeld {
                    repository_root: location.canonical_repo_root,
                    holder: read_holder_record(&lock_path),
                    lock_path,
                }
                .into());
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("failed to lock `{}`", lock_path.display()));
            }
        }
        if let Err(error) = write_holder_record(&mut file, workspace_dir) {
            // 기록은 진단용이다. 쓰지 못해도 잠금은 유효하다.
            tracing::warn!(error = %error, "kernel lock holder record write failed");
        }
        Ok(RepositoryKernelLock {
            _file: file,
            repository_key: location.repository_identity,
        })
    }
}

fn open_lock_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("failed to open `{}`", path.display()))
}

fn write_holder_record(file: &mut File, workspace_dir: &str) -> std::io::Result<()> {
    let record = format!(
        "pid={}\nstarted_at={}\nworkspace={workspace_dir}\n",
        std::process::id(),
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    file.set_len(0)?;
    file.write_all(record.as_bytes())?;
    file.flush()
}

/// 쥔 프로세스의 기록을 읽어 한 줄로 만든다. Windows 는 잠긴 파일을 다른 핸들에서 읽지 못하므로 `None` 이다.
fn read_holder_record(path: &Path) -> Option<String> {
    // 쥔 쪽이 기록을 비우고 다시 쓰는 사이에 읽으면 비어 있다. 잠깐 뒤 한 번 더 읽는다.
    read_holder_record_once(path).or_else(|| {
        std::thread::sleep(HOLDER_RECORD_RETRY_DELAY);
        read_holder_record_once(path)
    })
}

fn read_holder_record_once(path: &Path) -> Option<String> {
    let mut body = String::new();
    File::open(path)
        .ok()?
        .take(HOLDER_RECORD_MAX_BYTES)
        .read_to_string(&mut body)
        .ok()?;
    describe_holder_record(&body)
}

fn describe_holder_record(body: &str) -> Option<String> {
    let field = |name: &str| {
        body.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let pid = field("pid")?;
    let mut description = format!("held by pid {pid}");
    if let Some(workspace) = field("workspace") {
        description.push_str(&format!(" in `{workspace}`"));
    }
    if let Some(started_at) = field("started_at") {
        description.push_str(&format!(" since {started_at}"));
    }
    Some(description)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 시험이 끝나면 지우는 임시 폴더이다.
    struct TempRoot(PathBuf);

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_root(label: &str) -> TempRoot {
        static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-kernel-lock-{label}-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root should be created");
        TempRoot(root)
    }

    fn git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(directory)
            .output()
            .expect("git should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// 커밋 하나와 linked worktree 하나를 가진 저장소이다. 돌려주는 값은 (임시 폴더, 본 checkout, worktree) 이다.
    fn repository_with_linked_worktree(label: &str) -> (TempRoot, String, String) {
        let root = temp_root(label);
        let checkout = root.0.join("repo");
        std::fs::create_dir_all(&checkout).expect("checkout should be created");
        git(&checkout, &["init", "-b", "prerelease"]);
        git(&checkout, &["config", "user.name", "Akra Test"]);
        git(
            &checkout,
            &["config", "user.email", "akra-test@example.com"],
        );
        std::fs::write(checkout.join("README.md"), "seed\n").expect("seed should write");
        git(&checkout, &["add", "README.md"]);
        git(&checkout, &["commit", "-m", "seed"]);
        let worktree = root.0.join("linked-worktree");
        git(
            &checkout,
            &[
                "worktree",
                "add",
                "-b",
                "feature/linked",
                worktree.to_string_lossy().as_ref(),
            ],
        );
        (
            root,
            checkout.display().to_string(),
            worktree.display().to_string(),
        )
    }

    fn acquire(workspace_dir: &str) -> Result<RepositoryKernelLock> {
        let location = crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(
            workspace_dir,
        )?;
        RepositoryKernelLockAdapter::acquire(location, workspace_dir)
    }

    fn held_error(result: Result<RepositoryKernelLock>) -> RepositoryKernelLockHeld {
        let error = result.expect_err("the second acquire must be refused");
        error
            .downcast_ref::<RepositoryKernelLockHeld>()
            .unwrap_or_else(|| panic!("refusal must name the holder, got: {error:#}"))
            .clone()
    }

    #[test]
    fn a_linked_worktree_cannot_take_the_lock_its_repository_holds() {
        let (_root, checkout, worktree) = repository_with_linked_worktree("worktree");
        let _lock = acquire(&checkout).expect("first lock");

        let refused = held_error(acquire(&worktree));

        assert!(refused.lock_path.ends_with("runtime/kernel.lock"));
        assert!(
            !refused.lock_path.starts_with(&checkout) && !refused.lock_path.starts_with(&worktree),
            "the lock lives under AKRA_HOME, not inside a checkout"
        );
        #[cfg(unix)]
        assert_eq!(
            refused.holder,
            Some(format!(
                "held by pid {} in `{checkout}` since {}",
                std::process::id(),
                std::fs::read_to_string(&refused.lock_path)
                    .expect("holder record should be readable")
                    .lines()
                    .find_map(|line| line.strip_prefix("started_at="))
                    .expect("holder record should carry the start time")
            )),
            "the refusal names the holding process and its folder"
        );
        assert!(
            refused
                .to_string()
                .starts_with("another Akra kernel already runs for repository"),
            "{refused}"
        );
        // 같은 폴더의 두 번째 잠금도 거절된다.
        held_error(acquire(&checkout));
    }

    #[test]
    fn different_repositories_hold_their_own_locks() {
        let (_first_root, first, _) = repository_with_linked_worktree("first");
        let (_second_root, second, _) = repository_with_linked_worktree("second");

        let first_lock = acquire(&first).expect("first repository");
        let second_lock = acquire(&second).expect("second repository");

        assert_ne!(first_lock.repository_key(), second_lock.repository_key());
    }

    #[test]
    fn dropping_the_lock_releases_it_for_the_next_process() {
        let (_root, checkout, worktree) = repository_with_linked_worktree("release");
        let lock = acquire(&worktree).expect("first lock");
        let repository_key = lock.repository_key().to_string();
        drop(lock);

        let next = acquire(&checkout).expect("lock after release");

        assert_eq!(next.repository_key(), repository_key);
    }

    #[test]
    fn a_plain_folder_locks_by_its_own_path_outside_the_folder() {
        let folder = temp_root("plain");
        let workspace = folder.0.display().to_string();
        let _lock = acquire(&workspace).expect("plain folder lock");

        let refused = held_error(acquire(&workspace));

        assert!(
            !refused.lock_path.starts_with(&folder.0),
            "the lock lives under AKRA_HOME, not inside the folder"
        );
    }

    #[test]
    fn holder_record_describes_pid_folder_and_start_time() {
        let body = "pid=4242\nstarted_at=2026-11-09T01:02:03Z\nworkspace=/repo/worktree\n";

        assert_eq!(
            describe_holder_record(body).as_deref(),
            Some("held by pid 4242 in `/repo/worktree` since 2026-11-09T01:02:03Z")
        );
        assert_eq!(describe_holder_record(""), None, "no pid, no description");
    }

    #[test]
    fn held_error_names_the_repository_holder_and_lock_path() {
        let error = RepositoryKernelLockHeld {
            repository_root: "/repo".to_string(),
            lock_path: PathBuf::from("/akra/projects/repo-1/runtime/kernel.lock"),
            holder: Some("held by pid 7".to_string()),
        };

        assert_eq!(
            error.to_string(),
            "another Akra kernel already runs for repository `/repo` (held by pid 7; lock \
             `/akra/projects/repo-1/runtime/kernel.lock`). Only one Akra TUI or kernel daemon \
             runs per repository, including its worktrees. Close that process and try again"
        );
    }
}
