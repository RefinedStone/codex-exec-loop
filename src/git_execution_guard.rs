use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::{git_subprocess, subprocess};

const MAX_GIT_CONFIG_STDOUT_BYTES: usize = 256 * 1024;
const MAX_GIT_CONFIG_STDERR_BYTES: usize = 64 * 1024;
use akra_verification_native::git_config::{
    EXECUTABLE_CONFIG_SECTION_REGEX, config_key_enables_external_execution,
    validate_host_git_execution_config_keys,
};

pub(crate) fn ensure_git_command_execution_config_safe(
    program: &str,
    args: &[&str],
    current_dir: Option<&str>,
) -> Result<()> {
    if !git_subprocess::is_git_program(std::ffi::OsStr::new(program))
        || !git_command_can_execute_repository_configuration(args)
    {
        return Ok(());
    }
    reject_inline_execution_config(args)?;
    let repo_context = git_repo_context(args, current_dir)
        .context("host Git command has no inspectable repository context")?;
    ensure_host_git_execution_config_safe(&repo_context)
}

fn reject_inline_execution_config(args: &[&str]) -> Result<()> {
    let mut index = 0usize;
    while index < args.len() {
        if args[index] == "-c" {
            let assignment = args
                .get(index.saturating_add(1))
                .context("host Git command contains an incomplete inline config option")?;
            let key = assignment
                .split_once('=')
                .map_or(*assignment, |(key, _)| key)
                .to_ascii_lowercase();
            if config_key_enables_external_execution(&key) {
                bail!(
                    "host Git command is blocked by inline executable configuration: {}",
                    escaped_config_key_label(&key)
                );
            }
            index = index.saturating_add(2);
            continue;
        }
        if args[index].starts_with("--config-env") {
            bail!("host Git command cannot inject configuration through the environment");
        }
        index = index.saturating_add(1);
    }
    Ok(())
}

fn git_command_can_execute_repository_configuration(args: &[&str]) -> bool {
    let Some(command_index) = git_subcommand_index(args) else {
        return false;
    };
    matches!(
        args[command_index],
        "add"
            | "am"
            | "apply"
            | "branch"
            | "checkout"
            | "checkout-index"
            | "cherry-pick"
            | "clean"
            | "commit"
            | "diff"
            | "difftool"
            | "grep"
            | "log"
            | "merge"
            | "mergetool"
            | "mv"
            | "rebase"
            | "read-tree"
            | "reset"
            | "restore"
            | "rm"
            | "show"
            | "stash"
            | "status"
            | "submodule"
            | "switch"
            | "update-index"
            | "update-ref"
            | "worktree"
    )
}

fn git_subcommand_index(args: &[&str]) -> Option<usize> {
    let mut index = 0usize;
    while index < args.len() {
        match args[index] {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                index = index.saturating_add(2);
            }
            argument if argument.starts_with('-') => {
                index = index.saturating_add(1);
            }
            _ => return Some(index),
        }
    }
    None
}

fn git_repo_context(args: &[&str], current_dir: Option<&str>) -> Option<PathBuf> {
    let mut context = current_dir.map(PathBuf::from);
    let mut index = 0usize;
    while index < args.len() {
        if args[index] == "-C" {
            let next = args.get(index.saturating_add(1))?;
            context = Some(resolve_git_context_path(context.as_deref(), next)?);
            index = index.saturating_add(2);
            continue;
        }
        if args[index] == "--git-dir" {
            let next = args.get(index.saturating_add(1))?;
            context = Some(resolve_git_context_path(context.as_deref(), next)?);
            index = index.saturating_add(2);
            continue;
        }
        if let Some(next) = args[index].strip_prefix("--git-dir=") {
            context = Some(resolve_git_context_path(context.as_deref(), next)?);
        }
        if args[index] == "--work-tree" {
            index = index.saturating_add(2);
            continue;
        }
        index = index.saturating_add(1);
    }
    context.or_else(|| std::env::current_dir().ok())
}

fn resolve_git_context_path(base: Option<&Path>, value: &str) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Some(path)
    } else if let Some(base) = base {
        Some(base.join(path))
    } else {
        Some(std::env::current_dir().ok()?.join(path))
    }
}

/*
 * Host-owned worktree mutations must not execute commands selected by repository-local Git
 * configuration. The Git subprocess boundary already removes global/system/environment config;
 * this audit reads only key names from the remaining effective local plus worktree scopes,
 * including local includes. Config values never enter Akra memory, diagnostics, or logs.
 *
 * Network routing and credential keys are deliberately outside this local-mutation guard. The
 * guarded operations do not update submodules or contact remotes, while remote writes use the
 * isolated GitHub execution context. A future host-owned network Git operation must introduce a
 * separate value-aware policy before it is added to this mutation boundary.
 */
pub(crate) fn ensure_host_git_execution_config_safe(repo_or_worktree: &Path) -> Result<()> {
    ensure_host_git_execution_config_safe_with_policy(repo_or_worktree, false)
}

