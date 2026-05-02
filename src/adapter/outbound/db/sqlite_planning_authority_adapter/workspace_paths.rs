/*
학습 주석:
이 모듈은 SQLite planning authority store가 어느 경로에 있어야 하는지 결정하는
경로 해석 계층입니다.

repo-scoped planning authority의 핵심은 "현재 process가 어느 worktree나 하위 디렉터리에서
실행되더라도 같은 Git repository에는 같은 authority DB를 사용한다"는 점입니다. 그래서 단순히
`workspace_dir/.akra/...` 같은 경로를 쓰지 않고, 먼저 Git이 보는 canonical repository root를
찾은 뒤 그 값을 안정적인 project 관리 디렉터리 이름으로 바꿉니다.

이 파일은 outbound DB adapter 내부 helper지만 application 계층의 의미와 강하게 연결됩니다.
`PlanningAuthorityLocation`은 이후 store, draft_files, active_documents, runtime_projection
모듈이 모두 공유하는 기준 좌표입니다.
*/
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use anyhow::Result;

use crate::domain::planning::PlanningAuthorityLocation;

use super::SqlitePlanningAuthorityAdapter;

/*
학습 주석:
경로 정책을 나타내는 작은 상수들입니다.

`AKRA_HOME`이 있으면 모든 authority store는 그 아래에 모입니다. 없으면 일반 실행에서는
사용자 홈의 `.akra`를 쓰고, 테스트에서는 임시 디렉터리를 씁니다. `projects/<repo>-<hash>/runtime`
형태를 택하는 이유는 같은 이름의 repository가 다른 절대 경로에 여러 개 있어도 충돌하지 않게
하기 위해서입니다.
*/
const AKRA_HOME_ENV: &str = "AKRA_HOME";
const AKRA_HOME_DIRECTORY: &str = ".akra";
const AKRA_PROJECTS_DIRECTORY: &str = "projects";
const RUNTIME_DIRECTORY: &str = "runtime";
const AUTHORITY_STORE_FILE_NAME: &str = "planning-authority.db";

impl SqlitePlanningAuthorityAdapter {
    /*
    학습 주석:
    주어진 workspace가 Git repository로 해석되는지 확인합니다.

    이 값은 파일시스템 workspace adapter가 "디스크의 planning 파일을 직접 볼 것인가,
    아니면 repo-scoped authority DB로 위임할 것인가"를 고를 때 쓰는 빠른 분기 조건입니다.
    `resolve_canonical_repo_root`가 `None`을 반환하면 Git 명령으로 root를 찾지 못했다는 뜻이고,
    그 경우 기존 파일 기반 workspace 흐름을 유지합니다.
    */
    pub(crate) fn is_git_backed_workspace(workspace_dir: &str) -> bool {
        resolve_canonical_repo_root(workspace_dir).is_some()
    }

    /*
    학습 주석:
    active planning 파일을 해석할 기준 root를 반환합니다.

    repo-scoped authority가 가능한 경우에는 Git canonical root를 돌려주고, 그렇지 않으면
    입력 workspace 경로를 가능한 만큼 canonicalize해서 돌려줍니다. 이 fallback이 중요한 이유는
    호출자가 Git repository 밖에서도 같은 API를 호출할 수 있기 때문입니다. 즉 이 함수는
    "repo-aware이면 repo root, 아니면 workspace root"라는 adapter 경계의 기준점을 제공합니다.
    */
    pub(crate) fn resolve_active_workspace_root(workspace_dir: &str) -> PathBuf {
        Self::resolve_authority_location_from_workspace(workspace_dir)
            .map(|location| PathBuf::from(location.canonical_repo_root))
            .unwrap_or_else(|_| canonicalize_best_effort(Path::new(workspace_dir)))
    }

