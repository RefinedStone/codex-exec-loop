use std::fs;
use std::path::{Path, PathBuf};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;

use super::super::ensure_directory_exists;

fn slot_leases_root(pool_root: &Path) -> PathBuf {
    pool_root.join(".leases")
}

pub(in crate::application::service::parallel_mode) fn slot_lease_file_path(
    pool_root: &Path,
    slot_id: &str,
) -> PathBuf {
    slot_leases_root(pool_root).join(format!("{slot_id}.json"))
}

pub(in crate::application::service::parallel_mode) fn write_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<(), String> {
    planning_authority
        .upsert_runtime_slot_lease(workspace_dir, lease)
        .map_err(|error| format!("failed to store slot lease `{}`: {error}", lease.slot_id))?;

    let leases_root = slot_leases_root(pool_root);
    ensure_directory_exists(&leases_root)
        .map_err(|error| format!("failed to create lease directory: {error}"))?;
    let lease_path = slot_lease_file_path(pool_root, &lease.slot_id);
    let temp_path = lease_path.with_extension("tmp");
    let lease_body = serde_json::to_string_pretty(lease)
        .map_err(|error| format!("failed to serialize slot lease: {error}"))?;
    fs::write(&temp_path, lease_body).map_err(|error| {
        format!(
            "failed to write temporary slot lease `{}`: {error}",
            lease.slot_id
        )
    })?;
    fs::rename(&temp_path, &lease_path)
        .map_err(|error| format!("failed to persist slot lease `{}`: {error}", lease.slot_id))
}

pub(in crate::application::service::parallel_mode) fn remove_slot_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    slot_id: &str,
) -> bool {
    if planning_authority
        .remove_runtime_slot_lease(workspace_dir, slot_id)
        .is_err()
    {
        return false;
    }
    let lease_path = slot_lease_file_path(pool_root, slot_id);
    !lease_path.exists() || fs::remove_file(lease_path).is_ok()
}
