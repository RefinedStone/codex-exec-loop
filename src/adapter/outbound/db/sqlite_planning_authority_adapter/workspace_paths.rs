/*
SQLite planning authority store가 어느 경로에 있어야 하는지 결정하는 경로 해석 계층이다.

repo-scoped planning authority의 핵심은 "현재 process가 어느 worktree나 하위 디렉터리에서
실행되더라도 같은 Git repository에는 같은 authority DB를 사용한다"는 점이다. 그래서 단순히
`workspace_dir/.akra/...` 같은 경로를 쓰지 않고, 먼저 Git이 보는 canonical repository root를
찾은 뒤 그 값을 안정적인 project 관리 디렉터리 이름으로 바꾼다.

이 파일은 outbound DB adapter 내부 helper지만 application 계층의 의미와 강하게 연결된다.
`PlanningAuthorityLocation`은 이후 store, draft_files, active_documents, runtime_projection
모듈이 모두 공유하는 기준 좌표다.
*/
use std::collections::BTreeMap;
#[cfg(not(test))]
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, anyhow, bail};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::domain::planning::PlanningAuthorityLocation;
use crate::git_subprocess;

use super::SqlitePlanningAuthorityAdapter;

/*
경로 정책을 나타내는 작은 상수들이다.

일반 실행에서 `AKRA_HOME`이 있으면 모든 authority store는 그 아래에 모이고, 없으면 사용자 홈의
`.akra`를 쓴다. unit test는 병렬 테스트가 process-global 환경변수를 바꾸더라도 다른 fixture의
저장소를 채택하지 않도록 항상 임시 디렉터리의 고정 test root를 쓴다.
`projects/<repo>-<hash>/runtime` 형태를 택하는 이유는 같은 이름의 repository가 다른 절대 경로에
여러 개 있어도 충돌하지 않게 하기 위해서다.
*/
#[cfg(not(test))]
const AKRA_HOME_ENV: &str = "AKRA_HOME";
const AKRA_HOME_DIRECTORY: &str = ".akra";
const AKRA_PROJECTS_DIRECTORY: &str = "projects";
const RUNTIME_DIRECTORY: &str = "runtime";
const AUTHORITY_STORE_FILE_NAME: &str = "planning-authority.db";
pub(super) const REPOSITORY_INCARNATION_MARKER_FILE_NAME: &str = "akra-repository-id";
const REPOSITORY_INCARNATION_LOCK_FILE_NAME: &str = "akra-repository-id.lock";
const REPOSITORY_INCARNATION_MARKER_HEADER: &str = "akra-repository-incarnation-v1";
const REPOSITORY_INCARNATION_ID_HEX_LENGTH: usize = 64;
const REPOSITORY_PROJECT_NAMESPACE_MAX_LENGTH: usize = 160;
const REPOSITORY_MARKER_MAX_BYTES: u64 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepositoryIncarnation {
    id: String,
    project_namespace: String,
}

#[derive(Debug, Clone)]
struct RepositoryPathResolution {
    canonical_repo_root: PathBuf,
    canonical_common_dir: PathBuf,
    repository_incarnation: RepositoryIncarnation,
    repository_identity: String,
    project_namespace: String,
}

impl SqlitePlanningAuthorityAdapter {
    /*
    주어진 workspace가 Git repository로 해석되는지 확인한다.

    이 값은 파일시스템 workspace adapter가 "디스크의 planning 파일을 직접 볼 것인가,
    아니면 repo-scoped authority DB로 위임할 것인가"를 고를 때 쓰는 빠른 분기 조건이다.
    `resolve_repository_paths`가 `None`을 반환하면 Git 명령으로 root를 찾지 못했다는 뜻이고,
    그 경우 기존 파일 기반 workspace 흐름을 유지한다.
    */
    pub(crate) fn is_git_backed_workspace(workspace_dir: &str) -> bool {
        match resolve_repository_paths(workspace_dir) {
            Ok(Some(_)) => true,
            Ok(None) => false,
            // A broken repository marker must keep the workspace on the repo-scoped path so the
            // later fallible store operation reports the security error instead of silently
            // switching to direct filesystem authority.
            Err(_) => git_stdout(workspace_dir, &["rev-parse", "--show-toplevel"]).is_some(),
        }
    }

    /*
    active planning 파일을 해석할 기준 root를 반환한다.

    repo-scoped authority가 가능한 경우에는 Git canonical root를 돌려주고, 그렇지 않으면
    입력 workspace 경로를 가능한 만큼 canonicalize해서 돌려준다. 이 fallback이 중요한 이유는
    호출자가 Git repository 밖에서도 같은 API를 호출할 수 있기 때문이다. 즉 이 함수는
    "repo-aware이면 repo root, 아니면 workspace root"라는 adapter 경계의 기준점을 제공한다.
    */
    pub(crate) fn resolve_active_workspace_root(workspace_dir: &str) -> PathBuf {
        Self::resolve_authority_location_from_workspace(workspace_dir)
            .map(|location| PathBuf::from(location.canonical_repo_root))
            .unwrap_or_else(|_| canonicalize_best_effort(Path::new(workspace_dir)))
    }