    /*
    학습 주석:
    workspace 입력 하나로 authority store가 필요로 하는 모든 기준 경로를 계산합니다.

    반환되는 `PlanningAuthorityLocation`의 각 필드는 서로 다른 책임을 가집니다.
    - `workspace_root`: 사용자가 넘긴 workspace를 canonicalize한 위치입니다.
    - `canonical_repo_root`: Git repository 전체를 대표하는 root입니다. linked worktree라면
      worktree 디렉터리가 아니라 main repository root로 보정됩니다.
    - `runtime_dir`: repo별 관리 데이터가 들어가는 `.akra/projects/.../runtime` 위치입니다.
    - `authority_store_path`: 실제 SQLite DB 파일 경로입니다.

    이 계산을 한 곳에 모아 두면 나머지 DB 모듈은 "DB를 어디에 둘 것인가"를 다시 판단하지 않고
    location 값만 따라갈 수 있습니다.
    */
    pub(crate) fn resolve_authority_location_from_workspace(
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityLocation> {
        let workspace_root = canonicalize_best_effort(Path::new(workspace_dir));
        let canonical_repo_root =
            resolve_canonical_repo_root(workspace_dir).unwrap_or_else(|| workspace_root.clone());
        let runtime_dir = management_project_root(&canonical_repo_root).join(RUNTIME_DIRECTORY);
        let authority_store_path = runtime_dir.join(AUTHORITY_STORE_FILE_NAME);

        Ok(PlanningAuthorityLocation {
            workspace_root: workspace_root.display().to_string(),
            canonical_repo_root: canonical_repo_root.display().to_string(),
            runtime_dir: runtime_dir.display().to_string(),
            authority_store_path: authority_store_path.display().to_string(),
        })
    }
}

/*
학습 주석:
초안 전체를 가리키는 표시용 경로를 만듭니다.

repo-scoped draft는 실제 디렉터리 파일이 아니라 SQLite 행들의 묶음입니다. 그래도 application/TUI
쪽에서는 "어디에 staged 되었는지"를 문자열로 보여줘야 하므로, DB 파일 경로 뒤에 fragment처럼
`#drafts/<draft_name>`을 붙입니다. 이 형식은 실제 OS 경로라기보다 authority store 내부 좌표입니다.
*/
pub(super) fn draft_directory_display_path(
    location: &PlanningAuthorityLocation,
    draft_name: &str,
) -> String {
    format!("{}#drafts/{draft_name}", location.authority_store_path)
}

/*
학습 주석:
초안 안의 단일 active planning 파일을 가리키는 표시용 경로를 만듭니다.

`active_path`는 호출 위치에 따라 Windows 구분자, `./` prefix, planning workspace prefix를 포함할 수
있습니다. 이 함수는 표시 문자열이 일관되도록 `/` 기준 상대 경로로 정리합니다. 정리한 뒤에는
`<db-path>#drafts/<draft-name>/<relative-active-path>` 형태가 되어, 실제 DB 파일과 DB 안의 논리적
초안 엔트리를 한 문자열로 함께 보여줄 수 있습니다.
*/
pub(super) fn draft_display_path(
    location: &PlanningAuthorityLocation,
    draft_name: &str,
    active_path: &str,
) -> String {
    let draft_relative_path = active_path
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches(".codex-exec-loop/planning/")
        .to_string();
    format!(
        "{}#drafts/{draft_name}/{draft_relative_path}",
        location.authority_store_path
    )
}

/*
학습 주석:
canonical repository root를 repo별 관리 디렉터리로 변환합니다.

사용자에게 익숙한 repo 이름을 앞에 두고, 절대 경로에서 만든 짧은 stable hash를 뒤에 붙입니다.
repo 이름만 쓰면 `/tmp/app`과 `/work/app`처럼 이름이 같은 저장소가 충돌합니다. 절대 경로 전체를
디렉터리 이름으로 쓰면 너무 길고 OS별 path separator 문제도 커집니다. 그래서 사람이 읽을 수 있는
repo 이름과 충돌 방지 hash를 조합합니다.
*/
fn management_project_root(canonical_repo_root: &Path) -> PathBuf {
    let repo_name = canonical_repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    akra_home_root().join(AKRA_PROJECTS_DIRECTORY).join(format!(
        "{repo_name}-{}",
        stable_short_hash(&canonical_repo_root.to_string_lossy())
    ))
}

/*
학습 주석:
authority store 관리 데이터의 최상위 root를 결정합니다.

우선순위는 명시적 환경변수 `AKRA_HOME`이 가장 높습니다. 이 값은 테스트나 사용자가 별도 데이터
디렉터리를 지정하고 싶을 때 전체 저장 위치를 제어하는 스위치입니다. 테스트 빌드에서는 사용자 홈을
오염시키지 않도록 temp dir 아래를 사용하고, 일반 빌드에서는 HOME/USERPROFILE을 찾아 `.akra`를
붙입니다.
*/
fn akra_home_root() -> PathBuf {
    if let Some(path) = env::var_os(AKRA_HOME_ENV).filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }

    #[cfg(test)]
    {
        env::temp_dir().join(AKRA_HOME_DIRECTORY).join("tests")
    }

    #[cfg(not(test))]
    {
        env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(AKRA_HOME_DIRECTORY)
    }
}

/*
학습 주석:
관리 디렉터리 이름에 붙일 짧은 안정 hash를 만듭니다.

여기서는 외부 crate 없이 FNV-1a 방식의 간단한 64-bit hash를 직접 씁니다. 보안 목적 hash가 아니라
같은 repo 이름의 경로 충돌을 줄이는 용도이므로 빠르고 결정적인 값이면 충분합니다. 마지막에
16자리 hex 중 앞 12자리만 쓰는 것은 디렉터리 이름을 짧게 유지하기 위한 선택입니다.
*/
fn stable_short_hash(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..12].to_string()
}

