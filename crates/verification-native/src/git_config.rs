//! Bounded, value-free Git execution configuration classification.
//! Repository identity checks and authorization remain with the caller.
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;

const MAX_REPORTED_EXECUTION_KEYS: usize = 64;
const MAX_GIT_CONFIG_KEY_BYTES: usize = 4 * 1024;
const MAX_REPORTED_CONFIG_KEY_BYTES: usize = 256;
pub const EXECUTABLE_CONFIG_SECTION_REGEX: &str =
    "^(filter\\.|merge\\.|diff\\.|interactive\\.|core\\.|tar\\.|mergetool\\.|difftool\\.)";

/// Reuse the same policy when an audited caller owns a stricter process deadline.
pub fn validate_host_git_execution_config_keys(payload: &[u8]) -> Result<()> {
    validate_execution_config_keys_with_policy(payload, false)
}

fn validate_execution_config_keys_with_policy(
    payload: &[u8],
    allow_verified_core_worktree: bool,
) -> Result<()> {
    let unsafe_keys =
        unsafe_execution_config_keys_with_policy(payload, allow_verified_core_worktree)?;
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

/// Render the existing bounded, escaped key diagnostic without inspecting config values.
pub fn escaped_config_key_label(key: &str) -> String {
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

pub fn config_key_enables_external_execution(key: &str) -> bool {
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

/// Classify keys after the caller has independently verified the submodule worktree.
/// This pure check grants no repository, task, or delivery authority.
pub fn validate_submodule_config_keys_after_worktree_validation(payload: &[u8]) -> Result<()> {
    validate_execution_config_keys_with_policy(payload, true)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn verified_submodule_exception_only_excludes_core_worktree_from_the_same_limits() {
        assert!(validate_host_git_execution_config_keys(b"core.worktree\0").is_err());
        assert!(
            validate_submodule_config_keys_after_worktree_validation(b"core.worktree\0").is_ok()
        );
        assert!(
            validate_submodule_config_keys_after_worktree_validation(
                b"core.worktree\0filter.hostile.clean\0"
            )
            .is_err()
        );
        let keys = (0..MAX_REPORTED_EXECUTION_KEYS)
            .map(|index| format!("filter.key{index}.clean\0"))
            .collect::<String>();
        let with_worktree = format!("{keys}core.worktree\0");
        assert!(
            validate_host_git_execution_config_keys(with_worktree.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("too many")
        );
        let error =
            validate_submodule_config_keys_after_worktree_validation(with_worktree.as_bytes())
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("blocked by executable repository configuration")
        );
        let over_limit = format!("{with_worktree}filter.additional.clean\0");
        assert!(
            validate_submodule_config_keys_after_worktree_validation(over_limit.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("too many")
        );
        for invalid in [vec![0xff, 0], vec![b'x'; MAX_GIT_CONFIG_KEY_BYTES + 1]] {
            assert!(validate_host_git_execution_config_keys(&invalid).is_err());
            assert!(validate_submodule_config_keys_after_worktree_validation(&invalid).is_err());
        }
    }
}
