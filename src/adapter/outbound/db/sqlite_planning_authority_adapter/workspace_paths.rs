use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use anyhow::Result;

use crate::domain::planning::PlanningAuthorityLocation;

use super::SqlitePlanningAuthorityAdapter;

const AKRA_HOME_ENV: &str = "AKRA_HOME";
const AKRA_HOME_DIRECTORY: &str = ".akra";
const AKRA_PROJECTS_DIRECTORY: &str = "projects";
const RUNTIME_DIRECTORY: &str = "runtime";
const AUTHORITY_STORE_FILE_NAME: &str = "planning-authority.db";

impl SqlitePlanningAuthorityAdapter {
    pub(crate) fn is_git_backed_workspace(workspace_dir: &str) -> bool {
        resolve_canonical_repo_root(workspace_dir).is_some()
    }

    pub(crate) fn resolve_active_workspace_root(workspace_dir: &str) -> PathBuf {
        Self::resolve_authority_location_from_workspace(workspace_dir)
            .map(|location| PathBuf::from(location.canonical_repo_root))
            .unwrap_or_else(|_| canonicalize_best_effort(Path::new(workspace_dir)))
    }

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

pub(super) fn draft_directory_display_path(
    location: &PlanningAuthorityLocation,
    draft_name: &str,
) -> String {
    format!("{}#drafts/{draft_name}", location.authority_store_path)
}

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

fn canonical_repo_root_cache() -> &'static Mutex<BTreeMap<String, PathBuf>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn absolutize_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    base.join(path)
}

pub(super) fn canonicalize_best_effort(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
