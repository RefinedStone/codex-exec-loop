//! 작업 사실 outbox는 checkpoint 변경과 같은 transaction에서 생성한다.
use super::{SqliteIssueAutomationStoreAdapter, transaction};
use crate::domain::issue_automation::coordinator::{
    IssueWorkCheckpoint, IssueWorkEvent, IssueWorkEventKind,
};
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, params};

pub(super) const TABLE: &str = "issue_work_events";
pub(super) const SCHEMA: &str = r#"
CREATE TABLE issue_work_events (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    event_id TEXT COLLATE BINARY NOT NULL,
    event TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    acknowledged INTEGER NOT NULL CHECK (acknowledged IN (0, 1)),
    PRIMARY KEY (workspace_dir, event_id)
) STRICT, WITHOUT ROWID
"#;

pub(super) fn append(
    connection: &Connection,
    workspace: &str,
    old: Option<&IssueWorkCheckpoint>,
    work: &IssueWorkCheckpoint,
    revision: u64,
) -> Result<()> {
    let mut kinds = Vec::new();
    if !work.task_ids.is_empty() && old.is_none_or(|old| old.task_ids.is_empty()) {
        kinds.push(IssueWorkEventKind::Submitted {
            task_ids: work.task_ids.clone(),
        });
    }
    if let Some(source_sha) = &work.permit_source
        && old.is_none_or(|old| {
            old.permit_source != work.permit_source
                || old.permit_revision != work.permit_revision
                || old.delivery_granted != work.delivery_granted
        })
    {
        kinds.push(IssueWorkEventKind::PermitGranted {
            source_sha: source_sha.clone(),
            gate_revision: work.permit_revision,
            delivery: work.delivery_granted,
        });
    }
    for kind in kinds {
        append_fact(connection, workspace, work, kind, revision)?;
    }
    Ok(())
}

/// 호출자가 work/관찰 저장과 같은 트랜잭션 안에서 확정한 사실을 기록한다.
pub(super) fn append_fact(
    connection: &Connection,
    workspace: &str,
    work: &IssueWorkCheckpoint,
    kind: IssueWorkEventKind,
    revision: u64,
) -> Result<()> {
    let event = IssueWorkEvent::new(
        work.order.issue.clone(),
        work.order.work_order_id.clone(),
        kind,
        revision,
    );
    connection.execute(
        "INSERT INTO issue_work_events
         (workspace_dir,event_id,event,revision,acknowledged) VALUES (?1,?2,?3,?4,0)",
        params![
            workspace,
            event.event_id,
            serde_json::to_string(&event)?,
            super::sql_revision(revision)?
        ],
    )?;
    Ok(())
}

pub(super) fn pending(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
) -> Result<Vec<IssueWorkEvent>> {
    adapter.with_connection(workspace, |connection, _| {
        let mut statement = connection.prepare(
            "SELECT event_id,event FROM issue_work_events
            WHERE workspace_dir=?1 AND acknowledged=0 ORDER BY revision, event_id LIMIT 100",
        )?;
        let mut rows = statement.query([workspace])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let encoded: String = row.get(1)?;
            if encoded.len() > 64 * 1024 {
                bail!("the issue event exceeds its bound");
            }
            let event: IssueWorkEvent =
                serde_json::from_str(&encoded).map_err(|_| anyhow!("invalid issue work event"))?;
            if event.issue.project_scope.workspace_dir != workspace
                || event.event_id != row.get::<_, String>(0)?
                || event.event_id
                    != IssueWorkEvent::new(
                        event.issue.clone(),
                        event.work_order_id.clone(),
                        event.kind.clone(),
                        event.revision,
                    )
                    .event_id
            {
                bail!("the issue event identity does not match");
            }
            events.push(event);
        }
        Ok(events)
    })
}

pub(super) fn acknowledge(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    event_id: &str,
) -> Result<()> {
    adapter.with_connection(workspace, |connection, guard| transaction(connection, guard, |tx| {
        tx.execute("UPDATE issue_work_events SET acknowledged=1 WHERE workspace_dir=?1 AND event_id=?2",
            params![workspace,event_id])?;
        Ok(())
    }))
}
