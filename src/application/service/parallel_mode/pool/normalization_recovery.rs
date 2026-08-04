use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelCommandOutput, ParallelModeRuntimePort,
};

use super::super::git_sequence::{
    GitCommandSequenceReport, GitCommandStep, GitCommandStepReport, run_git_sequence,
};
use super::paths::{git_dir_has_pending_operation_with_runtime, resolve_git_dir_with_runtime};
#[cfg(test)]
use super::worktree_paths_match;
use super::{
    PoolMutationLock, SlotGitStatus, git_command_directory, git_worktree_destination,
    inspect_slot_git_status_with_runtime, parse_worktree_records,
    worktree_paths_match_with_runtime,
};

const MAX_NORMALIZATION_PATHS: usize = 64;
const MAX_NORMALIZATION_STATUS_BYTES: usize = 1024 * 1024;
const MAX_NORMALIZATION_FILE_BYTES: usize = 1024 * 1024;
const NORMALIZATION_QUARANTINE_PREFIX: &str = ".normalization-recovery-";
const NORMALIZATION_REPLACEMENT_PREFIX: &str = ".normalization-replacement-";
const SAFE_TEXT_ATTRIBUTES: [(&str, &[&str]); 6] = [
    ("text", &["set", "auto"]),
    ("eol", &["lf"]),
    ("filter", &["unspecified", "unset"]),
    ("working-tree-encoding", &["unspecified", "unset"]),
    ("ident", &["unspecified", "unset"]),
    ("diff", &["unspecified", "unset"]),
];

#[derive(Debug)]
struct NormalizationPathProof {
    path: String,
    mode: String,
    index_oid: String,
}

pub(super) struct NormalizationRecoveryRequest<'a> {
    pub(super) repo_root: &'a str,
    pub(super) pool_root: &'a Path,
    pub(super) slot_id: &'a str,
    pub(super) slot_path: &'a Path,
    pub(super) source_oid: &'a str,
    pub(super) target_ref: &'a str,
}

