//! Resolves the integration branch for both delivery and remote branch protection.
use std::path::Path;

pub(crate) const DEFAULT_PARALLEL_MODE_INTEGRATION_BRANCH: &str = "prerelease";

pub(crate) fn for_workspace(
    root: &Path,
    read_environment: impl FnOnce(&str) -> Result<Option<String>, String>,
    read_git_config: impl FnOnce(&str) -> Option<String>,
) -> Result<String, String> {
    if let Some(resolved) = super::current_process_config_for_workspace(root).map_err(|error| {
        format!(
            "failed to resolve configuration for target workspace `{root}`: {error:#}",
            root = root.display()
        )
    })? {
        return Ok(resolved.config.parallel.integration_branch);
    }
    let environment = read_environment("AKRA_PARALLEL_INTEGRATION_BRANCH")?;
    let configured = read_git_config("akra.parallelIntegrationBranch");
    resolve_parallel_mode_integration_branch_strict(environment.as_deref(), configured.as_deref())
}

pub(crate) fn resolve_parallel_mode_integration_branch_strict(
    env_value: Option<&str>,
    config_value: Option<&str>,
) -> Result<String, String> {
    if let Some(value) = env_value.filter(|value| !value.trim().is_empty()) {
        return normalize_parallel_mode_integration_branch(Some(value)).ok_or_else(|| {
            "AKRA_PARALLEL_INTEGRATION_BRANCH is invalid; delivery target fallback is disabled"
                .to_string()
        });
    }
    if let Some(value) = config_value.filter(|value| !value.trim().is_empty()) {
        return normalize_parallel_mode_integration_branch(Some(value)).ok_or_else(|| {
            "akra.parallelIntegrationBranch is invalid; delivery target fallback is disabled"
                .to_string()
        });
    }
    Ok(DEFAULT_PARALLEL_MODE_INTEGRATION_BRANCH.to_string())
}

pub(crate) fn normalize_parallel_mode_integration_branch(value: Option<&str>) -> Option<String> {
    let raw_value = value?;
    if raw_value.is_empty()
        || raw_value != raw_value.trim()
        || raw_value == "HEAD"
        || raw_value == "@"
        || raw_value.starts_with('-')
        || raw_value.starts_with('/')
        || raw_value.ends_with('/')
        || raw_value.ends_with('.')
        || raw_value.contains("..")
        || raw_value.contains("@{")
        || raw_value.ends_with(".lock")
        || raw_value.chars().any(|ch| {
            ch <= '\u{1f}'
                || ch == '\u{7f}'
                || ch.is_whitespace()
                || matches!(ch, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
    {
        return None;
    }
    if raw_value
        .split('/')
        .any(|segment| segment.is_empty() || segment.starts_with('.') || segment.ends_with(".lock"))
    {
        return None;
    }
    Some(raw_value.to_string())
}