    /*
    workspace 입력 하나로 authority store가 필요로 하는 모든 기준 경로를 계산한다.

    반환되는 `PlanningAuthorityLocation`의 각 필드는 서로 다른 책임을 가진다.
    - `workspace_root`: 사용자가 넘긴 workspace를 canonicalize한 위치다.
    - `canonical_repo_root`: branch/worktree 동작이 사용할 대표 checkout root다. 일반 linked
      worktree라면 main checkout으로 보정하고, main checkout이 없는 bare-backed repository는 현재
      checkout root를 유지한다.
    - `repository_identity`: canonical Git common-dir에 owner-private하게 저장된 incarnation이다.
      checkout 경로가 재사용되어도 authority DB namespace와 binding이 이전 repo를 채택하지 않는다.
    - `runtime_dir`: repo별 관리 데이터가 들어가는 `.akra/projects/.../runtime` 위치다.
    - `authority_store_path`: 실제 SQLite DB 파일 경로다.

    이 계산을 한 곳에 모아 두면 나머지 DB 모듈은 "DB를 어디에 둘 것인가"를 다시 판단하지 않고
    location 값만 따라갈 수 있다.
    */
    pub(crate) fn resolve_authority_location_from_workspace(
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityLocation> {
        let workspace_root = canonicalize_best_effort(Path::new(workspace_dir));
        let repository_paths = resolve_repository_paths(workspace_dir)?;
        let canonical_repo_root = repository_paths
            .as_ref()
            .map(|paths| paths.canonical_repo_root.clone())
            .unwrap_or_else(|| workspace_root.clone());
        let repository_identity = repository_paths
            .as_ref()
            .map(|paths| paths.repository_identity.clone())
            .unwrap_or_else(|| workspace_root.display().to_string());
        let project_root = if let Some(paths) = repository_paths.as_ref() {
            akra_home_root()?
                .join(AKRA_PROJECTS_DIRECTORY)
                .join(&paths.project_namespace)
        } else {
            management_project_root(&workspace_root, true)?
        };
        let runtime_dir = project_root.join(RUNTIME_DIRECTORY);
        let authority_store_path = runtime_dir.join(AUTHORITY_STORE_FILE_NAME);

        Ok(PlanningAuthorityLocation {
            workspace_root: workspace_root.display().to_string(),
            canonical_repo_root: canonical_repo_root.display().to_string(),
            repository_identity,
            runtime_dir: runtime_dir.display().to_string(),
            authority_store_path: authority_store_path.display().to_string(),
        })
    }
}

/*
초안 전체를 가리키는 표시용 경로를 만든다.

repo-scoped draft는 실제 디렉터리 파일이 아니라 SQLite 행들의 묶음이다. 그래도 application/TUI
쪽에서는 "어디에 staged 되었는지"를 문자열로 보여줘야 하므로, DB 파일 경로 뒤에 fragment처럼
`#drafts/<draft_name>`을 붙인다. 이 형식은 실제 OS 경로라기보다 authority store 내부 좌표다.
*/
pub(super) fn draft_directory_display_path(
    location: &PlanningAuthorityLocation,
    draft_name: &str,
) -> String {
    format!("{}#drafts/{draft_name}", location.authority_store_path)
}

/*
초안 안의 단일 active planning 파일을 가리키는 표시용 경로를 만든다.

`active_path`는 호출 위치에 따라 Windows 구분자, `./` prefix, planning workspace prefix를 포함할 수
있다. 이 함수는 표시 문자열이 일관되도록 `/` 기준 상대 경로로 정리한다. 정리한 뒤에는
`<db-path>#drafts/<draft-name>/<relative-active-path>` 형태가 되어, 실제 DB 파일과 DB 안의 논리적
초안 엔트리를 한 문자열로 함께 보여줄 수 있다.
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
canonical repository root를 repo별 관리 디렉터리로 변환한다.

사용자에게 익숙한 repo 이름을 앞에 두고, 절대 경로에서 만든 짧은 stable hash를 뒤에 붙인다.
repo 이름만 쓰면 `/tmp/app`과 `/work/app`처럼 이름이 같은 저장소가 충돌한다. 절대 경로 전체를
디렉터리 이름으로 쓰면 너무 길고 OS별 path separator 문제도 커진다. 그래서 사람이 읽을 수 있는
repo 이름과 충돌 방지 hash를 조합한다.
*/
fn management_project_root(
    storage_namespace_key: &Path,
    uses_legacy_namespace_hash: bool,
) -> Result<PathBuf> {
    let repo_name = storage_namespace_key
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.strip_suffix(".git").unwrap_or(name))
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    let namespace_hash = if uses_legacy_namespace_hash {
        legacy_short_hash(&storage_namespace_key.to_string_lossy())
    } else {
        repository_identity_hash(&storage_namespace_key.to_string_lossy())
    };
    Ok(akra_home_root()?
        .join(AKRA_PROJECTS_DIRECTORY)
        .join(format!("{repo_name}-{namespace_hash}")))
}