pub(crate) fn ensure_verified_submodule_git_execution_config_safe(
    repo_or_worktree: &Path,
    verified_canonical_worktree: &Path,
    canonical_parent_root: &Path,
) -> Result<()> {
    let current_canonical = std::fs::canonicalize(repo_or_worktree).with_context(|| {
        format!(
            "failed to revalidate verified submodule path `{}`",
            repo_or_worktree.display()
        )
    })?;
    if current_canonical != verified_canonical_worktree
        || current_canonical == canonical_parent_root
        || !current_canonical.starts_with(canonical_parent_root)
    {
        bail!("verified submodule path identity or containment changed before Git inspection");
    }

    let effective_toplevel = effective_git_toplevel(repo_or_worktree)?;
    let effective_toplevel = std::fs::canonicalize(&effective_toplevel).with_context(|| {
        format!(
            "effective submodule core.worktree could not be canonicalized: {}",
            effective_toplevel.display()
        )
    })?;
    if effective_toplevel != current_canonical {
        bail!(
            "submodule core.worktree resolves outside its verified worktree: {}",
            effective_toplevel.display()
        );
    }

    ensure_host_git_execution_config_safe_with_policy(repo_or_worktree, true)?;
    let final_canonical = std::fs::canonicalize(repo_or_worktree)
        .context("failed to revalidate submodule path after execution-config audit")?;
    if final_canonical != current_canonical {
        bail!("verified submodule path changed during Git execution-config audit");
    }
    Ok(())
}

fn effective_git_toplevel(repo_or_worktree: &Path) -> Result<PathBuf> {
    let mut command = git_subprocess::command(std::iter::empty::<&str>());
    command
        .arg("-C")
        .arg(repo_or_worktree)
        .args(["rev-parse", "--show-toplevel"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = subprocess::wait_with_output_timeout_and_limits(
        subprocess::spawn(&mut command).context("failed to start submodule worktree audit")?,
        "submodule effective worktree audit",
        subprocess::configured_subprocess_timeout(),
        MAX_GIT_CONFIG_STDOUT_BYTES,
        MAX_GIT_CONFIG_STDERR_BYTES,
    )
    .context("submodule effective worktree audit exceeded its bounded process contract")?;
    if !output.status.success() {
        bail!(
            "submodule effective worktree audit failed with status {}",
            output.status
        );
    }
    let output = String::from_utf8(output.stdout)
        .context("submodule effective worktree audit returned non-UTF-8 output")?;
    let value = output.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        bail!("submodule effective worktree audit returned an invalid path");
    }
    Ok(PathBuf::from(value))
}

fn ensure_host_git_execution_config_safe_with_policy(
    repo_or_worktree: &Path,
    allow_verified_core_worktree: bool,
) -> Result<()> {
    let mut command = git_subprocess::command(std::iter::empty::<&str>());
    command
        .arg("-C")
        .arg(repo_or_worktree)
        .args([
            "config",
            "--includes",
            "--null",
            "--name-only",
            "--get-regexp",
            EXECUTABLE_CONFIG_SECTION_REGEX,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = subprocess::spawn(&mut command).with_context(|| {
        format!(
            "failed to start host Git execution-config audit for `{}`",
            repo_or_worktree.display()
        )
    })?;
    let output = subprocess::wait_with_output_timeout_and_limits(
        child,
        "host Git execution-config audit",
        subprocess::configured_subprocess_timeout(),
        MAX_GIT_CONFIG_STDOUT_BYTES,
        MAX_GIT_CONFIG_STDERR_BYTES,
    )
    .context("host Git execution-config audit exceeded its bounded process contract")?;
    if !output.status.success() && output.status.code() != Some(1) {
        bail!(
            "host Git execution-config audit failed with status {}",
            output.status
        );
    }

    validate_execution_config_keys_with_policy(&output.stdout, allow_verified_core_worktree)
}

fn validate_execution_config_keys_with_policy(
    payload: &[u8],
    allow_verified_core_worktree: bool,
) -> Result<()> {
    if allow_verified_core_worktree {
        akra_verification_native::git_config::validate_submodule_config_keys_after_worktree_validation(payload)
    } else {
        validate_host_git_execution_config_keys(payload)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        ensure_host_git_execution_config_safe, git_command_can_execute_repository_configuration,
        git_repo_context,
    };

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn explicit_git_directory_is_the_execution_config_audit_context() {
        assert!(git_command_can_execute_repository_configuration(&[
            "--git-dir",
            "remote.git",
            "show",
        ]));
        assert_eq!(
            git_repo_context(
                &["--git-dir", "remote.git", "show", "HEAD:file.txt"],
                Some("workspace"),
            ),
            Some(PathBuf::from("workspace").join("remote.git"))
        );
        assert_eq!(
            git_repo_context(
                &["--git-dir=remote.git", "--work-tree", "checkout", "show"],
                Some("workspace"),
            ),
            Some(PathBuf::from("workspace").join("remote.git"))
        );
    }

    #[test]
    fn effective_worktree_config_is_audited_without_reading_or_executing_the_value() {
        let repo = temp_repo("worktree-scope");
        let marker = repo.join("hostile-marker");
        run_git(&repo, ["config", "extensions.worktreeConfig", "true"]);
        run_git(
            &repo,
            [
                "config",
                "--worktree",
                "merge.hostile.driver",
                &format!("printf attacked > {}", marker.display()),
            ],
        );

        let error = ensure_host_git_execution_config_safe(&repo)
            .expect_err("worktree-scoped command config must block host mutation");
        assert!(error.to_string().contains("merge.hostile.driver"));
        assert!(
            !marker.exists(),
            "the audit must never execute a config value"
        );
        let _ = fs::remove_dir_all(repo);
    }

    fn temp_repo(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "akra-git-execution-guard-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("test repository should create");
        run_git(&path, ["init", "-q"]);
        path
    }

    fn run_git<const N: usize>(repo: &Path, args: [&str; N]) {
        let mut command = crate::git_subprocess::command(std::iter::empty::<&str>());
        command
            .arg("-C")
            .arg(repo)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let output =
            crate::subprocess::command_output(&mut command, "git execution guard test setup")
                .expect("test Git command should run");
        assert!(
            output.status.success(),
            "test Git command failed: {output:?}"
        );
    }
}
