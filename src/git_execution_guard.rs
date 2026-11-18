use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::{git_subprocess, subprocess};

const MAX_GIT_CONFIG_STDOUT_BYTES: usize = 256 * 1024;
const MAX_GIT_CONFIG_STDERR_BYTES: usize = 64 * 1024;
const MAX_REPORTED_EXECUTION_KEYS: usize = 64;
const MAX_GIT_CONFIG_KEY_BYTES: usize = 4 * 1024;
const MAX_REPORTED_CONFIG_KEY_BYTES: usize = 256;
const EXECUTABLE_CONFIG_SECTION_REGEX: &str =
    "^(filter\\.|merge\\.|diff\\.|interactive\\.|core\\.|tar\\.|mergetool\\.|difftool\\.)";

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

    let unsafe_keys =
        unsafe_execution_config_keys_with_policy(&output.stdout, allow_verified_core_worktree)?;
    if !unsafe_keys.is_empty() {
        let labels = unsafe_keys
            .iter()
            .map(|key| escaped_config_key_label(key))
            .collect::<Vec<_>>();
        bail!(
            "host Git mutation is blocked by executable repository configuration: {}",
            labels.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
fn unsafe_execution_config_keys(payload: &[u8]) -> Result<Vec<String>> {
    unsafe_execution_config_keys_with_policy(payload, false)
}

fn unsafe_execution_config_keys_with_policy(
    payload: &[u8],
    allow_verified_core_worktree: bool,
) -> Result<Vec<String>> {
    let mut unsafe_keys = BTreeSet::new();
    for record in payload.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        if record.len() > MAX_GIT_CONFIG_KEY_BYTES {
            bail!("host Git execution-config audit returned an oversized key");
        }
        let key = std::str::from_utf8(record)
            .context("host Git execution-config audit returned a non-UTF-8 key")?
            .to_ascii_lowercase();
        if config_key_enables_external_execution(&key)
            && !(allow_verified_core_worktree && key == "core.worktree")
        {
            unsafe_keys.insert(key);
            if unsafe_keys.len() > MAX_REPORTED_EXECUTION_KEYS {
                bail!("host Git execution-config audit found too many executable keys");
            }
        }
    }
    Ok(unsafe_keys.into_iter().collect())
}

fn escaped_config_key_label(key: &str) -> String {
    let retained_bytes = MAX_REPORTED_CONFIG_KEY_BYTES.saturating_sub(3);
    let mut label = String::new();
    let mut truncated = false;
    for character in key.chars().flat_map(char::escape_default) {
        if label.len().saturating_add(character.len_utf8()) > retained_bytes {
            truncated = true;
            break;
        }
        label.push(character);
    }
    if truncated {
        label.push_str("...");
    }
    label
}

fn config_key_enables_external_execution(key: &str) -> bool {
    matches!(
        key,
        "diff.external" | "interactive.difffilter" | "core.alternaterefscommand" | "core.worktree"
    ) || scoped_key(key, "filter", "clean")
        || scoped_key(key, "filter", "smudge")
        || scoped_key(key, "filter", "process")
        || scoped_key(key, "filter", "required")
        || scoped_key(key, "merge", "driver")
        || scoped_key(key, "diff", "command")
        || scoped_key(key, "diff", "textconv")
        || scoped_key(key, "tar", "command")
        || scoped_key(key, "mergetool", "cmd")
        || scoped_key(key, "difftool", "cmd")
}

fn scoped_key(key: &str, section: &str, variable: &str) -> bool {
    key.strip_prefix(section)
        .and_then(|key| key.strip_prefix('.'))
        .and_then(|key| key.strip_suffix(variable))
        .and_then(|key| key.strip_suffix('.'))
        .is_some_and(|subsection| !subsection.is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        MAX_GIT_CONFIG_KEY_BYTES, MAX_REPORTED_CONFIG_KEY_BYTES,
        config_key_enables_external_execution, ensure_host_git_execution_config_safe,
        escaped_config_key_label, git_command_can_execute_repository_configuration,
        git_repo_context, unsafe_execution_config_keys,
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
    fn executable_config_key_vocabulary_is_explicit_and_value_free() {
        for key in [
            "filter.hostile.clean",
            "filter.hostile.smudge",
            "filter.hostile.process",
            "filter.hostile.required",
            "merge.hostile.driver",
            "diff.external",
            "diff.hostile.command",
            "diff.hostile.textconv",
            "interactive.diffFilter",
            "core.alternateRefsCommand",
            "core.worktree",
            "tar.hostile.command",
            "mergetool.hostile.cmd",
            "difftool.hostile.cmd",
        ] {
            assert!(
                config_key_enables_external_execution(&key.to_ascii_lowercase()),
                "{key} must remain in the executable-config denylist"
            );
        }
        for key in [
            "credential.helper",
            "url.ssh.insteadof",
            "core.sshcommand",
            "core.hooksPath",
            "core.fsmonitor",
            "submodule.child.update",
            "submodule.child.url",
            "diff.algorithm",
        ] {
            assert!(
                !config_key_enables_external_execution(&key.to_ascii_lowercase()),
                "{key} should not expand this mutation-specific denylist"
            );
        }

        let keys = unsafe_execution_config_keys(
            b"filter.hostile.smudge\0merge.hostile.driver\0filter.hostile.smudge\0",
        )
        .expect("NUL key stream should parse");
        assert_eq!(keys, ["filter.hostile.smudge", "merge.hostile.driver"]);

        let escaped = escaped_config_key_label("filter.hostile\nname.smudge");
        assert_eq!(escaped, "filter.hostile\\nname.smudge");
        assert!(!escaped.contains('\n'));
        let bounded = escaped_config_key_label(&format!(
            "filter.{}.smudge",
            "long-name".repeat(MAX_REPORTED_CONFIG_KEY_BYTES)
        ));
        assert!(bounded.len() <= MAX_REPORTED_CONFIG_KEY_BYTES);
        assert!(bounded.ends_with("..."));

        let oversized = format!("filter.{}.smudge", "x".repeat(MAX_GIT_CONFIG_KEY_BYTES));
        let payload = format!("{oversized}\0");
        let error = unsafe_execution_config_keys(payload.as_bytes())
            .expect_err("oversized config keys must fail before diagnostic rendering");
        assert!(error.to_string().contains("oversized key"));
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