pub(super) struct NormalizationRecoveryOutcome {
    pub(super) report: GitCommandSequenceReport,
    pub(super) quarantine_path: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacySlotStatusProof {
    Readable,
    EmptyIndex,
}

/*
An old pool baseline can contain CRLF or mixed blobs even though `.gitattributes` declares
`text eol=lf`. Git then reports a freshly checked-out file as unstaged because its clean
conversion differs from the malformed index blob, although the raw worktree bytes are identical
to that blob. This classifier proves that narrow state without weakening the ordinary dirty-slot
gate used by allocation, cleanup, and delivery.

Every path must be a regular unstaged-only porcelain-v2 record, its raw bytes must still equal the
current index OID, only built-in LF normalization may be configured, and Rust-normalized bytes plus
mode must already equal the frozen target tree. `hash-object --no-filters --stdin` calculates blob
identities without invoking a repository filter between proof steps.
*/
pub(super) fn has_target_equivalent_lf_normalization_drift(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    slot_status: SlotGitStatus,
    target_ref: &str,
) -> bool {
    if !slot_status.has_only_unstaged_changes()
        || target_ref.trim().is_empty()
        || runtime.ensure_git_execution_safe(slot_path).is_err()
        || !index_has_only_normal_entries(runtime, slot_path)
        || index_has_unmerged_entries(runtime, slot_path)
    {
        return false;
    }

    let Some(target_oid) = resolve_target_oid(runtime, slot_path, target_ref) else {
        return false;
    };
    let Some(path_proofs) = load_normalization_path_proofs(runtime, slot_path) else {
        return false;
    };
    if path_proofs.is_empty() || path_proofs.len() > MAX_NORMALIZATION_PATHS {
        return false;
    }

    path_proofs.iter().all(|proof| {
        path_bytes_match_index_and_target(runtime, slot_path, &target_oid, proof)
            && path_has_safe_lf_attributes(runtime, slot_path, &proof.path, false)
            && path_has_safe_lf_attributes(runtime, slot_path, &proof.path, true)
    }) && differs_only_by_cr_at_eol(runtime, slot_path)
}

pub(super) fn has_empty_linked_worktree_index(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
) -> bool {
    // A zero-byte index is the narrow interrupted-write signature observed on WSL. Keep every
    // other status failure blocked, including active Git locks and non-empty malformed indexes.
    if runtime.ensure_git_execution_safe(slot_path).is_err() {
        return false;
    }
    let Some(git_dir) = resolve_git_dir_with_runtime(runtime, slot_path) else {
        return false;
    };
    if git_dir_has_pending_operation_with_runtime(runtime, &git_dir) {
        return false;
    }
    runtime
        .read_bounded_unshared_regular_file(&git_dir.join("index"), MAX_NORMALIZATION_FILE_BYTES)
        .is_some_and(|bytes| bytes.is_empty())
}

/*
Proof alone cannot make an external editor or Git process stop writing. Instead of checking out
over the dirty worktree, provision and verify the clean target at a hidden staging path first.
Then move the legacy worktree to quarantine, repair its Git registration, and atomically rename
the staged worktree into the canonical lane. The canonical path never hosts a Git checkout: a
writer using an old file descriptor follows the legacy inode into quarantine, while a path-based
writer that recreates the brief canonical gap makes the final move fail without overwriting bytes.
*/
pub(super) fn quarantine_normalization_drift_and_replace_slot(
    runtime: &dyn ParallelModeRuntimePort,
    request: NormalizationRecoveryRequest<'_>,
    mutation_lock: &PoolMutationLock,
    recheck_unowned_authority: &dyn Fn() -> Result<(), String>,
) -> NormalizationRecoveryOutcome {
    quarantine_slot_and_replace(
        runtime,
        request,
        mutation_lock,
        recheck_unowned_authority,
        LegacySlotStatusProof::Readable,
    )
}

pub(super) fn quarantine_empty_index_and_replace_slot(
    runtime: &dyn ParallelModeRuntimePort,
    request: NormalizationRecoveryRequest<'_>,
    mutation_lock: &PoolMutationLock,
    recheck_unowned_authority: &dyn Fn() -> Result<(), String>,
) -> NormalizationRecoveryOutcome {
    quarantine_slot_and_replace(
        runtime,
        request,
        mutation_lock,
        recheck_unowned_authority,
        LegacySlotStatusProof::EmptyIndex,
    )
}

fn quarantine_slot_and_replace(
    runtime: &dyn ParallelModeRuntimePort,
    request: NormalizationRecoveryRequest<'_>,
    mutation_lock: &PoolMutationLock,
    recheck_unowned_authority: &dyn Fn() -> Result<(), String>,
    source_status_proof: LegacySlotStatusProof,
) -> NormalizationRecoveryOutcome {
    let quarantine_path = new_normalization_quarantine_path(
        runtime,
        request.pool_root,
        request.slot_id,
        request.source_oid,
    );
    let result = (|| {
        mutation_lock.verify_pool_root(request.pool_root)?;
        let canonical_slot_path = request.pool_root.join(request.slot_id);
        if !worktree_paths_match_with_runtime(runtime, request.slot_path, &canonical_slot_path) {
            return Err(
                "normalization recovery slot path is outside its canonical lane".to_string(),
            );
        }
        let incomplete_replacements = normalization_replacement_artifacts_for_slot(
            runtime,
            request.pool_root,
            request.slot_id,
        )?;
        if !incomplete_replacements.is_empty() {
            return Err(format!(
                "normalization recovery for slot `{}` is already incomplete; preserved replacement artifact(s): {}",
                request.slot_id,
                incomplete_replacements
                    .iter()
                    .map(|path| format!("`{}`", path.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let target_oid = resolve_target_oid(runtime, request.slot_path, request.target_ref)
            .ok_or_else(|| "normalization recovery target OID could not be resolved".to_string())?;
        let quarantine_path = quarantine_path.as_ref().map_err(Clone::clone)?;
        let replacement_path = new_normalization_replacement_path(
            runtime,
            request.pool_root,
            request.slot_id,
            &target_oid,
        )?;
        ensure_path_absent(runtime, quarantine_path, "quarantine")?;
        ensure_path_absent(runtime, &replacement_path, "replacement staging")?;
        let canonical_repo_root = runtime
            .canonicalize_path(Path::new(request.repo_root))
            .map_err(|error| {
                format!("normalization recovery repository root could not be pinned: {error}")
            })?;
        let git_repo_root = git_command_directory(&canonical_repo_root)?;
        runtime
            .ensure_git_execution_safe(Path::new(request.repo_root))
            .map_err(|error| format!("normalization recovery Git execution is unsafe: {error}"))?;
        recheck_unowned_authority()?;
        verify_registered_detached_worktree(
            runtime,
            &git_repo_root,
            request.slot_path,
            request.source_oid,
            false,
            source_status_proof,
        )?;
        mutation_lock.verify_pool_root(request.pool_root)?;

        let git_slot_path = git_worktree_path(&canonical_repo_root, request.slot_path)?;
        let git_quarantine_path = git_worktree_path(&canonical_repo_root, quarantine_path)?;
        let git_replacement_path = git_worktree_path(&canonical_repo_root, &replacement_path)?;
        let staging_directory = runtime.create_private_staging_directory(&replacement_path)?;
        mutation_lock.verify_pool_root(request.pool_root)?;
        run_before_normalization_staging_provision_hook(request.slot_path, &replacement_path);
        staging_directory.verify()?;
        let mut report = run_git_sequence(
            runtime,
            "quarantine target-equivalent LF normalization drift",
            vec![GitCommandStep::new(
                "provision clean replacement in normalization staging",
                [
                    "-C",
                    git_repo_root.as_str(),
                    "worktree",
                    "add",
                    "--detach",
                    git_replacement_path.as_str(),
                    target_oid.as_str(),
                ],
            )],
        );
        if !report.succeeded() {
            return Ok(report);
        }
        if let Err(detail) = staging_directory.verify() {
            append_verification_failure(
                &mut report,
                "verify normalization replacement staging identity",
                detail,
            );
            return Ok(report);
        }
        if let Err(detail) = verify_registered_detached_worktree(
            runtime,
            &git_repo_root,
            &replacement_path,
            &target_oid,
            true,
            LegacySlotStatusProof::Readable,
        ) {
            append_verification_failure(
                &mut report,
                "verify clean normalization replacement staging",
                detail,
            );
            return Ok(report);
        }

        run_before_normalization_quarantine_hook(request.slot_path);
        let pre_move_validation = (|| {
            mutation_lock.verify_pool_root(request.pool_root)?;
            recheck_unowned_authority()?;
            ensure_path_absent(runtime, quarantine_path, "quarantine")?;
            verify_registered_detached_worktree(
                runtime,
                &git_repo_root,
                request.slot_path,
                request.source_oid,
                false,
                source_status_proof,
            )?;
            staging_directory.verify()?;
            verify_registered_detached_worktree(
                runtime,
                &git_repo_root,
                &replacement_path,
                &target_oid,
                true,
                LegacySlotStatusProof::Readable,
            )
        })();
        if let Err(detail) = pre_move_validation {
            append_verification_failure(
                &mut report,
                "revalidate normalization recovery before quarantine move",
                detail,
            );
            return Ok(report);
        }

        run_before_normalization_atomic_rename_hook(quarantine_path);
        if !append_atomic_rename_step(
            runtime,
            &mut report,
            "atomically move legacy slot to normalization recovery quarantine",
            request.slot_path,
            quarantine_path,
        ) {
            return Ok(report);
        }
        if let Err(detail) = mutation_lock.verify_pool_root(request.pool_root) {
            append_verification_failure(
                &mut report,
                "revalidate pool root after quarantining legacy slot",
                detail,
            );
            return Ok(report);
        }
        let repair_quarantine_report = run_git_sequence(
            runtime,
            "quarantine target-equivalent LF normalization drift",
            vec![GitCommandStep::new(
                "repair quarantined worktree registration",
                [
                    "-C",
                    git_repo_root.as_str(),
                    "worktree",
                    "repair",
                    git_quarantine_path.as_str(),
                ],
            )],
        );
        report.steps.extend(repair_quarantine_report.steps);
        if !report.succeeded() {
            return Ok(report);
        }

        run_after_normalization_quarantine_move_hook(request.slot_path);
        let pre_install_validation = (|| {
            mutation_lock.verify_pool_root(request.pool_root)?;
            recheck_unowned_authority()?;
            ensure_path_absent(runtime, request.slot_path, "canonical slot")?;
            verify_registered_detached_worktree(
                runtime,
                &git_repo_root,
                quarantine_path,
                request.source_oid,
                false,
                source_status_proof,
            )?;
            staging_directory.verify()?;
            verify_registered_detached_worktree(
                runtime,
                &git_repo_root,
                &replacement_path,
                &target_oid,
                true,
                LegacySlotStatusProof::Readable,
            )
        })();
        if let Err(detail) = pre_install_validation {
            append_verification_failure(
                &mut report,
                "revalidate normalization recovery before canonical move",
                detail,
            );
            return Ok(report);
        }

        // Windows pins the staging directory without FILE_SHARE_DELETE, so release the verified
        // handle immediately before the no-replace move into the canonical lane.
        drop(staging_directory);
        run_before_normalization_atomic_rename_hook(request.slot_path);
        if !append_atomic_rename_step(
            runtime,
            &mut report,
            "atomically move staged replacement into the canonical pool slot",
            &replacement_path,
            request.slot_path,
        ) {
            return Ok(report);
        }
        if let Err(detail) = mutation_lock.verify_pool_root(request.pool_root) {
            append_verification_failure(
                &mut report,
                "revalidate pool root after installing canonical replacement",
                detail,
            );
            return Ok(report);
        }
        let repair_replacement_report = run_git_sequence(
            runtime,
            "quarantine target-equivalent LF normalization drift",
            vec![GitCommandStep::new(
                "repair canonical replacement worktree registration",
                [
                    "-C",
                    git_repo_root.as_str(),
                    "worktree",
                    "repair",
                    git_slot_path.as_str(),
                ],
            )],
        );
        report.steps.extend(repair_replacement_report.steps);
        if !report.succeeded() {
            return Ok(report);
        }

        let completed_validation =
            mutation_lock
                .verify_pool_root(request.pool_root)
                .and_then(|()| {
                    verify_completed_recovery(
                        runtime,
                        &git_repo_root,
                        request.slot_path,
                        quarantine_path,
                        &replacement_path,
                        request.source_oid,
                        &target_oid,
                        source_status_proof,
                    )
                });
        if let Err(detail) = completed_validation {
            append_verification_failure(
                &mut report,
                "verify quarantined legacy slot and clean replacement",
                detail,
            );
        }
        Ok(report)
    })();

    let report = result.unwrap_or_else(normalization_recovery_preflight_failure);
    let completed_quarantine_path = report.succeeded().then(|| quarantine_path.ok()).flatten();
    NormalizationRecoveryOutcome {
        report,
        quarantine_path: completed_quarantine_path,
    }
}

pub(in crate::application::service::parallel_mode) fn normalization_quarantine_path(
    pool_root: &Path,
    slot_id: &str,
    source_oid: &str,
) -> Option<PathBuf> {
    if slot_id.is_empty()
        || !slot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !matches!(source_oid.len(), 40 | 64)
        || !source_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(pool_root.join(format!(
        "{NORMALIZATION_QUARANTINE_PREFIX}{slot_id}-{}",
        source_oid.to_ascii_lowercase()
    )))
}

fn new_normalization_quarantine_path(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
    slot_id: &str,
    source_oid: &str,
) -> Result<PathBuf, String> {
    let base_path = normalization_quarantine_path(pool_root, slot_id, source_oid)
        .ok_or_else(|| "normalization recovery source identity is invalid".to_string())?;
    match runtime.path_exists_checked(&base_path) {
        Ok(false) => Ok(base_path),
        Ok(true) => Ok(base_path.with_file_name(format!(
            "{}-{}",
            base_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    "normalization recovery quarantine name is not valid Unicode".to_string()
                })?,
            random_normalization_nonce(runtime)?
        ))),
        Err(error) => Err(format!(
            "normalization quarantine path could not be inspected at `{}`: {error}",
            base_path.display()
        )),
    }
}

fn new_normalization_replacement_path(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
    slot_id: &str,
    target_oid: &str,
) -> Result<PathBuf, String> {
    if slot_id.is_empty()
        || !slot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !matches!(target_oid.len(), 40 | 64)
        || !target_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("normalization recovery target identity is invalid".to_string());
    }
    let nonce = random_normalization_nonce(runtime)?;
    Ok(pool_root.join(format!(
        "{NORMALIZATION_REPLACEMENT_PREFIX}{slot_id}-{}-{nonce}",
        target_oid.to_ascii_lowercase()
    )))
}

fn random_normalization_nonce(runtime: &dyn ParallelModeRuntimePort) -> Result<String, String> {
    let mut nonce = [0_u8; 16];
    runtime
        .fill_secure_random(&mut nonce)
        .map_err(|error| format!("normalization recovery requires OS randomness: {error}"))?;
    Ok(nonce.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn normalization_recovery_artifact_paths(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
) -> Result<Vec<PathBuf>, String> {
    let entries = match runtime.read_directory_paths(pool_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "normalization recovery artifacts could not be inspected under `{}`: {error}",
                pool_root.display()
            ));
        }
    };
    let mut artifacts = Vec::new();
    for path in entries {
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                normalization_quarantine_slot_id(name).is_some()
                    || normalization_replacement_slot_id(name).is_some()
            })
        {
            artifacts.push(path);
        }
    }
    artifacts.sort();
    Ok(artifacts)
}