/*
학습 주석:
workspace가 속한 canonical Git repository root를 찾고, 결과를 process-local cache에 저장합니다.

이 함수가 cache를 쓰는 이유는 planning workspace 동작 중 같은 workspace에 대해 root 해석을 여러
번 반복하기 때문입니다. Git 명령 실행은 파일 경로 계산보다 훨씬 비싸므로, canonicalized workspace
경로 문자열을 key로 삼아 한 번 계산한 root를 재사용합니다. cache 값은 `PathBuf`로 clone해서
반환하므로 caller가 마음대로 소유해도 전역 cache는 안전하게 남습니다.
*/
fn resolve_canonical_repo_root(workspace_dir: &str) -> Option<PathBuf> {
    let cache_key = canonicalize_best_effort(Path::new(workspace_dir))
        .display()
        .to_string();
    if let Some(cached_root) = canonical_repo_root_cache()
        .lock()
        .expect("canonical repo root cache mutex poisoned")
        .get(&cache_key)
        .cloned()
    {
        return Some(cached_root);
    }

    let resolved_root = resolve_canonical_repo_root_uncached(workspace_dir)?;
    canonical_repo_root_cache()
        .lock()
        .expect("canonical repo root cache mutex poisoned")
        .insert(cache_key, resolved_root.clone());
    Some(resolved_root)
}

/*
학습 주석:
지정 workspace에서 Git 명령을 실행하고 비어 있지 않은 stdout을 문자열로 반환합니다.

이 helper는 Git이 없거나, workspace가 Git repo가 아니거나, 명령이 실패하거나, stdout이 UTF-8이
아니거나, 결과가 빈 문자열인 경우를 모두 `None`으로 접습니다. 상위 함수 입장에서는 어떤 이유든
"Git root를 신뢰할 수 없다"는 하나의 상태로 처리하면 충분하기 때문입니다.
*/
fn git_stdout(workspace_dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(workspace_dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

/*
학습 주석:
cache를 거치지 않고 Git 명령만으로 canonical repository root를 계산합니다.

일반 checkout에서는 `git rev-parse --show-toplevel`이 repo root입니다. 하지만 Git linked worktree는
작업 디렉터리별 `.git`이 main repo의 `.git/worktrees/...` 아래를 가리킬 수 있습니다. 이 프로젝트의
repo-scoped authority는 같은 repository의 여러 worktree가 하나의 authority DB를 공유해야 하므로,
`--git-dir`이 `<common-dir>/worktrees/...` 아래에 있으면 `common-dir`의 parent를 canonical repo root로
돌려줍니다. 그 외의 경우에는 show-toplevel을 그대로 씁니다.
*/
fn resolve_canonical_repo_root_uncached(workspace_dir: &str) -> Option<PathBuf> {
    let show_toplevel = git_stdout(workspace_dir, &["rev-parse", "--show-toplevel"])?;
    let common_dir = git_stdout(workspace_dir, &["rev-parse", "--git-common-dir"])?;
    let git_dir = git_stdout(workspace_dir, &["rev-parse", "--git-dir"])?;
    let workspace_path = Path::new(workspace_dir);
    let canonical_toplevel =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&show_toplevel)));
    let canonical_common_dir =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&common_dir)));
    let canonical_git_dir =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&git_dir)));
    let worktrees_root = canonical_common_dir.join("worktrees");
    if canonical_git_dir.starts_with(&worktrees_root) {
        return canonical_common_dir.parent().map(Path::to_path_buf);
    }
    Some(canonical_toplevel)
}

/*
학습 주석:
canonical repo root cache의 전역 저장소입니다.

`OnceLock`은 최초 접근 때 한 번만 `Mutex<BTreeMap<...>>`를 만들고 이후에는 같은 객체를 재사용합니다.
`Mutex`를 둔 이유는 테스트나 future runtime에서 여러 thread가 동시에 workspace path를 해석해도
cache map 내부가 깨지지 않도록 하기 위해서입니다.
*/
fn canonical_repo_root_cache() -> &'static Mutex<BTreeMap<String, PathBuf>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/*
학습 주석:
Git 명령이 돌려준 경로를 workspace 기준 절대 경로로 바꿉니다.

`git rev-parse` 결과는 설정과 호출 위치에 따라 절대 경로나 상대 경로가 될 수 있습니다. 이미 절대
경로면 그대로 쓰고, 상대 경로면 명령을 실행한 workspace 디렉터리에 붙입니다. 이후 caller가
`canonicalize_best_effort`를 적용해 symlink와 `..` 등을 가능한 만큼 정리합니다.
*/
fn absolutize_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    base.join(path)
}

/*
학습 주석:
파일시스템 canonicalize를 시도하되 실패해도 원본 path를 보존합니다.

`fs::canonicalize`는 경로가 아직 존재하지 않거나 권한 문제가 있으면 실패할 수 있습니다. authority
location 계산은 "가능하면 정규화하고, 안 되면 입력 경로라도 유지"해야 이후 오류 맥락을 잃지
않습니다. 그래서 이 helper는 `Result`를 밖으로 노출하지 않고 best-effort `PathBuf`를 반환합니다.
*/
pub(super) fn canonicalize_best_effort(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
