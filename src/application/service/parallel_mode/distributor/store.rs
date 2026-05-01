use std::fs;
use std::path::{Path, PathBuf};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot};

use super::super::{
    current_timestamp, ensure_directory_exists, record_distributor_failed_session_detail,
};
use super::ParallelModeDistributorQueueRecord;
use super::queue_keys::sanitize_runtime_record_key;

fn distributor_queue_root(pool_root: &Path) -> PathBuf {
    pool_root.join(".distributor-queue")
}

fn distributor_queue_record_path(pool_root: &Path, queue_item_id: &str) -> PathBuf {
    distributor_queue_root(pool_root).join(format!("{queue_item_id}.json"))
}

pub(super) fn distributor_queue_item_id(
    lease: &ParallelModeSlotLeaseSnapshot,
    timestamp: &str,
) -> String {
    sanitize_runtime_record_key(&format!(
        "{}-{}-{}",
        lease.slot_id, lease.agent_id, timestamp
    ))
}

pub(super) fn queue_order_key_from_timestamp(timestamp: &str) -> u64 {
    timestamp
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .take(20)
        .collect::<String>()
        .parse::<u64>()
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn load_distributor_queue_records(
    pool_root: &Path,
) -> Vec<ParallelModeDistributorQueueRecord> {
    let queue_root = distributor_queue_root(pool_root);
    let Ok(entries) = fs::read_dir(queue_root) else {
        return Vec::new();
    };

    let mut records = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|content| {
            serde_json::from_str::<ParallelModeDistributorQueueRecord>(&content).ok()
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        left.enqueued_at
            .cmp(&right.enqueued_at)
            .then_with(|| left.queue_item_id.cmp(&right.queue_item_id))
    });
    records
}

pub(super) fn write_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    record: &ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    planning_authority
        .upsert_runtime_distributor_queue_record(workspace_dir, record)
        .map_err(|error| {
            format!(
                "failed to store distributor queue record `{}`: {error}",
                record.queue_item_id
            )
        })?;

    let queue_root = distributor_queue_root(pool_root);
    ensure_directory_exists(&queue_root)
        .map_err(|error| format!("failed to create distributor queue directory: {error}"))?;

    let path = distributor_queue_record_path(pool_root, &record.queue_item_id);
    let temp_path = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(record)
        .map_err(|error| format!("failed to serialize distributor queue record: {error}"))?;
    fs::write(&temp_path, body).map_err(|error| {
        format!(
            "failed to write temporary distributor queue record `{}`: {error}",
            record.queue_item_id
        )
    })?;
    fs::rename(&temp_path, &path).map_err(|error| {
        format!(
            "failed to persist distributor queue record `{}`: {error}",
            record.queue_item_id
        )
    })
}

pub(super) fn block_distributor_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
    failure_detail: String,
) -> Result<String, String> {
    record.queue_state = ParallelModeQueueItemState::Blocked;
    record.integration_state = "blocked".to_string();
    if record.recovery_note.is_none() {
        record.recovery_note = Some(failure_detail.clone());
    }
    record.integration_note = failure_detail.clone();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, workspace_dir, pool_root, record)?;
    if let Some(lease) = lease {
        let _ = record_distributor_failed_session_detail(
            planning_authority,
            workspace_dir,
            pool_root,
            lease,
            &failure_detail,
        );
    }

    Ok(format!(
        "distributor queue head blocked / agent: {} / {}",
        record.agent_id, failure_detail
    ))
}