fn normalization_quarantine_slot_id(name: &str) -> Option<&str> {
    let remainder = name.strip_prefix(NORMALIZATION_QUARANTINE_PREFIX)?;
    let (slot_and_oid, last_component) = remainder.rsplit_once('-')?;
    let (slot_id, source_oid) = if is_hex_identifier(last_component, &[40, 64]) {
        (slot_and_oid, last_component)
    } else if is_hex_identifier(last_component, &[32]) {
        slot_and_oid.rsplit_once('-')?
    } else {
        return None;
    };
    (is_valid_normalization_slot_id(slot_id) && is_hex_identifier(source_oid, &[40, 64]))
        .then_some(slot_id)
}

fn normalization_replacement_slot_id(name: &str) -> Option<&str> {
    let remainder = name.strip_prefix(NORMALIZATION_REPLACEMENT_PREFIX)?;
    let (slot_and_oid, nonce) = remainder.rsplit_once('-')?;
    let (slot_id, target_oid) = slot_and_oid.rsplit_once('-')?;
    (is_valid_normalization_slot_id(slot_id)
        && is_hex_identifier(target_oid, &[40, 64])
        && is_hex_identifier(nonce, &[32]))
    .then_some(slot_id)
}

fn is_valid_normalization_slot_id(slot_id: &str) -> bool {
    !slot_id.is_empty()
        && slot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn is_hex_identifier(value: &str, allowed_lengths: &[usize]) -> bool {
    allowed_lengths.contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn normalization_replacement_artifacts_for_slot(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
    slot_id: &str,
) -> Result<Vec<PathBuf>, String> {
    normalization_recovery_artifact_paths(runtime, pool_root).map(|artifacts| {
        artifacts
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(normalization_replacement_slot_id)
                    == Some(slot_id)
            })
            .collect()
    })
}

