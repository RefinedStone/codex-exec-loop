use anyhow::{Context, Result, bail};
use rusqlite::params;

use crate::domain::issue_automation::timeline::ISSUE_TIMELINE_READ_LIMIT;
use crate::domain::kernel_event::issue_event_matches;
use crate::domain::issue_work::IssueReference;
use crate::domain::kernel_event::{IssueEventReference, KernelEvent, RecordedKernelEvent};

use super::super::SqlitePlanningAuthorityAdapter;

pub(super) mod read_only;

pub(super) fn load_recent(
    workspace_dir: &str,
    issue: &IssueReference,
    limit: usize,
) -> Result<Option<Vec<RecordedKernelEvent>>> {
    let Some(location) =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_for_read(workspace_dir)?
    else {
        return Ok(None);
    };
    let Some(store) = read_only::open(&location)? else {
        return Ok(Some(Vec::new()));
    };
    let connection = &store.connection;
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='kernel_events')",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(Some(Vec::new()));
    }

    let reference = IssueEventReference::from(issue);
    let provider = serde_json::to_value(reference.provider)?
        .as_str()
        .context("issue event provider is not a string")?
        .to_owned();
    let limit = limit.clamp(1, ISSUE_TIMELINE_READ_LIMIT) as i64;
    // 이슈를 먼저 고른 뒤 최신 21개를 제한한다. 결정 기록의 중첩 좌표도 같은 조건을 따른다.
    let mut statement = connection
        .prepare(
            "WITH scoped AS (
            SELECT sequence, recorded_at, payload_json,
                CASE
                    WHEN kind IN ('issue_decision_requested', 'issue_decision_resolved', 'issue_decision_reminded')
                    THEN json_extract(payload_json, '$.decision.subject.issue')
                    ELSE json_extract(payload_json, '$.issue')
                END AS issue_ref
            FROM kernel_events
            WHERE kind IN (
                'issue_tracked', 'issue_triaged', 'issue_decision_requested',
                'issue_decision_resolved', 'issue_decision_reminded', 'issue_work_submitted', 'issue_held',
                'issue_released', 'issue_work_permit_revoked', 'issue_work_permit_granted',
                'issue_execution_progressed', 'issue_integrated', 'issue_closed',
                'issue_feedback_posted'
            )
        )
        SELECT sequence, recorded_at, payload_json FROM scoped
        WHERE json_extract(issue_ref, '$.provider') = ?1
          AND json_extract(issue_ref, '$.server_id') = ?2
          AND json_extract(issue_ref, '$.repository_id') = ?3
          AND json_extract(issue_ref, '$.issue_id') = ?4
          AND json_extract(issue_ref, '$.project_scope.workspace_dir') = ?5
        ORDER BY sequence DESC LIMIT ?6",
        )
        .context("failed to prepare recent issue timeline read")?;
    let mut rows = statement.query(params![
        provider,
        reference.server_id,
        reference.repository_id,
        reference.issue_id,
        reference.project_scope.workspace_dir,
        limit,
    ])?;
    let mut records = Vec::new();
    while let Some(row) = rows.next()? {
        let payload: String = row.get(2)?;
        let event: KernelEvent =
            serde_json::from_str(&payload).context("failed to decode an issue timeline record")?;
        if !issue_event_matches(&event, issue) {
            bail!("an issue timeline record does not match the requested scope");
        }
        records.push(RecordedKernelEvent {
            sequence: row.get(0)?,
            recorded_at: row.get(1)?,
            event,
        });
    }
    records.reverse();
    Ok(Some(records))
}

#[cfg(test)]
mod tests;