fn repository_namespace_base(
    storage_namespace_key: &Path,
    uses_legacy_namespace_hash: bool,
) -> String {
    let repo_name = storage_namespace_key
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.strip_suffix(".git").unwrap_or(name))
        .map(sanitize_repository_label)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "repository".to_string());
    let namespace_hash = if uses_legacy_namespace_hash {
        legacy_short_hash(&storage_namespace_key.to_string_lossy())
    } else {
        repository_identity_hash(&storage_namespace_key.to_string_lossy())
    };
    format!("{repo_name}-{namespace_hash}")
}

fn sanitize_repository_label(value: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_was_separator = false;
    for character in value.chars() {
        let normalized = if character.is_ascii_alphanumeric() || matches!(character, '.' | '_') {
            character.to_ascii_lowercase()
        } else {
            '-'
        };
        if normalized == '-' {
            if previous_was_separator || sanitized.is_empty() {
                continue;
            }
            previous_was_separator = true;
        } else {
            previous_was_separator = false;
        }
        sanitized.push(normalized);
        if sanitized.len() >= 48 {
            break;
        }
    }
    sanitized.trim_end_matches('-').to_string()
}

fn select_repository_project_namespace(
    namespace_base: &str,
    incarnation_id: &str,
) -> Result<String> {
    validate_repository_project_namespace(namespace_base)?;
    let legacy_project_root = akra_home_root()?
        .join(AKRA_PROJECTS_DIRECTORY)
        .join(namespace_base);
    match fs::symlink_metadata(&legacy_project_root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // An unoccupied historical namespace is safe to retain exactly. This preserves the
            // existing human-readable normal-repository layout without ever adopting old data.
            Ok(namespace_base.to_string())
        }
        Ok(_) => {
            // An occupied path is ambiguous: it can be the current repository's pre-marker DB or
            // a stale DB left by a deleted checkout at the same path. There is no Git-native proof
            // in that legacy DB that distinguishes those cases, so isolate the new incarnation and
            // leave the old namespace untouched for explicit recovery.
            Ok(format!(
                "{namespace_base}-{}",
                repository_identity_hash(incarnation_id)
            ))
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect legacy repository namespace {}",
                legacy_project_root.display()
            )
        }),
    }
}

/*
authority store 관리 데이터의 최상위 root를 결정한다.

일반 빌드에서는 명시적 환경변수 `AKRA_HOME`의 우선순위가 가장 높고, 없으면
HOME/USERPROFILE을 찾아 `.akra`를 붙인다. unit-test 빌드는 사용자 홈을 오염시키지 않고 병렬
configuration fixture의 process-global `AKRA_HOME` 변경에도 휘말리지 않도록 그 환경변수를 읽기
전에 temp dir 아래의 고정 test root를 반환한다.
*/
#[cfg(test)]
pub(super) fn akra_home_root() -> Result<PathBuf> {
    /*
     * macOS는 temp_dir를 /private/var 같은 물리 경로의 symlink로 제공한다.
     * authority store는 SQLITE_OPEN_NOFOLLOW로 경로 전체의 symlink 구성요소를
     * 거부하므로, 테스트 루트도 물리 경로로 정규화해야 연결이 가능하다.
     */
    Ok(crate::test_utils::platform_safe_temp_dir()
        .join(AKRA_HOME_DIRECTORY)
        .join("tests"))
}

#[cfg(not(test))]
pub(super) fn akra_home_root() -> Result<PathBuf> {
    if let Some(path) = env::var_os(AKRA_HOME_ENV) {
        return validate_absolute_akra_home(PathBuf::from(path));
    }

    #[cfg(unix)]
    {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is required when AKRA_HOME is not configured")?;
        Ok(validate_absolute_profile_home(home)?.join(AKRA_HOME_DIRECTORY))
    }

    #[cfg(windows)]
    {
        if let Some(home) = env::var_os("USERPROFILE").map(PathBuf::from) {
            return Ok(validate_absolute_profile_home(home)?.join(AKRA_HOME_DIRECTORY));
        }
        Ok(crate::private_fs::windows_local_app_data_path()?.join("Akra"))
    }

    #[cfg(not(any(unix, windows)))]
    {
        bail!("AKRA_HOME is required on this platform")
    }
}

#[cfg(test)]
#[test]
fn unit_test_authority_home_is_process_isolated() {
    assert_eq!(
        akra_home_root().expect("unit-test authority home should resolve"),
        crate::test_utils::platform_safe_temp_dir()
            .join(AKRA_HOME_DIRECTORY)
            .join("tests")
    );
}

fn validate_absolute_akra_home(path: PathBuf) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("AKRA_HOME must not be empty");
    }
    validate_absolute_profile_home(path).context("AKRA_HOME must be an absolute normalized path")
}

fn validate_absolute_profile_home(path: PathBuf) -> Result<PathBuf> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        bail!("storage root must be an absolute normalized path");
    }
    Ok(path)
}

/*
관리 디렉터리 이름에 붙일 짧은 안정 hash를 만든다.

여기서는 외부 crate 없이 FNV-1a 방식의 간단한 64-bit hash를 직접 쓴다. 보안 목적 hash가 아니라
같은 repo 이름의 경로 충돌을 줄이는 용도이므로 빠르고 결정적인 값이면 충분하다. 마지막에
16자리 hex 중 앞 12자리만 쓰는 것은 디렉터리 이름을 짧게 유지하기 위한 선택이다.
*/
fn legacy_short_hash(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..12].to_string()
}

