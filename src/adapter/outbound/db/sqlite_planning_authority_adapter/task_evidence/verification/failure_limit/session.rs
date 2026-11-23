use crate::domain::parallel_mode::ParallelModeAgentSessionDetailSnapshot;

use super::super::super::super::continuation;
use super::*;

pub(super) fn preserve(
    transaction: &Transaction<'_>,
    expected: &ParallelModeSlotLeaseSnapshot,
    at: &str,
) -> Result<()> {
    // Include JSON aliases as well as the indexed key. A second row presenting
    // the same session cannot become an ambiguous continuation target.
    let session_key = expected.session_key();
    let mut statement = transaction.prepare(
        "SELECT session_key,slot_id,updated_at,content FROM runtime_session_details
         WHERE session_key=?1 OR json_extract(content,'$.session_key')=?1 LIMIT 2",
    )?;
    let rows = statement
        .query_map([&session_key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let [(row_key, row_slot, row_updated, json)] = rows.as_slice() else {
        return Err(TaskEvidenceError::Unavailable.into());
    };
    let mut detail: ParallelModeAgentSessionDetailSnapshot = serde_json::from_str(json)?;
    if expected.state != ParallelModeSlotLeaseState::Running
        || row_key != &session_key
        || row_slot != &expected.slot_id
        || row_updated != &detail.updated_at
        || detail.session_key != session_key
        || detail.slot_id != expected.slot_id
        || detail.task_id != expected.task_id
        || detail.agent_id != expected.agent_id
        || detail.worktree_path != expected.worktree_path
        || detail.branch_name != expected.branch_name
        || detail.lease_started_at != expected.leased_at
    {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    if detail.thread_id.as_ref().is_none_or(|thread| {
        thread.is_empty()
            || thread.len() > 1024
            || thread
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
    }) {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    if detail.state_label != "awaiting_user" || detail.completion_state_label != "awaiting_user" {
        continuation::mark_awaiting_user_detail(&mut detail, at);
        continuation::write_detail(transaction, &detail)?;
    }
    Ok(())
}
