use super::*;

/*
pool root는 저장소 내부가 아니라 저장소 옆 형제 디렉터리에 둔다. 병렬 slot
worktree들이 원본 repo 안에 생기면 git status, cargo scan, editor search가 slot 파일까지
뒤섞어 볼 수 있기 때문이다. repo 이름과 canonical path hash를 함께 쓰면 같은 이름의
repo가 다른 위치에 있어도 pool root가 충돌하지 않는다.
*/
pub(in crate::application::service::parallel_mode) fn derive_default_pool_root(
    canonical_repo_root: &Path,
) -> PathBuf {
    // repository root가 filesystem root처럼 이름을 얻을 수 없는 경로여도 pool
    // 계산은 계속되어야 하므로 표시 이름만 안전한 fallback으로 바꾼다.
    let repo_name = canonical_repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    // sibling directory를 기준으로 잡아 source checkout 내부에 slot worktree가
    // 생기지 않게 한다. parent가 없으면 root 자체를 기준으로 삼아 path 계산만 유지한다.
    let parent_dir = canonical_repo_root.parent().unwrap_or(canonical_repo_root);

    parent_dir
        .join(format!("{repo_name}-akra-worktrees"))
        .join(stable_short_hash(&canonical_repo_root.to_string_lossy()))
        .join("akra-pool")
}

/*
이 hash는 보안용이 아니라 경로 충돌 방지용 안정 식별자이다. 같은 canonical
repo path는 항상 같은 pool root를 얻어야 하고, 서로 다른 checkout은 같은 repo 이름이어도
다른 pool root를 얻어야 한다. 짧은 FNV-1a 값이면 디렉터리 이름을 과하게 길게 만들지
않으면서 이 목적을 달성할 수 있다.
*/
fn stable_short_hash(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    // path key만 만들 목적이므로 별도 dependency 없이 고정된 byte 순회로
    // 플랫폼마다 같은 checkout 경로가 같은 짧은 suffix를 얻도록 한다.
    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..12].to_string()
}

/*
pool baseline head는 표준 remote branch를 먼저 보고, read-only inspection에서만 local branch를
fallback으로 쓴다. mutating reconcile은 별도 guard에서 missing remote 표준 branch를 현재 HEAD로
seed하거나 remote 기준으로 local branch를 맞춘다.
*/
pub(super) fn resolve_pool_baseline_head(repo_root: &str) -> Option<String> {
    resolve_branch_head(
        repo_root,
        &remote_tracking_branch_ref(DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH),
    )
    .or_else(|| resolve_branch_head(repo_root, POOL_BASELINE_BRANCH))
}

pub(super) fn resolve_branch_head(repo_root: &str, branch_name: &str) -> Option<String> {
    /*
    branch head resolution은 pool reconcile이 baseline과 agent branch의 실제 commit을
    비교할 때 쓰는 가장 작은 git query이다. branch name은 local branch, remote tracking ref,
    혹은 HEAD처럼 `git rev-parse`가 이해하는 refspec 그대로 들어온다. 실패를 오류로 올리지
    않고 `None`으로 접는 이유는 caller가 local prerelease 부재, remote만 존재, fresh repo 같은
    정상적인 fallback 순서를 직접 결정해야 하기 때문이다.
    */
    run_command("git", ["-C", repo_root, "rev-parse", branch_name], None)
}

