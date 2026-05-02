// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::*;

/*
학습 주석: pool root는 저장소 내부가 아니라 저장소 옆 형제 디렉터리에 둡니다. 병렬 slot
worktree들이 원본 repo 안에 생기면 git status, cargo scan, editor search가 slot 파일까지
뒤섞어 볼 수 있기 때문입니다. repo 이름과 canonical path hash를 함께 쓰면 같은 이름의
repo가 다른 위치에 있어도 pool root가 충돌하지 않습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(in crate::application::service::parallel_mode) fn derive_default_pool_root(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    canonical_repo_root: &Path,
) -> PathBuf {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let repo_name = canonical_repo_root
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .file_name()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .and_then(|name| name.to_str())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .filter(|name| !name.is_empty())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .unwrap_or("workspace");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let parent_dir = canonical_repo_root.parent().unwrap_or(canonical_repo_root);

    parent_dir
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join(format!("{repo_name}-akra-worktrees"))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join(stable_short_hash(&canonical_repo_root.to_string_lossy()))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("akra-pool")
}

/*
학습 주석: 이 hash는 보안용이 아니라 경로 충돌 방지용 안정 식별자입니다. 같은 canonical
repo path는 항상 같은 pool root를 얻어야 하고, 서로 다른 checkout은 같은 repo 이름이어도
다른 pool root를 얻어야 합니다. 짧은 FNV-1a 값이면 디렉터리 이름을 과하게 길게 만들지
않으면서 이 목적을 달성할 수 있습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn stable_short_hash(value: &str) -> String {
    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    const FNV_PRIME: u64 = 0x100000001b3;

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut hash = FNV_OFFSET;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..12].to_string()
}

/*
학습 주석: pool baseline head는 local branch, origin branch, 현재 HEAD 순서로 찾습니다.
reconcile 초기에 local baseline이 없을 수 있고, freshly cloned workspace에서는 origin
baseline만 있을 수 있습니다. 둘 다 없을 때 HEAD를 fallback으로 쓰면 최초 pool 생성이
완전히 막히지 않습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn resolve_pool_baseline_head(repo_root: &str) -> Option<String> {
    resolve_branch_head(repo_root, POOL_BASELINE_BRANCH)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .or_else(|| {
            resolve_branch_head(
                repo_root,
                &format!("refs/remotes/origin/{POOL_BASELINE_BRANCH}"),
            )
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .or_else(|| {
            run_command(
                "git",
                ["-C", repo_root, "rev-parse", "--verify", "HEAD"],
                None,
            )
        })
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn resolve_branch_head(repo_root: &str, branch_name: &str) -> Option<String> {
    /*
    학습 주석: branch head resolution은 pool reconcile이 baseline과 agent branch의 실제 commit을
    비교할 때 쓰는 가장 작은 git query입니다. branch name은 local branch, remote tracking ref,
    혹은 HEAD처럼 `git rev-parse`가 이해하는 refspec 그대로 들어옵니다. 실패를 오류로 올리지
    않고 `None`으로 접는 이유는 caller가 local prerelease 부재, remote만 존재, fresh repo 같은
    정상적인 fallback 순서를 직접 결정해야 하기 때문입니다.
    */
    run_command("git", ["-C", repo_root, "rev-parse", branch_name], None)
}

