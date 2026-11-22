//! 실행 관측의 영수증·수명·lineage와 outbox를 같은 transaction에 저장한다.
use super::{
    SqliteIssueAutomationStoreAdapter, next_revision, sql_revision, transaction,
    validate_reference_scope,
};
use crate::application::port::outbound::issue_automation_store_port::IssueExecutionSaveResult;
use crate::domain::issue_automation::coordinator::{
    IssueWorkCheckpoint, IssueWorkEvent, IssueWorkEventKind,
};
use crate::domain::issue_automation::execution::*;
use crate::domain::issue_automation::{IssueAutomationState, IssueTransition, TrackedIssue};
use crate::domain::issue_work::IssueReference;
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

pub(super) const TABLE: &str = "issue_execution_progress";
pub(super) const SCHEMA: &str = r#"
CREATE TABLE issue_execution_progress (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    work_order_id TEXT COLLATE BINARY NOT NULL,
    record TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (workspace_dir, work_order_id),
    FOREIGN KEY (workspace_dir, work_order_id)
        REFERENCES issue_work_checkpoints (workspace_dir, work_order_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

pub(super) const RECEIPT_TABLE: &str = "issue_execution_receipts";
pub(super) const RECEIPT_SCHEMA: &str = r#"
CREATE TABLE issue_execution_receipts (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    work_order_id TEXT COLLATE BINARY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('execution', 'terminal')),
    record_id TEXT COLLATE BINARY NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (workspace_dir, work_order_id, kind, record_id),
    FOREIGN KEY (workspace_dir, work_order_id)
        REFERENCES issue_work_checkpoints (workspace_dir, work_order_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

pub(super) fn get(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    issue: &IssueReference,
) -> Result<Option<IssueExecutionRecord>> {
    validate_reference_scope(workspace, issue)?;
    adapter.with_connection(workspace, |connection, _| {
        let Some(work) = super::coordinator::read(connection, workspace, issue)? else {
            return Ok(None);
        };
        read(connection, workspace, &work.order.work_order_id)
    })
}

pub(super) fn read(
    connection: &Connection,
    workspace: &str,
    work_order_id: &str,
) -> Result<Option<IssueExecutionRecord>> {
    let row: Option<(String, i64)> = connection
        .query_row(
            "SELECT record, revision FROM issue_execution_progress
             WHERE workspace_dir=?1 AND work_order_id=?2",
            params![workspace, work_order_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((encoded, revision)) = row else {
        return Ok(None);
    };
    if encoded.len() > 128 * 1024 {
        bail!("the execution progress exceeds its bound");
    }
    let record: IssueExecutionRecord = serde_json::from_str(&encoded)
        .map_err(|_| anyhow!("invalid persisted issue execution progress"))?;
    record.validate().map_err(|reason| anyhow!(reason))?;
    if revision < 1
        || record.progress.revision != u64::try_from(revision)?
        || record.work_order_id != work_order_id
    {
        bail!("the execution progress identity does not match");
    }
    Ok(Some(record))
}

pub(super) fn apply(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected_work: &IssueWorkCheckpoint,
    expected_progress_revision: u64,
    observation: &IssueWorkExecutionObservation,
    max_attempts: Option<u32>,
) -> Result<IssueExecutionSaveResult> {
    expected_work.validate().map_err(|reason| anyhow!(reason))?;
    observation.validate().map_err(|reason| anyhow!(reason))?;
    validate_reference_scope(workspace, &expected_work.order.issue)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(work) = super::coordinator::read(tx, workspace, &expected_work.order.issue)?
            else {
                return Ok(IssueExecutionSaveResult::Conflict);
            };
            if &work != expected_work {
                return Ok(IssueExecutionSaveResult::Conflict);
            }
            // Only append executor-confirmed membership. Ordinary save_issue_work remains immutable.
            if work
                .task_ids
                .iter()
                .any(|task| !observation.task_ids.contains(task))
            {
                bail!("an execution observation cannot remove the work lineage");
            }
            let previous = read(tx, workspace, &work.order.work_order_id)?;
            let old = previous
                .clone()
                .unwrap_or_else(|| IssueExecutionRecord::new(work.order.work_order_id.clone()));
            if old.progress.revision != expected_progress_revision {
                return Ok(IssueExecutionSaveResult::Conflict);
            }
            let mut execution_count: u32 = 0;
            for id in &observation.execution_ids {
                if insert_receipt(
                    tx,
                    workspace,
                    &work.order.work_order_id,
                    "execution",
                    id,
                    id,
                )? {
                    execution_count = execution_count
                        .checked_add(1)
                        .ok_or_else(|| anyhow!("execution receipt count overflow"))?;
                }
            }
            let mut verification_count: u32 = 0;
            for terminal in &observation.terminals {
                let first = insert_receipt(
                    tx,
                    workspace,
                    &work.order.work_order_id,
                    "terminal",
                    &terminal.record_id,
                    &serde_json::to_string(terminal)?,
                )?;
                if first && terminal.classification.needs_verification_attempt() {
                    verification_count = verification_count
                        .checked_add(1)
                        .ok_or_else(|| anyhow!("verification receipt count overflow"))?;
                }
            }
            let mut next = IssueExecutionRecord {
                work_order_id: work.order.work_order_id.clone(),
                progress: old
                    .progress
                    .reconcile_with_budget(
                        observation,
                        execution_count,
                        verification_count,
                        max_attempts,
                    )
                    .map_err(|reason| anyhow!(reason))?,
                question: observation.question.clone(),
            };
            let questions_changed = old.question != next.question
                && super::decision::supersede_work_questions(
                    tx,
                    workspace,
                    &work.order.issue,
                    &work.order.work_order_id,
                )?;
            let lineage_changed = work.task_ids != observation.task_ids;
            let tracked_changed = project_tracked(tx, workspace, &work, &old, &next)?;
            let changed = previous.is_none()
                || next != old
                || lineage_changed
                || tracked_changed
                || questions_changed;
            if !changed {
                return Ok(IssueExecutionSaveResult::Saved {
                    record: Box::new(old),
                    catalog_revision: super::triage::catalog_revision(tx, workspace)?,
                    changed: false,
                });
            }
            next.progress.revision = next_revision(old.progress.revision)?;
            next.validate().map_err(|reason| anyhow!(reason))?;
            let encoded = serde_json::to_string(&next)?;
            if encoded.len() > 128 * 1024 {
                bail!("the execution progress exceeds its bound");
            }
            tx.execute(
                "INSERT INTO issue_execution_progress
                 (workspace_dir, work_order_id, record, revision) VALUES (?1,?2,?3,?4)
                 ON CONFLICT(workspace_dir,work_order_id)
                 DO UPDATE SET record=excluded.record, revision=excluded.revision",
                params![
                    workspace,
                    next.work_order_id,
                    encoded,
                    sql_revision(next.progress.revision)?
                ],
            )?;
            if lineage_changed {
                let mut linked = work.clone();
                linked.task_ids = observation.task_ids.clone();
                linked.delivery_granted = false;
                linked.revision = next_revision(work.revision)?;
                linked.validate().map_err(|reason| anyhow!(reason))?;
                let encoded = serde_json::to_string(&linked)?;
                if encoded.len() > 512 * 1024 {
                    bail!("the issue work checkpoint exceeds its bound");
                }
                let affected = tx.execute(
                    "UPDATE issue_work_checkpoints SET checkpoint=?1, revision=?2
                     WHERE workspace_dir=?3 AND work_order_id=?4 AND revision=?5",
                    params![
                        encoded,
                        sql_revision(linked.revision)?,
                        workspace,
                        linked.order.work_order_id,
                        sql_revision(work.revision)?
                    ],
                )?;
                if affected != 1 {
                    bail!("the work lineage changed during its transaction");
                }
            }
            let revision = super::triage::bump_catalog(tx, workspace)?;
            // Registration may commit before its ACK reaches the issue checkpoint.
            // Recover the first submission fact atomically with the observed lineage.
            if work.task_ids.is_empty() && !observation.task_ids.is_empty() {
                super::events::append_fact(
                    tx,
                    workspace,
                    &work,
                    IssueWorkEventKind::Submitted {
                        task_ids: observation.task_ids.clone(),
                    },
                    revision,
                )?;
            }
            append_fact(
                tx,
                workspace,
                &work,
                IssueWorkEventKind::ExecutionProgressed {
                    progress: next.progress.clone(),
                },
                revision,
            )?;
            if old.progress.integration.is_none()
                && let Some(integration) = &next.progress.integration
            {
                append_fact(
                    tx,
                    workspace,
                    &work,
                    IssueWorkEventKind::Integrated {
                        integration: integration.clone(),
                    },
                    revision,
                )?;
            }
            Ok(IssueExecutionSaveResult::Saved {
                record: Box::new(next),
                catalog_revision: revision,
                changed: true,
            })
        })
    })
}

fn insert_receipt(
    connection: &Connection,
    workspace: &str,
    work_order_id: &str,
    kind: &str,
    record_id: &str,
    payload: &str,
) -> Result<bool> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT payload FROM issue_execution_receipts
             WHERE workspace_dir=?1 AND work_order_id=?2 AND kind=?3 AND record_id=?4",
            params![workspace, work_order_id, kind, record_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing) = existing {
        if existing != payload {
            bail!("a stable execution receipt changed its immutable fact");
        }
        return Ok(false);
    }
    connection.execute(
        "INSERT INTO issue_execution_receipts
         (workspace_dir,work_order_id,kind,record_id,payload) VALUES (?1,?2,?3,?4,?5)",
        params![workspace, work_order_id, kind, record_id, payload],
    )?;
    Ok(true)
}