pub(super) fn has_normalization_replacement_artifact_for_slot(
    artifacts: &[PathBuf],
    slot_id: &str,
) -> bool {
    artifacts.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(normalization_replacement_slot_id)
            == Some(slot_id)
    })
}

fn ensure_path_absent(
    runtime: &dyn ParallelModeRuntimePort,
    path: &Path,
    label: &str,
) -> Result<(), String> {
    match runtime.path_exists_checked(path) {
        Ok(false) => Ok(()),
        Ok(true) => Err(format!(
            "normalization {label} path already exists at `{}`",
            path.display()
        )),
        Err(error) => Err(format!(
            "normalization {label} path could not be inspected at `{}`: {error}",
            path.display()
        )),
    }
}

fn git_worktree_path(canonical_repo_root: &Path, worktree_path: &Path) -> Result<String, String> {
    git_worktree_destination(canonical_repo_root, worktree_path)?
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "normalization recovery path is not valid Unicode for Git: `{}`",
                worktree_path.display()
            )
        })
}

fn append_atomic_rename_step(
    runtime: &dyn ParallelModeRuntimePort,
    report: &mut GitCommandSequenceReport,
    label: &str,
    source: &Path,
    destination: &Path,
) -> bool {
    let result = runtime.atomic_rename_noreplace(source, destination);
    report.steps.push(GitCommandStepReport {
        label: label.to_string(),
        args: vec![
            source.display().to_string(),
            destination.display().to_string(),
        ],
        exit_code: Some(if result.is_ok() { 0 } else { 1 }),
        stdout: String::new(),
        stderr: result.err().map_or_else(String::new, |error| {
            format!(
                "atomic normalization worktree move from `{}` to `{}` failed: {error}",
                source.display(),
                destination.display()
            )
        }),
    });
    report.succeeded()
}