/*
학습 주석: `git worktree list --porcelain` 출력은 줄 기반 record 묶음입니다. 이 parser는
worktree path, HEAD sha, branch, detached 여부를 `GitWorktreeRecord`로 정규화합니다.
slot inspection과 reconcile은 이 정규화된 inventory를 기준으로 "slot path가 실제 git
worktree인가"를 판단합니다.

빈 줄을 만날 때마다 builder를 flush하는 구조라 마지막 record가 trailing blank 없이 끝나도
`chain(std::iter::once(""))` 덕분에 빠지지 않습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn parse_worktree_records(output: &str) -> Vec<GitWorktreeRecord> {
    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[derive(Default)]
    // 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
    struct Builder {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        path: Option<PathBuf>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        head_sha: Option<String>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        branch_name: Option<String>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        detached: bool,
    }

    // 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
    impl Builder {
        // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
        fn build(self) -> Option<GitWorktreeRecord> {
            Some(GitWorktreeRecord {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                path: self.path?,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                head_sha: self.head_sha.unwrap_or_default(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                branch_name: self.branch_name,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                detached: self.detached,
            })
        }
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut records = Vec::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut current = Builder::default();

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for line in output.lines().chain(std::iter::once("")) {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if line.is_empty() {
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(record) = std::mem::take(&mut current).build() {
                records.push(record);
            }
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let Some(path) = line.strip_prefix("worktree ") {
            current.path = Some(PathBuf::from(path));
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let Some(head_sha) = line.strip_prefix("HEAD ") {
            current.head_sha = Some(head_sha.to_string());
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let Some(branch_name) = line.strip_prefix("branch refs/heads/") {
            current.branch_name = Some(branch_name.to_string());
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if line == "detached" {
            current.detached = true;
        }
    }

    records
}

/*
학습 주석: git status inspection은 slot을 자동으로 reset하거나 cleanup해도 되는지 판단하는
공통 안전 게이트입니다. porcelain status로 staged, unstaged, untracked 변경을 보고,
git dir 안의 MERGE_HEAD/REBASE_HEAD/CHERRY_PICK_HEAD 같은 파일로 진행 중인 작업도 감지합니다.
파일 변경이 없더라도 rebase metadata가 남아 있으면 자동 조작은 위험하기 때문입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(in crate::application::service::parallel_mode) fn inspect_slot_git_status(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    slot_path: &Path,
) -> Option<SlotGitStatus> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slot_path_string = slot_path.display().to_string();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut status = SlotGitStatus::default();
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for line in status_output.lines().skip(1) {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if line.starts_with("??") {
            status.has_untracked = true;
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let x = line.chars().next().unwrap_or(' ');
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let y = line.chars().nth(1).unwrap_or(' ');
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if x != ' ' {
            status.has_staged = true;
        }
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if y != ' ' {
            status.has_unstaged = true;
        }
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let git_dir = resolve_git_dir(slot_path)?;
    status.has_pending_operation = [
        "MERGE_HEAD",
        "REBASE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
    ]
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .into_iter()
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .any(|path| git_dir.join(path).exists());

    Some(status)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn resolve_git_dir(slot_path: &Path) -> Option<PathBuf> {
    /*
    학습 주석: worktree의 `.git`은 일반 디렉터리일 수도 있고, common git dir을 가리키는 파일일
    수도 있습니다. `git rev-parse --git-dir`를 쓰면 두 경우를 git이 직접 해석해 주므로,
    slot cleanup이나 pending-operation 검사에서 잘못된 `.git` 경로를 추측하지 않아도 됩니다.
    반환 경로가 상대 경로일 수 있어 아래에서 slot path 기준 절대 경로로 보정합니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slot_path_string = slot_path.display().to_string();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let git_dir = run_command(
        "git",
        ["-C", slot_path_string.as_str(), "rev-parse", "--git-dir"],
        None,
    )?;
    Some(absolutize_path(slot_path, Path::new(&git_dir)))
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn absolutize_path(base_dir: &Path, path: &Path) -> PathBuf {
    /*
    학습 주석: git command output은 호출 위치와 git 설정에 따라 절대 경로 또는 상대 경로가 될 수
    있습니다. 이 helper는 상대 경로를 slot path 기준으로 붙여, 이후 `MERGE_HEAD` 같은 파일 존재
    확인이 현재 프로세스의 working directory에 의존하지 않게 만듭니다.
    */
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn display_pool_path(canonical_repo_root: &Path, path: &Path) -> String {
    /*
    학습 주석: pool worktree는 repo sibling directory 아래에 생성되므로 전체 절대 경로를 그대로
    보여 주면 TUI board가 길고 noisy해집니다. canonical repo의 parent를 표시 root로 삼아
    `repo-akra-worktrees/hash/akra-pool/slot-1`처럼 사람이 비교하기 쉬운 상대 label을 만들고,
    prefix stripping이 실패하면 절대 경로를 fallback으로 사용합니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let display_root = canonical_repo_root.parent().unwrap_or(canonical_repo_root);
    path.strip_prefix(display_root)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|relative| relative.display().to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .unwrap_or_else(|_| path.display().to_string())
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn annotate_worktree_label(base_label: String, detail: &str) -> String {
    /*
    학습 주석: pool board의 slot label은 기본 경로와 git 상태 detail을 합쳐 한 줄로 보여 줍니다.
    `clean`은 정상 상태라 중복 표시하지 않고, dirty, pending operation, branch mismatch 같은
    운영자가 봐야 하는 detail만 slash 뒤에 붙입니다. 이 함수가 표시 규칙을 모아 두어 slot
    inspection 쪽은 상태 판정에 집중할 수 있습니다.
    */
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if detail.is_empty() || detail == "clean" {
        base_label
    } else {
        format!("{base_label} / {detail}")
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn canonicalize_best_effort(path: &Path) -> PathBuf {
    /*
    학습 주석: canonicalize는 symlink와 `..`를 실제 경로로 접어 주지만, 아직 생성되지 않은 slot이나
    테스트 중 제거된 worktree에서는 실패할 수 있습니다. 여기서는 실패를 치명 오류로 만들지 않고
    원래 path를 보존해, caller가 존재하지 않는 경로도 비교와 표시 흐름에서 계속 다룰 수 있게
    합니다.
    */
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/*
학습 주석: worktree path 비교는 symlink나 상대 경로 때문에 문자열 비교만으로는 부족합니다.
canonicalize가 성공하면 실제 경로를 비교하고, 실패하면 원래 path를 fallback으로 사용합니다.
이 best-effort 비교는 nested directory에서 lease를 찾거나 slot path와 lease path를 맞출 때
불필요한 mismatch를 줄입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn worktree_paths_match(left: &Path, right: &Path) -> bool {
    canonicalize_best_effort(left) == canonicalize_best_effort(right)
}