fn repository_identity_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/*
workspace가 속한 canonical Git repository root와 repository incarnation을 찾는다.

같은 경로의 checkout이 삭제된 뒤 clone/init되면 path와 Git common-dir 문자열은 같아도 전혀 다른
authority owner다. 따라서 cache hit도 현재 workspace가 가리키는 common-dir을 Git에 다시 묻고 그 안의
private marker를 재검증한다. 검증 비용이 작은 cache만 유지하고, Git/pathname 결과 자체를 무기한
신뢰하지 않는다.
*/
fn resolve_repository_paths(workspace_dir: &str) -> Result<Option<RepositoryPathResolution>> {
    let cache_key = canonicalize_best_effort(Path::new(workspace_dir))
        .display()
        .to_string();
    let cached = repository_paths_cache()
        .lock()
        .map_err(|_| anyhow!("repository path cache mutex is poisoned"))?
        .get(&cache_key)
        .cloned();
    if let Some(cached) = cached {
        match cached_repository_paths_are_current(workspace_dir, &cached) {
            Ok(true) => return Ok(Some(cached)),
            Ok(false) => {}
            Err(error) => {
                repository_paths_cache()
                    .lock()
                    .map_err(|_| anyhow!("repository path cache mutex is poisoned"))?
                    .remove(&cache_key);
                return Err(error);
            }
        }
        repository_paths_cache()
            .lock()
            .map_err(|_| anyhow!("repository path cache mutex is poisoned"))?
            .remove(&cache_key);
    }

    let Some(resolved) = resolve_repository_paths_uncached(workspace_dir)? else {
        return Ok(None);
    };
    let mut cache = repository_paths_cache()
        .lock()
        .map_err(|_| anyhow!("repository path cache mutex is poisoned"))?;
    if cache.len() >= 256
        && let Some(eviction_key) = cache.keys().next().cloned()
    {
        cache.remove(&eviction_key);
    }
    cache.insert(cache_key, resolved.clone());
    Ok(Some(resolved))
}

fn cached_repository_paths_are_current(
    workspace_dir: &str,
    cached: &RepositoryPathResolution,
) -> Result<bool> {
    if !cached.canonical_repo_root.is_dir() {
        return Ok(false);
    }
    let Some(common_dir) = git_stdout(workspace_dir, &["rev-parse", "--git-common-dir"]) else {
        return Ok(false);
    };
    let workspace_path = Path::new(workspace_dir);
    let canonical_common_dir =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&common_dir)));
    if canonical_common_dir != cached.canonical_common_dir {
        return Ok(false);
    }
    validate_repository_metadata_directory(&canonical_common_dir)?;
    let Some(incarnation) = read_repository_incarnation_marker(&canonical_common_dir)? else {
        return Ok(false);
    };
    Ok(incarnation == cached.repository_incarnation)
}

fn repository_paths_cache() -> &'static Mutex<BTreeMap<String, RepositoryPathResolution>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, RepositoryPathResolution>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/*
지정 workspace에서 Git 명령을 실행하고 비어 있지 않은 stdout을 문자열로 반환한다.

이 helper는 Git이 없거나, workspace가 Git repo가 아니거나, 명령이 실패하거나, stdout이 UTF-8이
아니거나, 결과가 빈 문자열인 경우를 모두 `None`으로 접는다. 상위 함수 입장에서는 어떤 이유든
"Git root를 신뢰할 수 없다"는 하나의 상태로 처리하면 충분하기 때문이다.
*/
fn git_stdout(workspace_dir: &str, args: &[&str]) -> Option<String> {
    let mut command = git_subprocess::command(args.iter().copied());
    command.current_dir(workspace_dir);
    let output =
        crate::subprocess::command_output(&mut command, "git repository root lookup").ok()?;
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
cache를 거치지 않고 Git 명령만으로 canonical repository root를 계산한다.

일반 checkout에서는 `git rev-parse --show-toplevel`이 repo root다. 하지만 Git linked worktree는
작업 디렉터리별 `.git`이 main repo의 `.git/worktrees/...` 아래를 가리킬 수 있다. 이 프로젝트의
repo-scoped authority는 같은 repository의 여러 worktree가 하나의 authority DB를 공유해야 하므로,
`--git-dir`이 `<common-dir>/worktrees/...` 아래에 있으면 `common-dir`의 parent를 canonical repo root로
돌려준다. 그 외의 경우에는 show-toplevel을 그대로 쓴다.
*/
fn resolve_repository_paths_uncached(
    workspace_dir: &str,
) -> Result<Option<RepositoryPathResolution>> {
    let Some(show_toplevel) = git_stdout(workspace_dir, &["rev-parse", "--show-toplevel"]) else {
        return Ok(None);
    };
    let common_dir = git_stdout(workspace_dir, &["rev-parse", "--git-common-dir"])
        .ok_or_else(|| anyhow!("Git repository common directory could not be resolved"))?;
    let git_dir = git_stdout(workspace_dir, &["rev-parse", "--git-dir"])
        .ok_or_else(|| anyhow!("Git repository metadata directory could not be resolved"))?;
    let workspace_path = Path::new(workspace_dir);
    let canonical_toplevel =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&show_toplevel)));
    let canonical_common_dir =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&common_dir)));
    let canonical_git_dir =
        canonicalize_best_effort(&absolutize_path(workspace_path, Path::new(&git_dir)));
    let primary_checkout_root = resolve_primary_checkout_root(workspace_dir, &canonical_common_dir);
    let worktrees_root = canonical_common_dir.join("worktrees");
    let canonical_repo_root = if canonical_git_dir.starts_with(&worktrees_root) {
        primary_checkout_root
            .clone()
            .unwrap_or_else(|| canonical_toplevel.clone())
    } else {
        canonical_toplevel
    };
    let (storage_namespace_key, uses_legacy_namespace_hash) = primary_checkout_root
        .map(|root| (root, true))
        .unwrap_or_else(|| (canonical_common_dir.clone(), false));
    let namespace_base =
        repository_namespace_base(&storage_namespace_key, uses_legacy_namespace_hash);
    let repository_incarnation =
        load_or_create_repository_incarnation(&canonical_common_dir, &namespace_base)?;
    let repository_identity = format!(
        "{REPOSITORY_INCARNATION_MARKER_HEADER}:{}",
        repository_incarnation.id
    );
    let project_namespace = repository_incarnation.project_namespace.clone();

    Ok(Some(RepositoryPathResolution {
        canonical_repo_root,
        canonical_common_dir,
        repository_incarnation,
        repository_identity,
        project_namespace,
    }))
}

