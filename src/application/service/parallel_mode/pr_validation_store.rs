use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{PrValidationRecord, PrValidationRecordKey};

const PR_VALIDATION_MIRROR_ROOT: &str = ".pr-validation";

fn ensure_pr_validation_mirror_root(
    runtime: &dyn ParallelModeRuntimePort,
    pool_root: &Path,
) -> Result<(), String> {
    runtime
        .prepare_runtime_mirror_root(pool_root)
        .map_err(|error| format!("PR validation mirror root could not be initialized: {error}"))
}

pub(super) fn pr_validation_record_relative_path(key: &PrValidationRecordKey) -> PathBuf {
    let digest = Sha256::digest(key.as_str().as_bytes());
    PathBuf::from(PR_VALIDATION_MIRROR_ROOT).join(format!("{digest:x}.json"))
}

pub(crate) fn persist_pr_validation_record(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &Path,
    expected: Option<&PrValidationRecord>,
    replacement: &PrValidationRecord,
) -> Result<(), String> {
    if expected.is_some_and(|record| record.key() != replacement.key()) {
        return Err("PR validation persistence cannot change record identity".to_string());
    }

    ensure_pr_validation_mirror_root(runtime, pool_root)?;
    let relative = pr_validation_record_relative_path(replacement.key());
    let observed_mirror = runtime
        .read_runtime_mirror_optional(pool_root, &relative)
        .map_err(|error| {
            format!(
                "failed to inspect PR validation mirror `{}`: {error}",
                replacement.key().as_str()
            )
        })?;
    let replacement_body = serde_json::to_string_pretty(replacement)
        .map_err(|error| format!("failed to serialize PR validation record: {error}"))?;

    let authority_changed = planning_authority
        .compare_and_swap_runtime_pr_validation_record(
            workspace_dir,
            replacement.key(),
            expected,
            Some(replacement),
        )
        .map_err(|error| {
            format!(
                "failed to persist PR validation authority record `{}`: {error}",
                replacement.key().as_str()
            )
        })?;
    if !authority_changed {
        return Err(format!(
            "PR validation record `{}` changed before validation transition",
            replacement.key().as_str()
        ));
    }

    let mirror_failure = match runtime.compare_and_swap_runtime_mirror_file(
        pool_root,
        &relative,
        observed_mirror.as_deref(),
        Some(&replacement_body),
    ) {
        Ok(true) => return Ok(()),
        Ok(false) => "mirror changed after its exact snapshot was inspected".to_string(),
        Err(error) => error.to_string(),
    };

    let mirror_restored = runtime
        .compare_and_swap_runtime_mirror_file(
            pool_root,
            &relative,
            Some(&replacement_body),
            observed_mirror.as_deref(),
        )
        .unwrap_or(false)
        || runtime
            .read_runtime_mirror_optional(pool_root, &relative)
            .is_ok_and(|body| body.as_deref() == observed_mirror.as_deref());
    let authority_restored = planning_authority
        .compare_and_swap_runtime_pr_validation_record(
            workspace_dir,
            replacement.key(),
            Some(replacement),
            expected,
        )
        .unwrap_or(false);
    let rollback = match (authority_restored, mirror_restored) {
        (true, true) => "exact previous authority snapshot restored; mirror snapshot restored",
        (true, false) => {
            "exact previous authority snapshot restored; mirror was replaced or could not be restored"
        }
        (false, true) => {
            "authority was replaced or could not be restored; mirror snapshot restored"
        }
        (false, false) => {
            "replacement state preserved; neither store matched the failed transition"
        }
    };
    Err(format!(
        "failed to persist PR validation mirror `{}`: {mirror_failure}; rollback: {rollback}",
        replacement.key().as_str()
    ))
}

pub(crate) fn recover_pr_validation_record_mirror(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &Path,
    record_key: &PrValidationRecordKey,
) -> Result<Option<PrValidationRecord>, String> {
    let authoritative = planning_authority
        .load_runtime_pr_validation_record(workspace_dir, record_key)
        .map_err(|error| {
            format!(
                "failed to load PR validation authority record `{}`: {error}",
                record_key.as_str()
            )
        })?;
    ensure_pr_validation_mirror_root(runtime, pool_root)?;
    let relative = pr_validation_record_relative_path(record_key);
    let observed_mirror = runtime
        .read_runtime_mirror_optional(pool_root, &relative)
        .map_err(|error| {
            format!(
                "failed to inspect PR validation recovery mirror `{}`: {error}",
                record_key.as_str()
            )
        })?;
    let authoritative_body = authoritative
        .as_ref()
        .map(serde_json::to_string_pretty)
        .transpose()
        .map_err(|error| format!("failed to serialize PR validation recovery record: {error}"))?;

    if observed_mirror.as_deref() == authoritative_body.as_deref() {
        return Ok(authoritative);
    }
    let repaired = runtime
        .compare_and_swap_runtime_mirror_file(
            pool_root,
            &relative,
            observed_mirror.as_deref(),
            authoritative_body.as_deref(),
        )
        .map_err(|error| {
            format!(
                "failed to repair PR validation mirror `{}`: {error}",
                record_key.as_str()
            )
        })?;
    if !repaired
        && !runtime
            .read_runtime_mirror_optional(pool_root, &relative)
            .is_ok_and(|body| body.as_deref() == authoritative_body.as_deref())
    {
        return Err(format!(
            "PR validation mirror `{}` changed during authority recovery",
            record_key.as_str()
        ));
    }
    Ok(authoritative)
}