/*
`git worktree list --porcelain` 출력은 줄 기반 record 묶음이다. 이 parser는
worktree path, HEAD sha, branch, detached 여부를 `GitWorktreeRecord`로 정규화한다.
slot inspection과 reconcile은 이 정규화된 inventory를 기준으로 "slot path가 실제 git
worktree인가"를 판단한다.

빈 줄을 만날 때마다 builder를 flush하는 구조라 마지막 record가 trailing blank 없이 끝나도
`chain(std::iter::once(""))` 덕분에 빠지지 않는다.
*/
pub(super) fn parse_worktree_records(output: &str) -> Vec<GitWorktreeRecord> {
    // porcelain record는 줄 순서가 고정되어 있지만 중간 필드가 빠질 수 있다.
    // local builder가 record 하나의 부분 상태만 들고 있다가 path가 있는 묶음만 최종 inventory로 낸다.
    #[derive(Default)]
    struct Builder {
        path: Option<PathBuf>,
        head_sha: Option<String>,
        branch_name: Option<String>,
        detached: bool,
    }

    impl Builder {
        fn build(self) -> Option<GitWorktreeRecord> {
            Some(GitWorktreeRecord {
                path: self.path?,
                head_sha: self.head_sha.unwrap_or_default(),
                branch_name: self.branch_name,
                detached: self.detached,
            })
        }
    }

    let mut records = Vec::new();
    let mut current = Builder::default();

    for line in output.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(record) = std::mem::take(&mut current).build() {
                records.push(record);
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            current.path = Some(PathBuf::from(path));
            continue;
        }
        if let Some(head_sha) = line.strip_prefix("HEAD ") {
            current.head_sha = Some(head_sha.to_string());
            continue;
        }
        // `refs/heads/` prefix를 제거해 board와 reconciler가 local branch 이름만
        // 비교하게 한다. detached record는 branch line 없이 별도 marker로 들어온다.
        if let Some(branch_name) = line.strip_prefix("branch refs/heads/") {
            current.branch_name = Some(branch_name.to_string());
            continue;
        }
        if line == "detached" {
            current.detached = true;
        }
    }

    records
}

/*
git status inspection은 slot을 자동으로 reset하거나 cleanup해도 되는지 판단하는
공통 안전 게이트이다. porcelain status로 staged, unstaged, untracked 변경을 보고,
git dir 안의 MERGE_HEAD/rebase-merge/rebase-apply/CHERRY_PICK_HEAD 같은 metadata로 진행 중인
작업도 감지한다. 파일 변경이 없더라도 실제 operation metadata가 남아 있으면 자동 조작은
위험하기 때문이다.
*/
pub(in crate::application::service::parallel_mode) fn inspect_slot_git_status(
    slot_path: &Path,
) -> Option<SlotGitStatus> {
    let slot_path_string = slot_path.display().to_string();
    let status_output = run_command(
        "git",
        [
            "-C",
            slot_path_string.as_str(),
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=all",
        ],
        None,
    )?;

    let mut status = SlotGitStatus::default();
    for line in status_output.lines().skip(1) {
        // porcelain v1에서 `??`는 index/worktree column 의미가 없는 untracked
        // marker라 staged/unstaged 판정으로 흘리지 않고 별도 flag만 세운다.
        if line.starts_with("??") {
            status.has_untracked = true;
            continue;
        }

        // porcelain v1의 첫 두 칸은 각각 index(X)와 worktree(Y) 상태이다.
        // pool cleanup은 두 종류의 사용자 변경을 따로 보고해야 자동 reset 판단을 보수적으로 할 수 있다.
        let x = line.chars().next().unwrap_or(' ');
        let y = line.chars().nth(1).unwrap_or(' ');
        if x != ' ' {
            status.has_staged = true;
        }
        if y != ' ' {
            status.has_unstaged = true;
        }
    }

    // status output에는 merge/rebase 진행 중 metadata가 항상 직접 드러나지 않으므로
    // git dir을 별도로 찾아 자동 조작을 막아야 하는 pending 상태까지 합산한다.
    let git_dir = resolve_git_dir(slot_path)?;
    status.has_pending_operation = [
        "MERGE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
    ]
    .into_iter()
    .any(|path| git_dir.join(path).exists());

    Some(status)
}

pub(super) fn resolve_git_dir(slot_path: &Path) -> Option<PathBuf> {
    /*
    worktree의 `.git`은 일반 디렉터리일 수도 있고, common git dir을 가리키는 파일일
    수도 있다. `git rev-parse --git-dir`를 쓰면 두 경우를 git이 직접 해석해 주므로,
    slot cleanup이나 pending-operation 검사에서 잘못된 `.git` 경로를 추측하지 않아도 된다.
    반환 경로가 상대 경로일 수 있어 아래에서 slot path 기준 절대 경로로 보정한다.
    */
    let slot_path_string = slot_path.display().to_string();
    let git_dir = run_command(
        "git",
        ["-C", slot_path_string.as_str(), "rev-parse", "--git-dir"],
        None,
    )?;
    Some(absolutize_path(slot_path, Path::new(&git_dir)))
}