fn resolve_primary_checkout_root(
    workspace_dir: &str,
    expected_common_dir: &Path,
) -> Option<PathBuf> {
    let porcelain = git_stdout(workspace_dir, &["worktree", "list", "--porcelain", "-z"])?;
    let mut fields = porcelain.split('\0');
    let primary_field = fields.next()?.strip_prefix("worktree ")?;
    if fields
        .by_ref()
        .take_while(|field| !field.is_empty())
        .any(|field| field == "bare")
    {
        return None;
    }

    let primary_root = canonicalize_best_effort(Path::new(primary_field));
    if !primary_root.join(".git").exists() {
        return None;
    }
    let primary_root_text = primary_root.to_str()?;
    let primary_toplevel = git_stdout(primary_root_text, &["rev-parse", "--show-toplevel"])?;
    let canonical_primary_toplevel = canonicalize_best_effort(&absolutize_path(
        &primary_root,
        Path::new(&primary_toplevel),
    ));
    if canonical_primary_toplevel != primary_root {
        return None;
    }
    let primary_common_dir = git_stdout(primary_root_text, &["rev-parse", "--git-common-dir"])?;
    let canonical_primary_common_dir = canonicalize_best_effort(&absolutize_path(
        &primary_root,
        Path::new(&primary_common_dir),
    ));
    (canonical_primary_common_dir == expected_common_dir).then_some(primary_root)
}

