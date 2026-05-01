use std::collections::BTreeSet;

use super::readiness::{command_succeeds, run_command};
use super::{
    AGENT_BRANCH_TRUNCATION_HASH_LEN, AKRA_AGENT_BRANCH_PREFIX, DEFAULT_PUSH_REMOTE_NAME,
    MAX_AGENT_BRANCH_SLUG_LEN,
};

pub(super) fn allocate_agent_branch_name(
    repo_root: &str,
    slot_id: &str,
    task_slug: &str,
    task_id: &str,
    task_title: &str,
) -> String {
    let sanitized_slug = sanitize_task_slug(task_slug)
        .or_else(|| sanitize_task_slug(task_id))
        .or_else(|| sanitize_task_slug(task_title))
        .unwrap_or_else(|| "task".to_string());
    let remote_branch_names = remote_agent_branch_names(repo_root, slot_id);
    let mut collision_index = 1usize;
    loop {
        let candidate = build_agent_branch_name(slot_id, &sanitized_slug, collision_index);
        if agent_branch_name_is_available(repo_root, &candidate, &remote_branch_names) {
            return candidate;
        }
        collision_index += 1;
    }
}

fn agent_branch_name_is_available(
    repo_root: &str,
    branch_name: &str,
    remote_branch_names: &BTreeSet<String>,
) -> bool {
    !branch_exists(repo_root, branch_name) && !remote_branch_names.contains(branch_name)
}

fn build_agent_branch_name(slot_id: &str, sanitized_slug: &str, collision_index: usize) -> String {
    let collision_suffix = if collision_index > 1 {
        format!("-{collision_index}")
    } else {
        String::new()
    };
    let bounded_slug = bounded_agent_branch_slug(
        sanitized_slug,
        MAX_AGENT_BRANCH_SLUG_LEN.saturating_sub(collision_suffix.len()),
    );
    format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/{bounded_slug}{collision_suffix}")
}

fn bounded_agent_branch_slug(slug: &str, max_len: usize) -> String {
    if slug.len() <= max_len {
        return slug.to_string();
    }

    let hash = short_branch_slug_hash(slug);
    if max_len <= hash.len() {
        return hash[..max_len].to_string();
    }

    let prefix_len = max_len.saturating_sub(hash.len() + 1);
    let prefix = truncate_to_char_boundary(slug, prefix_len).trim_end_matches('-');
    if prefix.is_empty() {
        return hash[..max_len.min(hash.len())].to_string();
    }

    format!("{prefix}-{hash}")
}

pub(super) fn short_branch_slug_hash(input: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..AGENT_BRANCH_TRUNCATION_HASH_LEN].to_string()
}

fn truncate_to_char_boundary(value: &str, max_len: usize) -> &str {
    if value.len() <= max_len {
        return value;
    }

    let mut boundary = 0usize;
    for (index, character) in value.char_indices() {
        let next_boundary = index + character.len_utf8();
        if next_boundary > max_len {
            break;
        }
        boundary = next_boundary;
    }

    &value[..boundary]
}

pub(super) fn sanitize_task_slug(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_was_dash = false;

    for ch in input.chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            previous_was_dash = false;
            continue;
        }
        if !previous_was_dash && !slug.is_empty() {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

pub(super) fn branch_exists(repo_root: &str, branch_name: &str) -> bool {
    command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ],
    )
}

fn remote_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    let mut branch_names = remote_tracking_agent_branch_names(repo_root, slot_id);
    branch_names.extend(remote_live_agent_branch_names(repo_root, slot_id));
    branch_names
}

fn remote_tracking_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    let refs_prefix =
        format!("refs/remotes/{DEFAULT_PUSH_REMOTE_NAME}/{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    let branch_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    run_command(
        "git",
        [
            "-C",
            repo_root,
            "for-each-ref",
            "--format=%(refname)",
            refs_prefix.as_str(),
        ],
        None,
    )
    .map(|output| {
        output
            .lines()
            .filter_map(|line| line.strip_prefix(&refs_prefix))
            .map(|suffix| format!("{branch_prefix}{suffix}"))
            .collect()
    })
    .unwrap_or_default()
}

fn remote_live_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    let refs_prefix = "refs/heads/";
    let branch_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    let pattern = format!("refs/heads/{branch_prefix}*");
    run_command(
        "git",
        [
            "-C",
            repo_root,
            "ls-remote",
            "--heads",
            DEFAULT_PUSH_REMOTE_NAME,
            pattern.as_str(),
        ],
        None,
    )
    .map(|output| {
        output
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .filter_map(|remote_ref| remote_ref.strip_prefix(refs_prefix))
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}