fn verify_registered_detached_worktree(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    worktree_path: &Path,
    expected_oid: &str,
    require_clean: bool,
    status_proof: LegacySlotStatusProof,
) -> Result<(), String> {
    if !worktree_paths_match_with_runtime(runtime, worktree_path, worktree_path) {
        return Err(format!(
            "normalization recovery worktree path is missing or link-aliased at `{}`",
            worktree_path.display()
        ));
    }
    let records = load_worktree_inventory(runtime, repo_root)?;
    if !records.iter().any(|record| {
        worktree_paths_match_with_runtime(runtime, &record.path, worktree_path)
            && record.detached
            && record.head_sha.eq_ignore_ascii_case(expected_oid)
    }) {
        return Err(format!(
            "normalization recovery worktree is not registered detached at `{expected_oid}`: `{}`",
            worktree_path.display()
        ));
    }
    if status_proof == LegacySlotStatusProof::EmptyIndex {
        return has_empty_linked_worktree_index(runtime, worktree_path)
            .then_some(())
            .ok_or_else(|| {
                format!(
                    "normalization recovery source no longer has an unowned empty index at `{}`",
                    worktree_path.display()
                )
            });
    }
    let status = inspect_slot_git_status_with_runtime(runtime, worktree_path).map_err(|error| {
        format!(
            "normalization recovery status could not be inspected at `{}`: {error}",
            worktree_path.display()
        )
    })?;
    if status.has_pending_operation() {
        return Err(format!(
            "normalization recovery worktree has pending Git operation/lock metadata at `{}`",
            worktree_path.display()
        ));
    }
    if require_clean && !status.is_clean_baseline() {
        return Err(format!(
            "normalization replacement worktree is not clean at `{}`",
            worktree_path.display()
        ));
    }
    Ok(())
}

