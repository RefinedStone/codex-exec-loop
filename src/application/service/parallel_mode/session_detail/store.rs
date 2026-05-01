use std::fs;
use std::path::{Path, PathBuf};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeSlotLeaseSnapshot,
};

use super::super::ensure_directory_exists;
use super::lease_session_key;

pub(super) fn push_session_history(
    detail: &mut ParallelModeAgentSessionDetailSnapshot,
    state_label: &str,
    timestamp: String,
    summary: String,
) {
    if detail
        .history
        .last()
        .is_some_and(|entry| entry.state_label == state_label && entry.summary == summary)
    {
        return;
    }

    detail
        .history
        .push(ParallelModeAgentSessionHistoryEntry::new(
            state_label,
            timestamp,
            summary,
        ));
}

pub(super) fn update_agent_session_detail_record<F>(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    lease: &ParallelModeSlotLeaseSnapshot,
    mutate: F,
) -> Result<ParallelModeAgentSessionDetailSnapshot, String>
where
    F: FnOnce(
        Option<ParallelModeAgentSessionDetailSnapshot>,
    ) -> ParallelModeAgentSessionDetailSnapshot,
{
    let session_key = lease_session_key(lease);
    let current = read_agent_session_detail_record(pool_root, &session_key);
    let detail = mutate(current);
    write_agent_session_detail_record(planning_authority, workspace_dir, pool_root, &detail)?;
    Ok(detail)
}

pub(crate) fn read_agent_session_detail_record(
    pool_root: &Path,
    session_key: &str,
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    let path = agent_session_detail_record_path(pool_root, session_key);
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub(super) fn write_agent_session_detail_record(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    pool_root: &Path,
    detail: &ParallelModeAgentSessionDetailSnapshot,
) -> Result<(), String> {
    planning_authority
        .upsert_runtime_session_detail(workspace_dir, detail)
        .map_err(|error| {
            format!(
                "failed to store agent session detail `{}`: {error}",
                detail.session_key
            )
        })?;

    let history_dir = agent_session_history_dir(pool_root);
    ensure_directory_exists(&history_dir)
        .map_err(|error| format!("failed to create agent session history directory: {error}"))?;

    let path = agent_session_detail_record_path(pool_root, &detail.session_key);
    let temp_path = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(detail)
        .map_err(|error| format!("failed to serialize agent session detail: {error}"))?;
    fs::write(&temp_path, body).map_err(|error| {
        format!(
            "failed to write temporary agent session detail `{}`: {error}",
            detail.session_key
        )
    })?;
    fs::rename(&temp_path, &path).map_err(|error| {
        format!(
            "failed to persist agent session detail `{}`: {error}",
            detail.session_key
        )
    })
}

fn agent_session_history_dir(pool_root: &Path) -> PathBuf {
    pool_root.join(".agent-sessions")
}

pub(crate) fn agent_session_detail_record_path(pool_root: &Path, session_key: &str) -> PathBuf {
    let mut filename = String::new();
    for ch in session_key.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            filename.push(ch);
        } else {
            filename.push('_');
        }
    }

    agent_session_history_dir(pool_root).join(format!("{filename}.json"))
}