fn load_or_create_repository_incarnation(
    common_dir: &Path,
    namespace_base: &str,
) -> Result<RepositoryIncarnation> {
    validate_repository_metadata_directory(common_dir)?;
    if let Some(incarnation) = read_repository_incarnation_marker(common_dir)? {
        validate_repository_metadata_directory(common_dir)?;
        return Ok(incarnation);
    }

    let id = generate_repository_incarnation_id()?;
    let incarnation = RepositoryIncarnation {
        project_namespace: select_repository_project_namespace(namespace_base, &id)?,
        id,
    };
    let marker_path = common_dir.join(REPOSITORY_INCARNATION_MARKER_FILE_NAME);
    let lock_path = common_dir.join(REPOSITORY_INCARNATION_LOCK_FILE_NAME);
    let body = render_repository_incarnation_marker(&incarnation);

    for _ in 0..50 {
        match create_private_repository_marker_lock(&lock_path) {
            Ok(mut lock) => {
                // Another process can finish the marker between our initial read and this lock
                // acquisition. The fixed lock serializes creators, so rechecking here prevents a
                // later creator from replacing the winner with its independently generated id.
                if let Some(installed) = read_repository_incarnation_marker(common_dir)? {
                    drop(lock);
                    fs::remove_file(&lock_path).with_context(|| {
                        format!(
                            "failed to remove redundant repository marker lock {}",
                            lock_path.display()
                        )
                    })?;
                    validate_repository_metadata_directory(common_dir)?;
                    return Ok(installed);
                }
                lock.write_all(body.as_bytes()).with_context(|| {
                    format!(
                        "failed to write repository incarnation marker lock {}",
                        lock_path.display()
                    )
                })?;
                lock.sync_all().with_context(|| {
                    format!(
                        "failed to sync repository incarnation marker lock {}",
                        lock_path.display()
                    )
                })?;
                validate_open_repository_marker_file(&lock_path, &lock)?;
                drop(lock);
                match install_repository_marker_no_replace(&lock_path, &marker_path) {
                    Ok(()) => {
                        sync_repository_metadata_directory(common_dir)?;
                        let installed = read_repository_incarnation_marker(common_dir)?
                            .ok_or_else(|| {
                                anyhow!(
                                    "repository incarnation marker disappeared after installation"
                                )
                            })?;
                        if installed != incarnation {
                            bail!("repository incarnation marker changed during installation");
                        }
                        validate_repository_metadata_directory(common_dir)?;
                        return Ok(installed);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let _ = fs::remove_file(&lock_path);
                        if let Some(installed) = read_repository_incarnation_marker(common_dir)? {
                            validate_repository_metadata_directory(common_dir)?;
                            return Ok(installed);
                        }
                        return Err(error).context(
                            "repository incarnation marker destination was replaced unexpectedly",
                        );
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!(
                                "failed to install repository incarnation marker {}",
                                marker_path.display()
                            )
                        });
                    }
                }
            }
            Err(error) if is_already_exists_error(&error) => {
                if let Some(installed) = read_repository_incarnation_marker(common_dir)? {
                    validate_repository_metadata_directory(common_dir)?;
                    return Ok(installed);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }

    // A creator may have crashed after syncing a complete private lock but before rename. Adopt
    // only a fully valid, owner-private lock; an empty or malformed lock remains a fail-closed
    // operator-visible condition instead of being deleted while another process could own it.
    let recovered = read_repository_incarnation_file(&lock_path).with_context(|| {
        format!(
            "repository incarnation marker lock remained incomplete or unsafe: {}",
            lock_path.display()
        )
    })?;
    match install_repository_marker_no_replace(&lock_path, &marker_path) {
        Ok(()) => {
            sync_repository_metadata_directory(common_dir)?;
            let installed = read_repository_incarnation_marker(common_dir)?.ok_or_else(|| {
                anyhow!("recovered repository incarnation marker disappeared after installation")
            })?;
            if installed != recovered {
                bail!("recovered repository incarnation marker changed during installation");
            }
            validate_repository_metadata_directory(common_dir)?;
            Ok(installed)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            read_repository_incarnation_marker(common_dir)?.ok_or_else(|| {
                anyhow!(
                    "repository incarnation marker appeared but could not be validated: {error}"
                )
            })
        }
        Err(error) => Err(error).context("failed to recover repository incarnation marker lock"),
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn install_repository_marker_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "repository marker source path contains NUL",
        )
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "repository marker destination path contains NUL",
        )
    })?;
    // SAFETY: both C strings are NUL-terminated for this call. RENAME_NOREPLACE is the kernel
    // primitive that makes destination non-existence part of the atomic rename operation.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_vendor = "apple")]
fn install_repository_marker_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "repository marker source path contains NUL",
        )
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "repository marker destination path contains NUL",
        )
    })?;
    // SAFETY: both C strings remain valid for the call and RENAME_EXCL forbids replacement.
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn install_repository_marker_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::MoveFileW;

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both vectors are NUL-terminated and live through the call. MoveFileW, unlike
    // MoveFileExW with MOVEFILE_REPLACE_EXISTING, fails when the destination already exists.
    if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))
))]
fn install_repository_marker_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    // link(2) is an atomic create-if-absent primitive. Removing the source after a successful link
    // leaves the destination as the sole link; a failed unlink remains fail-closed because marker
    // validation rejects multiply-linked files.
    fs::hard_link(source, destination)?;
    fs::remove_file(source)
}

#[cfg(not(any(unix, windows)))]
fn install_repository_marker_no_replace(
    _source: &Path,
    _destination: &Path,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace marker installation is unsupported on this platform",
    ))
}

fn is_already_exists_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists)
}

fn generate_repository_incarnation_id() -> Result<String> {
    let mut random = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut random)
        .context("operating-system randomness is required for repository incarnation identity")?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn render_repository_incarnation_marker(incarnation: &RepositoryIncarnation) -> String {
    format!(
        "{REPOSITORY_INCARNATION_MARKER_HEADER}\nid={}\nproject={}\n",
        incarnation.id, incarnation.project_namespace
    )
}