fn load_worktree_inventory(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> Result<Vec<super::GitWorktreeRecord>, String> {
    let output = run_git(
        runtime,
        Path::new(repo_root),
        ["worktree", "list", "--porcelain"],
    )
    .ok_or_else(|| "normalization recovery worktree inventory command failed".to_string())?;
    if !output.succeeded() {
        return Err("normalization recovery worktree inventory could not be read".to_string());
    }
    let output = std::str::from_utf8(&output.stdout)
        .map_err(|_| "normalization recovery worktree inventory is not UTF-8".to_string())?;
    Ok(parse_worktree_records(output))
}

#[allow(clippy::too_many_arguments)]
fn verify_completed_recovery(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    slot_path: &Path,
    quarantine_path: &Path,
    replacement_path: &Path,
    source_oid: &str,
    target_oid: &str,
    source_status_proof: LegacySlotStatusProof,
) -> Result<(), String> {
    verify_registered_detached_worktree(
        runtime,
        repo_root,
        quarantine_path,
        source_oid,
        false,
        source_status_proof,
    )?;
    verify_registered_detached_worktree(
        runtime,
        repo_root,
        slot_path,
        target_oid,
        true,
        LegacySlotStatusProof::Readable,
    )?;
    ensure_path_absent(runtime, replacement_path, "replacement staging")?;
    let records = load_worktree_inventory(runtime, repo_root)?;
    if records.iter().any(|record| record.path == replacement_path) {
        return Err(format!(
            "normalization replacement staging remains registered at `{}`",
            replacement_path.display()
        ));
    }
    Ok(())
}

fn append_verification_failure(report: &mut GitCommandSequenceReport, label: &str, detail: String) {
    report.steps.push(GitCommandStepReport {
        label: label.to_string(),
        args: vec!["worktree/status/head verification".to_string()],
        exit_code: Some(1),
        stdout: String::new(),
        stderr: detail,
    });
}

fn normalization_recovery_preflight_failure(detail: String) -> GitCommandSequenceReport {
    GitCommandSequenceReport {
        label: "quarantine target-equivalent LF normalization drift".to_string(),
        steps: vec![GitCommandStepReport {
            label: "verify normalization recovery preconditions".to_string(),
            args: vec!["pool/worktree/target verification".to_string()],
            exit_code: None,
            stdout: String::new(),
            stderr: detail,
        }],
    }
}

fn resolve_target_oid(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    target_ref: &str,
) -> Option<String> {
    let commit_ref = format!("{target_ref}^{{commit}}");
    let output = run_git(
        runtime,
        slot_path,
        ["rev-parse", "--verify", commit_ref.as_str()],
    )?;
    successful_ascii_stdout(output)
}

fn index_has_only_normal_entries(runtime: &dyn ParallelModeRuntimePort, slot_path: &Path) -> bool {
    let Some(output) = run_git(runtime, slot_path, ["ls-files", "-v", "-z", "--"]) else {
        return false;
    };
    output.succeeded()
        && output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .all(|record| record.starts_with(b"H "))
}

fn index_has_unmerged_entries(runtime: &dyn ParallelModeRuntimePort, slot_path: &Path) -> bool {
    run_git(runtime, slot_path, ["ls-files", "--unmerged", "-z", "--"])
        .is_none_or(|output| !output.succeeded() || !output.stdout.is_empty())
}

fn load_normalization_path_proofs(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
) -> Option<Vec<NormalizationPathProof>> {
    let output = run_git(
        runtime,
        slot_path,
        [
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--ignore-submodules=none",
        ],
    )?;
    if !output.succeeded() || output.stdout.len() > MAX_NORMALIZATION_STATUS_BYTES {
        return None;
    }

    let mut proofs = Vec::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let record = std::str::from_utf8(record).ok()?;
        let fields = record.splitn(9, ' ').collect::<Vec<_>>();
        if fields.len() != 9
            || fields[0] != "1"
            || fields[1] != ".M"
            || fields[2] != "N..."
            || fields[3] != fields[4]
            || fields[4] != fields[5]
            || !matches!(fields[3], "100644" | "100755")
            || fields[6] != fields[7]
            || fields[8].is_empty()
        {
            return None;
        }
        proofs.push(NormalizationPathProof {
            path: fields[8].to_string(),
            mode: fields[4].to_string(),
            index_oid: fields[7].to_string(),
        });
        if proofs.len() > MAX_NORMALIZATION_PATHS {
            return None;
        }
    }
    Some(proofs)
}

fn differs_only_by_cr_at_eol(runtime: &dyn ParallelModeRuntimePort, slot_path: &Path) -> bool {
    run_git(
        runtime,
        slot_path,
        [
            "diff",
            "--quiet",
            "--ignore-cr-at-eol",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
            "--",
        ],
    )
    .is_some_and(|output| output.succeeded())
}