fn append_fact(
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
            sql_revision(revision)?
        ],
    )?;
    Ok(())
}

fn project_tracked(
    tx: &Transaction<'_>,
    workspace: &str,
    work: &IssueWorkCheckpoint,
    old: &IssueExecutionRecord,
    next: &IssueExecutionRecord,
) -> Result<bool> {
    let Some(mut tracked) = super::read_issue(tx, workspace, &work.order.issue)? else {
        bail!("the execution work has no tracked issue");
    };
    use IssueAutomationState as State;
    if matches!(tracked.state(), State::Done | State::Cancelled)
        || (tracked.state() == State::Failed && next.progress.integration.is_none())
    {
        // 취소는 보존한다. 한도 소진 뒤에도 신뢰된 통합 사실은 재시도 없이 완료로 반영한다.
        return Ok(false);
    }
    let mut changed = false;
    if tracked.state() == State::AwaitingDecision
        && ((old.question.is_some() && next.question.is_none())
            || matches!(
                next.progress.state,
                IssueExecutionState::Done | IssueExecutionState::Failed
            ))
    {
        changed |= transition(tx, &mut tracked, IssueTransition::ResumeDecision)?;
    }
    if tracked.state() == State::AwaitingDecision {
        return Ok(changed);
    }
    if tracked.state() == State::Ready {
        changed |= transition(tx, &mut tracked, IssueTransition::Queue)?;
    }
    if tracked.state() == State::Queued && next.progress.state != IssueExecutionState::Queued {
        changed |= transition(tx, &mut tracked, IssueTransition::Start)?;
    }
    match next.progress.state {
        IssueExecutionState::Queued => {}
        IssueExecutionState::Running => {
            if tracked.state() == State::InReview {
                changed |= transition(tx, &mut tracked, IssueTransition::Rework)?;
            }
        }
        IssueExecutionState::InReview | IssueExecutionState::Done => {
            if tracked.state() == State::Running {
                changed |= transition(tx, &mut tracked, IssueTransition::OpenReview)?;
            }
            if next.progress.state == IssueExecutionState::Done
                && matches!(tracked.state(), State::InReview | State::Failed)
            {
                changed |= transition(tx, &mut tracked, IssueTransition::Complete)?;
            }
        }
        IssueExecutionState::Failed => {
            if matches!(tracked.state(), State::Running | State::InReview) {
                changed |= transition(tx, &mut tracked, IssueTransition::ExhaustAttempts)?;
            }
        }
    }
    if next.question.is_some() && matches!(tracked.state(), State::Running | State::InReview) {
        changed |= transition(tx, &mut tracked, IssueTransition::AwaitDecision)?;
    }
    Ok(changed)
}

fn transition(
    tx: &Transaction<'_>,
    tracked: &mut TrackedIssue,
    change: IssueTransition,
) -> Result<bool> {
    let mut candidate = tracked.clone();
    if !candidate.transition(change)? {
        return Ok(false);
    }
    tracked.validate_replacement(&candidate)?;
    let mut record = candidate.to_record();
    record.revision = next_revision(tracked.revision())?;
    super::update_issue_row(tx, tracked.revision(), &record)?;
    *tracked = TrackedIssue::hydrate(record)?;
    Ok(true)
}

#[cfg(test)]
mod tests;