fn read_repository_incarnation_marker(common_dir: &Path) -> Result<Option<RepositoryIncarnation>> {
    let marker_path = common_dir.join(REPOSITORY_INCARNATION_MARKER_FILE_NAME);
    match read_repository_incarnation_file(&marker_path) {
        Ok(incarnation) => Ok(Some(incarnation)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn read_repository_incarnation_file(path: &Path) -> Result<RepositoryIncarnation> {
    let mut file = open_private_repository_marker_file(path)?;
    let mut body = Vec::new();
    Read::by_ref(&mut file)
        .take(REPOSITORY_MARKER_MAX_BYTES + 1)
        .read_to_end(&mut body)
        .with_context(|| format!("failed to read repository marker {}", path.display()))?;
    if body.len() as u64 > REPOSITORY_MARKER_MAX_BYTES {
        bail!(
            "repository incarnation marker is oversized: {}",
            path.display()
        );
    }
    validate_open_repository_marker_file(path, &file)?;
    let body = std::str::from_utf8(&body)
        .with_context(|| format!("repository marker is not UTF-8: {}", path.display()))?;
    parse_repository_incarnation_marker(body)
        .with_context(|| format!("invalid repository incarnation marker: {}", path.display()))
}

fn parse_repository_incarnation_marker(body: &str) -> Result<RepositoryIncarnation> {
    if body.contains('\r') || !body.ends_with('\n') {
        bail!("marker must use canonical LF-terminated text");
    }
    let mut lines = body.lines();
    if lines.next() != Some(REPOSITORY_INCARNATION_MARKER_HEADER) {
        bail!("marker header is unsupported");
    }
    let id = lines
        .next()
        .and_then(|line| line.strip_prefix("id="))
        .ok_or_else(|| anyhow!("marker id is missing"))?;
    if id.len() != REPOSITORY_INCARNATION_ID_HEX_LENGTH
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("marker id must be 256-bit lowercase hexadecimal");
    }
    let project_namespace = lines
        .next()
        .and_then(|line| line.strip_prefix("project="))
        .ok_or_else(|| anyhow!("marker project namespace is missing"))?;
    validate_repository_project_namespace(project_namespace)?;
    if lines.next().is_some() {
        bail!("marker contains unexpected fields");
    }
    Ok(RepositoryIncarnation {
        id: id.to_string(),
        project_namespace: project_namespace.to_string(),
    })
}

fn validate_repository_project_namespace(project_namespace: &str) -> Result<()> {
    if project_namespace.is_empty()
        || project_namespace.len() > REPOSITORY_PROJECT_NAMESPACE_MAX_LENGTH
        || project_namespace == "."
        || project_namespace == ".."
        || !project_namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        bail!("repository marker project is not a safe project directory name");
    }
    Ok(())
}

#[cfg(unix)]
fn validate_repository_metadata_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| {
            format!(
                "failed to securely open Git common directory {}",
                path.display()
            )
        })?;
    let opened = directory
        .metadata()
        .with_context(|| format!("failed to inspect Git common directory {}", path.display()))?;
    let current = fs::symlink_metadata(path)
        .with_context(|| format!("failed to verify Git common directory {}", path.display()))?;
    if !opened.is_dir()
        || current.file_type().is_symlink()
        || !current.is_dir()
        || opened.uid() != unsafe { libc::geteuid() }
        || current.uid() != opened.uid()
        || opened.mode() & 0o022 != 0
        || opened.dev() != current.dev()
        || opened.ino() != current.ino()
    {
        bail!(
            "Git common directory must be a stable current-user-owned directory not writable by other users: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(windows)]
fn validate_repository_metadata_directory(path: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_path_identity,
    };

    let directory = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .with_context(|| {
            format!(
                "failed to securely open Git common directory {}",
                path.display()
            )
        })?;
    validate_windows_path_identity(path, &directory, true)
}

#[cfg(not(any(unix, windows)))]
fn validate_repository_metadata_directory(path: &Path) -> Result<()> {
    bail!(
        "secure Git repository incarnation markers are unsupported on this platform: {}",
        path.display()
    )
}

#[cfg(unix)]
fn create_private_repository_marker_lock(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    validate_open_repository_marker_file(path, &file)?;
    Ok(file)
}

#[cfg(windows)]
fn create_private_repository_marker_lock(path: &Path) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
        WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC, set_windows_private_acl,
    };

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(
            WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC,
        )
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    set_windows_private_acl(&file, false)?;
    validate_open_repository_marker_file(path, &file)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn create_private_repository_marker_lock(path: &Path) -> Result<File> {
    bail!(
        "secure repository marker creation is unsupported on this platform: {}",
        path.display()
    )
}

#[cfg(unix)]
fn open_private_repository_marker_file(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    validate_open_repository_marker_file(path, &file)?;
    Ok(file)
}

#[cfg(windows)]
fn open_private_repository_marker_file(path: &Path) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
        WINDOWS_READ_CONTROL,
    };

    let file = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    validate_open_repository_marker_file(path, &file)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_private_repository_marker_file(path: &Path) -> Result<File> {
    bail!(
        "secure repository marker reads are unsupported on this platform: {}",
        path.display()
    )
}