fn absolutize_path(base_dir: &Path, path: &Path) -> PathBuf {
    /*
    git command output은 호출 위치와 git 설정에 따라 절대 경로 또는 상대 경로가 될 수
    있다. 이 helper는 상대 경로를 slot path 기준으로 붙여, 이후 `MERGE_HEAD` 같은 파일 존재
    확인이 현재 프로세스의 working directory에 의존하지 않게 만든다.
    */
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

pub(super) fn display_pool_path(canonical_repo_root: &Path, path: &Path) -> String {
    /*
    pool worktree는 repo sibling directory 아래에 생성되므로 전체 절대 경로를 그대로
    보여 주면 TUI board가 길고 noisy해진다. canonical repo의 parent를 표시 root로 삼아
    `repo-akra-worktrees/hash/akra-pool/slot-1`처럼 사람이 비교하기 쉬운 상대 label을 만들고,
    prefix stripping이 실패하면 절대 경로를 fallback으로 사용한다.
    */
    let display_root = canonical_repo_root.parent().unwrap_or(canonical_repo_root);
    path.strip_prefix(display_root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

pub(super) fn annotate_worktree_label(base_label: String, detail: &str) -> String {
    /*
    pool board의 slot label은 기본 경로와 git 상태 detail을 합쳐 한 줄로 보여 준다.
    `clean`은 정상 상태라 중복 표시하지 않고, dirty, pending operation, branch mismatch 같은
    운영자가 봐야 하는 detail만 slash 뒤에 붙인다. 이 함수가 표시 규칙을 모아 두어 slot
    inspection 쪽은 상태 판정에 집중할 수 있다.
    */
    if detail.is_empty() || detail == "clean" {
        base_label
    } else {
        format!("{base_label} / {detail}")
    }
}

pub(super) fn canonicalize_best_effort(path: &Path) -> PathBuf {
    /*
    canonicalize는 symlink와 `..`를 실제 경로로 접어 주지만, 아직 생성되지 않은 slot이나
    테스트 중 제거된 worktree에서는 실패할 수 있다. 여기서는 실패를 치명 오류로 만들지 않고
    원래 path를 보존해, caller가 존재하지 않는 경로도 비교와 표시 흐름에서 계속 다룰 수 있게
    한다.
    */
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/*
worktree path 비교는 symlink나 상대 경로 때문에 문자열 비교만으로는 부족하다.
canonicalize가 성공하면 실제 경로를 비교하고, 실패하면 원래 path를 fallback으로 사용한다.
이 best-effort 비교는 nested directory에서 lease를 찾거나 slot path와 lease path를 맞출 때
불필요한 mismatch를 줄인다.
*/
pub(super) fn worktree_paths_match(left: &Path, right: &Path) -> bool {
    canonicalize_best_effort(left) == canonicalize_best_effort(right)
}

#[cfg(test)]
mod tests {
    use super::resolve_git_dir;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_repo(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();

        std::env::temp_dir().join(format!(
            "akra-pool-paths-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()
            .expect("git command should run");

        assert!(status.success(), "git command failed: {args:?}");
    }

    #[test]
    fn resolve_git_dir_absolutizes_relative_git_dir_output() {
        let repo = unique_repo("relative-git-dir");
        fs::create_dir_all(&repo).expect("repo directory should be created");
        run_git(&repo, &["init", "-q"]);

        assert_eq!(
            resolve_git_dir(&repo).expect("git directory should resolve"),
            repo.join(".git")
        );

        fs::remove_dir_all(&repo).expect("repo directory should be removed");
    }

    #[test]
    fn resolve_git_dir_returns_none_outside_git_repository() {
        let workspace = unique_repo("not-a-repo");
        fs::create_dir_all(&workspace).expect("workspace directory should be created");

        assert_eq!(resolve_git_dir(&workspace), None);

        fs::remove_dir_all(&workspace).expect("workspace directory should be removed");
    }
}
