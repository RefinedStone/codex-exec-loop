use std::collections::BTreeSet;
use std::path::{Component, Path};
use std::time::Instant;

use super::context::{PrivateGitContext, text_line};
use super::environment::{MAX_BLOB_BYTES, MAX_PATHS, MAX_TOTAL_BYTES, valid_oid};
use crate::domain::planning::verification::TaskVerificationNotRunReason as Reason;
use crate::trusted_executable::PinnedNativeExecutable;

const MAX_DIFF_METADATA_BYTES: usize = 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;

pub(super) fn commit_tree(
    context: &mut PrivateGitContext,
    binary: &PinnedNativeExecutable,
    commit: &str,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, Reason> {
    let exact = context
        .output(
            binary,
            &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
            128,
            deadline,
            cancelled,
        )
        .map_err(|_| Reason::Unavailable)?;
    if text_line(&exact.stdout).map_err(|_| Reason::InvalidInput)? != commit {
        return Err(Reason::InvalidInput);
    }
    let tree = context
        .output(
            binary,
            &["rev-parse", "--verify", &format!("{commit}^{{tree}}")],
            128,
            deadline,
            cancelled,
        )
        .map_err(|_| Reason::Unavailable)?;
    let tree = text_line(&tree.stdout).map_err(|_| Reason::InvalidInput)?;
    if !valid_oid(&tree) || tree.len() != commit.len() {
        return Err(Reason::InvalidInput);
    }
    Ok(tree)
}

pub(super) fn inspect_changed_inputs(
    context: &mut PrivateGitContext,
    binary: &PinnedNativeExecutable,
    base: &str,
    source: &str,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Reason> {
    let kind = context
        .output(
            binary,
            &["cat-file", "-t", source],
            128,
            deadline,
            cancelled,
        )
        .map_err(|_| Reason::Unavailable)?;
    if text_line(&kind.stdout).map_err(|_| Reason::InvalidInput)? != "tree" {
        return Err(Reason::InvalidInput);
    }
    let diff = context
        .output(
            binary,
            &[
                "diff",
                "--raw",
                "-z",
                "--no-abbrev",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--ignore-submodules=none",
                base,
                source,
                "--",
            ],
            MAX_DIFF_METADATA_BYTES,
            deadline,
            cancelled,
        )
        .map_err(|_| Reason::Unavailable)?;
    let blobs = parse_changed_blobs(&diff.stdout, source.len())?;
    let mut total = 0_u64;
    for oid in blobs {
        let size = context
            .output(binary, &["cat-file", "-s", &oid], 128, deadline, cancelled)
            .map_err(|_| Reason::Unavailable)?;
        let size = text_line(&size.stdout).map_err(|_| Reason::InvalidInput)?;
        let size: u64 = size.parse().map_err(|_| Reason::InvalidInput)?;
        total = total.checked_add(size).ok_or(Reason::InvalidInput)?;
        if size > MAX_BLOB_BYTES as u64 || total > MAX_TOTAL_BYTES {
            return Err(Reason::InvalidInput);
        }
        let output = context
            .output(
                binary,
                &["cat-file", "blob", &oid],
                MAX_BLOB_BYTES,
                deadline,
                cancelled,
            )
            .map_err(|_| Reason::Unavailable)?;
        if output.stdout.len() as u64 != size
            || output.stdout.contains(&0)
            || std::str::from_utf8(&output.stdout).is_err()
        {
            return Err(Reason::InvalidInput);
        }
    }
    Ok(())
}

pub(super) fn parse_changed_blobs(
    bytes: &[u8],
    oid_length: usize,
) -> Result<BTreeSet<String>, Reason> {
    let mut blobs = BTreeSet::new();
    if bytes.is_empty() {
        return Ok(blobs);
    }
    let body = bytes.strip_suffix(&[0]).ok_or(Reason::InvalidInput)?;
    let records = body.split(|byte| *byte == 0).collect::<Vec<_>>();
    if records.len() % 2 != 0 || records.len() / 2 > MAX_PATHS {
        return Err(Reason::InvalidInput);
    }
    for pair in records.chunks_exact(2) {
        let header = std::str::from_utf8(pair[0]).map_err(|_| Reason::InvalidInput)?;
        let fields = header
            .strip_prefix(':')
            .ok_or(Reason::InvalidInput)?
            .split(' ')
            .collect::<Vec<_>>();
        if fields.len() != 5 || !matches!(fields[4], "A" | "D" | "M" | "T") {
            return Err(Reason::InvalidInput);
        }
        let path = std::str::from_utf8(pair[1]).map_err(|_| Reason::InvalidInput)?;
        if path.is_empty()
            || path.len() > MAX_PATH_BYTES
            || path.contains('\\')
            || path.chars().any(char::is_control)
            || Path::new(path).is_absolute()
            || !Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(Reason::InvalidInput);
        }
        for (mode, oid) in [(fields[0], fields[2]), (fields[1], fields[3])] {
            if oid.len() != oid_length || !valid_oid(oid) {
                return Err(Reason::InvalidInput);
            }
            if mode == "000000" {
                if !oid.bytes().all(|byte| byte == b'0') {
                    return Err(Reason::InvalidInput);
                }
            } else if matches!(mode, "100644" | "100755") && !oid.bytes().all(|byte| byte == b'0') {
                blobs.insert(oid.into());
            } else {
                return Err(Reason::InvalidInput);
            }
        }
    }
    Ok(blobs)
}