fn normalization_candidate_path(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    path: &str,
) -> Option<PathBuf> {
    let relative_path = Path::new(path);
    if relative_path.as_os_str().is_empty()
        || relative_path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    let candidate_path = slot_path.join(relative_path);
    if !worktree_paths_match_with_runtime(runtime, &candidate_path, &candidate_path) {
        return None;
    }
    Some(candidate_path)
}

fn path_has_safe_lf_attributes(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    path: &str,
    cached: bool,
) -> bool {
    let mut args = vec![OsString::from("check-attr")];
    if cached {
        args.push(OsString::from("--cached"));
    }
    args.extend([
        OsString::from("-z"),
        OsString::from("text"),
        OsString::from("eol"),
        OsString::from("filter"),
        OsString::from("working-tree-encoding"),
        OsString::from("ident"),
        OsString::from("diff"),
        OsString::from("--"),
        OsString::from(path),
    ]);
    let Some(output) = run_git_os(runtime, slot_path, args) else {
        return false;
    };
    if !output.succeeded() {
        return false;
    }
    let fields = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    fields.len() == SAFE_TEXT_ATTRIBUTES.len() * 3
        && fields.chunks_exact(3).zip(SAFE_TEXT_ATTRIBUTES).all(
            |(chunk, (expected_attribute, allowed_values))| {
                chunk[0] == path.as_bytes()
                    && chunk[1] == expected_attribute.as_bytes()
                    && allowed_values
                        .iter()
                        .any(|allowed| chunk[2] == allowed.as_bytes())
            },
        )
}

fn path_bytes_match_index_and_target(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    target_oid: &str,
    proof: &NormalizationPathProof,
) -> bool {
    let Some(candidate_path) = normalization_candidate_path(runtime, slot_path, &proof.path) else {
        return false;
    };
    let Some(raw_bytes) =
        runtime.read_bounded_unshared_regular_file(&candidate_path, MAX_NORMALIZATION_FILE_BYTES)
    else {
        return false;
    };
    let Some(raw_oid) = hash_blob_bytes(runtime, slot_path, &raw_bytes) else {
        return false;
    };
    if raw_oid != proof.index_oid {
        return false;
    }
    let normalized_bytes = normalize_crlf(&raw_bytes);
    if normalized_bytes == raw_bytes {
        return false;
    }
    let Some(normalized_oid) = hash_blob_bytes(runtime, slot_path, &normalized_bytes) else {
        return false;
    };
    target_tree_matches_blob(runtime, slot_path, target_oid, proof, &normalized_oid)
}

fn normalize_crlf(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            normalized.push(b'\n');
            index += 2;
        } else {
            normalized.push(bytes[index]);
            index += 1;
        }
    }
    normalized
}

fn hash_blob_bytes(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    bytes: &[u8],
) -> Option<String> {
    let mut command_args = vec![OsString::from("-C"), slot_path.as_os_str().to_os_string()];
    command_args.extend([
        OsString::from("hash-object"),
        OsString::from("--no-filters"),
        OsString::from("--stdin"),
    ]);
    let output = runtime.run_git_command(&command_args, Some(bytes)).ok()?;
    successful_ascii_stdout(output)
}

fn target_tree_matches_blob(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    target_oid: &str,
    proof: &NormalizationPathProof,
    normalized_oid: &str,
) -> bool {
    let Some(output) = run_git(
        runtime,
        slot_path,
        ["ls-tree", "-z", target_oid, "--", proof.path.as_str()],
    ) else {
        return false;
    };
    if !output.succeeded() {
        return false;
    }
    let records = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    if records.len() != 1 {
        return false;
    }
    let Ok(record) = std::str::from_utf8(records[0]) else {
        return false;
    };
    let Some((metadata, path)) = record.split_once('\t') else {
        return false;
    };
    let metadata = metadata.split_whitespace().collect::<Vec<_>>();
    metadata.len() == 3
        && metadata[0] == proof.mode
        && metadata[1] == "blob"
        && metadata[2] == normalized_oid
        && metadata[2] != proof.index_oid
        && path == proof.path
}

fn run_git<const N: usize>(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    args: [&str; N],
) -> Option<ParallelCommandOutput> {
    run_git_os(
        runtime,
        slot_path,
        args.into_iter().map(OsString::from).collect(),
    )
}

fn run_git_os(
    runtime: &dyn ParallelModeRuntimePort,
    slot_path: &Path,
    args: Vec<OsString>,
) -> Option<ParallelCommandOutput> {
    let mut command_args = vec![OsString::from("-C"), slot_path.as_os_str().to_os_string()];
    command_args.extend(args);
    runtime.run_git_command(&command_args, None).ok()
}