#[cfg(unix)]
fn validate_open_repository_marker_file(path: &Path, file: &File) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let opened = file
        .metadata()
        .with_context(|| format!("failed to inspect repository marker {}", path.display()))?;
    let current = fs::symlink_metadata(path)
        .with_context(|| format!("failed to verify repository marker {}", path.display()))?;
    if !opened.is_file()
        || current.file_type().is_symlink()
        || !current.is_file()
        || opened.uid() != unsafe { libc::geteuid() }
        || current.uid() != opened.uid()
        || opened.nlink() != 1
        || current.nlink() != 1
        || opened.permissions().mode() & 0o777 != 0o600
        || current.permissions().mode() & 0o777 != 0o600
        || opened.dev() != current.dev()
        || opened.ino() != current.ino()
    {
        bail!(
            "repository marker must be a stable current-user-owned private regular file with one link: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(windows)]
fn validate_open_repository_marker_file(path: &Path, file: &File) -> Result<()> {
    use crate::private_fs::{
        validate_windows_path_identity, validate_windows_private_owner_and_acl,
    };

    validate_windows_path_identity(path, file, false)?;
    validate_windows_private_owner_and_acl(path, file)
}

#[cfg(not(any(unix, windows)))]
fn validate_open_repository_marker_file(path: &Path, _file: &File) -> Result<()> {
    bail!(
        "secure repository marker validation is unsupported on this platform: {}",
        path.display()
    )
}

#[cfg(unix)]
fn sync_repository_metadata_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("failed to open Git common directory {}", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync Git common directory {}", path.display()))
}

#[cfg(not(unix))]
fn sync_repository_metadata_directory(_path: &Path) -> Result<()> {
    Ok(())
}

/*
Git 명령이 돌려준 경로를 workspace 기준 절대 경로로 바꾼다.

`git rev-parse` 결과는 설정과 호출 위치에 따라 절대 경로나 상대 경로가 될 수 있다. 이미 절대
경로면 그대로 쓰고, 상대 경로면 명령을 실행한 workspace 디렉터리에 붙인다. 이후 caller가
`canonicalize_best_effort`를 적용해 symlink와 `..` 등을 가능한 만큼 정리한다.
*/
fn absolutize_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    base.join(path)
}

/*
파일시스템 canonicalize를 시도하되 실패해도 원본 path를 보존한다.

`fs::canonicalize`는 경로가 아직 존재하지 않거나 권한 문제가 있으면 실패할 수 있다. authority
location 계산은 "가능하면 정규화하고, 안 되면 입력 경로라도 유지"해야 이후 오류 맥락을 잃지
않는다. 그래서 이 helper는 `Result`를 밖으로 노출하지 않고 best-effort `PathBuf`를 반환한다.
*/
pub(super) fn canonicalize_best_effort(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{
        REPOSITORY_INCARNATION_MARKER_HEADER, parse_repository_incarnation_marker,
        render_repository_incarnation_marker, validate_absolute_akra_home,
    };

    #[cfg(any(unix, windows))]
    use super::install_repository_marker_no_replace;

    #[test]
    fn explicit_akra_home_rejects_cwd_and_parent_traversal() {
        assert!(validate_absolute_akra_home("".into()).is_err());
        assert!(validate_absolute_akra_home(".".into()).is_err());
        assert!(validate_absolute_akra_home("repo/.akra".into()).is_err());

        #[cfg(unix)]
        assert!(validate_absolute_akra_home("/tmp/repo/../.akra".into()).is_err());
        #[cfg(windows)]
        assert!(validate_absolute_akra_home(r"C:\repo\..\.akra".into()).is_err());
    }

    #[test]
    fn repository_incarnation_marker_parser_accepts_only_canonical_safe_fields() {
        let canonical = format!(
            "{REPOSITORY_INCARNATION_MARKER_HEADER}\nid={}\nproject=repo-0123456789ab\n",
            "01".repeat(32)
        );
        let parsed = parse_repository_incarnation_marker(&canonical)
            .expect("canonical repository marker should parse");
        assert_eq!(render_repository_incarnation_marker(&parsed), canonical);
        assert_eq!(parsed.project_namespace, "repo-0123456789ab");

        for invalid in [
            canonical.trim_end().to_string(),
            canonical.replace("\n", "\r\n"),
            canonical.replace("id=01", "id=GG"),
            canonical.replace("project=repo-0123456789ab", "project=../repo"),
            format!("{canonical}extra=value\n"),
        ] {
            assert!(
                parse_repository_incarnation_marker(&invalid).is_err(),
                "non-canonical marker must be rejected"
            );
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn repository_marker_install_is_atomic_and_never_replaces_a_winner() {
        let root = std::env::temp_dir().join(format!(
            "akra-repository-marker-install-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock should follow the Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir(&root).expect("marker install fixture should create");

        let existing = root.join("existing-marker");
        let attempted = root.join("attempted-lock");
        std::fs::write(&existing, b"winner").expect("existing marker should write");
        std::fs::write(&attempted, b"replacement").expect("attempted marker should write");
        let error = install_repository_marker_no_replace(&attempted, &existing)
            .expect_err("an existing marker must never be replaced");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(&existing).expect("existing marker should remain readable"),
            b"winner"
        );
        assert_eq!(
            std::fs::read(&attempted).expect("losing source should remain readable"),
            b"replacement"
        );

        let destination = std::sync::Arc::new(root.join("race-marker"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = [b"candidate-a".as_slice(), b"candidate-b".as_slice()]
            .into_iter()
            .enumerate()
            .map(|(index, body)| {
                let source = root.join(format!("race-lock-{index}"));
                std::fs::write(&source, body).expect("race source should write");
                let destination = std::sync::Arc::clone(&destination);
                let barrier = std::sync::Arc::clone(&barrier);
                let body = body.to_vec();
                std::thread::spawn(move || {
                    barrier.wait();
                    let result = install_repository_marker_no_replace(&source, &destination);
                    (body, result)
                })
            })
            .collect::<Vec<_>>();
        let outcomes = handles
            .into_iter()
            .map(|handle| handle.join().expect("marker race thread should join"))
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes.iter().filter(|(_, result)| result.is_ok()).count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter_map(|(body, result)| result.is_ok().then_some(body.as_slice()))
                .next()
                .expect("race should have one winner"),
            std::fs::read(destination.as_ref())
                .expect("race destination should exist")
                .as_slice()
        );
        for (_, result) in &outcomes {
            if let Err(error) = result {
                assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
            }
        }

        std::fs::remove_dir_all(&root).expect("marker install fixture should clean up");
    }
}