fn successful_ascii_stdout(output: ParallelCommandOutput) -> Option<String> {
    if !output.succeeded() {
        return None;
    }
    let value = std::str::from_utf8(&output.stdout).ok()?.trim();
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

#[cfg(test)]
type BeforeQuarantineHook = Box<dyn FnOnce(&Path) + Send + 'static>;

#[cfg(test)]
static BEFORE_NORMALIZATION_QUARANTINE_HOOK: std::sync::Mutex<
    Vec<(PathBuf, BeforeQuarantineHook)>,
> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
static AFTER_NORMALIZATION_QUARANTINE_MOVE_HOOK: std::sync::Mutex<
    Vec<(PathBuf, BeforeQuarantineHook)>,
> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
static BEFORE_NORMALIZATION_ATOMIC_RENAME_HOOK: std::sync::Mutex<
    Vec<(PathBuf, BeforeQuarantineHook)>,
> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
static BEFORE_NORMALIZATION_STAGING_PROVISION_HOOK: std::sync::Mutex<
    Vec<(PathBuf, BeforeQuarantineHook)>,
> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn install_before_normalization_quarantine_hook(
    slot_path: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    BEFORE_NORMALIZATION_QUARANTINE_HOOK
        .lock()
        .expect("normalization quarantine test hook should not be poisoned")
        .push((slot_path.to_path_buf(), Box::new(hook)));
}

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn install_after_normalization_quarantine_move_hook(
    slot_path: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    AFTER_NORMALIZATION_QUARANTINE_MOVE_HOOK
        .lock()
        .expect("normalization post-move test hook should not be poisoned")
        .push((slot_path.to_path_buf(), Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(in crate::application::service::parallel_mode) fn install_before_normalization_atomic_rename_hook(
    destination: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    BEFORE_NORMALIZATION_ATOMIC_RENAME_HOOK
        .lock()
        .expect("normalization atomic-rename test hook should not be poisoned")
        .push((destination.to_path_buf(), Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(in crate::application::service::parallel_mode) fn install_before_normalization_staging_provision_hook(
    slot_path: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    BEFORE_NORMALIZATION_STAGING_PROVISION_HOOK
        .lock()
        .expect("normalization staging test hook should not be poisoned")
        .push((slot_path.to_path_buf(), Box::new(hook)));
}

#[cfg(test)]
fn run_before_normalization_quarantine_hook(slot_path: &Path) {
    let hook = {
        let mut guard = BEFORE_NORMALIZATION_QUARANTINE_HOOK
            .lock()
            .expect("normalization quarantine test hook should not be poisoned");
        guard
            .iter()
            .position(|(expected_path, _)| worktree_paths_match(expected_path, slot_path))
            .map(|position| guard.swap_remove(position).1)
    };
    if let Some(hook) = hook {
        hook(slot_path);
    }
}

#[cfg(test)]
fn run_after_normalization_quarantine_move_hook(slot_path: &Path) {
    let hook = {
        let mut guard = AFTER_NORMALIZATION_QUARANTINE_MOVE_HOOK
            .lock()
            .expect("normalization post-move test hook should not be poisoned");
        guard
            .iter()
            .position(|(expected_path, _)| expected_path == slot_path)
            .map(|position| guard.swap_remove(position).1)
    };
    if let Some(hook) = hook {
        hook(slot_path);
    }
}

#[cfg(test)]
fn run_before_normalization_atomic_rename_hook(destination: &Path) {
    let hook = {
        let mut guard = BEFORE_NORMALIZATION_ATOMIC_RENAME_HOOK
            .lock()
            .expect("normalization atomic-rename test hook should not be poisoned");
        guard
            .iter()
            .position(|(expected_path, _)| expected_path == destination)
            .map(|position| guard.swap_remove(position).1)
    };
    if let Some(hook) = hook {
        hook(destination);
    }
}

#[cfg(test)]
fn run_before_normalization_staging_provision_hook(slot_path: &Path, replacement_path: &Path) {
    let hook = {
        let mut guard = BEFORE_NORMALIZATION_STAGING_PROVISION_HOOK
            .lock()
            .expect("normalization staging test hook should not be poisoned");
        guard
            .iter()
            .position(|(expected_path, _)| worktree_paths_match(expected_path, slot_path))
            .map(|position| guard.swap_remove(position).1)
    };
    if let Some(hook) = hook {
        hook(replacement_path);
    }
}

#[cfg(not(test))]
fn run_before_normalization_quarantine_hook(_slot_path: &Path) {}

#[cfg(not(test))]
fn run_after_normalization_quarantine_move_hook(_slot_path: &Path) {}

#[cfg(not(test))]
fn run_before_normalization_atomic_rename_hook(_destination: &Path) {}

#[cfg(not(test))]
fn run_before_normalization_staging_provision_hook(_slot_path: &Path, _replacement_path: &Path) {}
