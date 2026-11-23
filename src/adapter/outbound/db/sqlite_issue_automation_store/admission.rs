//! Atomic first-work reservations. All admitted, nonterminal checkpoints count as running,
//! including registration intents, queued work and human review. Worker threads do not.
use super::{
    SqliteIssueAutomationStoreAdapter, coordinator, transaction, validate_reference_scope,
};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueTriageFence, IssueWorkAdmissionResult, IssueWorkSaveResult,
};
use crate::domain::issue_automation::coordinator::{
    IssueStartBudgetLimit, IssueStartBudgetWait, IssueWorkCheckpoint,
};
use crate::domain::issue_automation::policy::IssueAutomationBudget;
use crate::domain::issue_automation::triage::IssueTriageAssessment;
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, Transaction, params};

mod schema;
mod waiting;
pub(super) use schema::{create_schema, schema_objects};
pub(super) use waiting::projection;

#[cfg(test)]
mod tests;

const MILLIS_PER_UTC_DAY: u64 = 86_400_000;

pub(super) fn admit(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    fence: IssueTriageFence,
    assessment: &IssueTriageAssessment,
    candidate: &IssueWorkCheckpoint,
    budget: &IssueAutomationBudget,
    now_unix_ms: u64,
) -> Result<IssueWorkAdmissionResult> {
    candidate.validate().map_err(|reason| anyhow!(reason))?;
    validate_reference_scope(workspace, &candidate.order.issue)?;
    i64::try_from(now_unix_ms)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            admit_current(
                tx,
                workspace,
                fence,
                assessment,
                candidate,
                budget,
                now_unix_ms,
            )
        })
    })
}

fn admit_current(
    tx: &Transaction<'_>,
    workspace: &str,
    fence: IssueTriageFence,
    assessment: &IssueTriageAssessment,
    candidate: &IssueWorkCheckpoint,
    budget: &IssueAutomationBudget,
    now_unix_ms: u64,
) -> Result<IssueWorkAdmissionResult> {
    let admitted_at = i64::try_from(now_unix_ms)?;
    if !coordinator::validate_current(tx, workspace, fence, assessment, candidate)? {
        return Ok(IssueWorkAdmissionResult::Conflict);
    }
    if let Some(current) = coordinator::read(tx, workspace, &candidate.order.issue)? {
        coordinator::validate_identity(&current, candidate)?;
        coordinator::validate_authorization(
            tx,
            workspace,
            assessment,
            &current.order.issue,
            current.current_start_authority(),
            current.current_authorization_decision_id(),
            current.current_authorization_basis(),
        )?;
        // ACK loss/restart/day changes replay the original reservation. Lowered budgets
        // never charge it again. Current start authority is still checked independently.
        let changed = waiting::clear(tx, workspace, &candidate.order.issue)?;
        let catalog_revision = if changed {
            super::triage::bump_catalog(tx, workspace)?
        } else {
            super::triage::catalog_revision(tx, workspace)?
        };
        return Ok(IssueWorkAdmissionResult::Saved {
            record: Box::new(current),
            catalog_revision,
            changed,
        });
    }
    if candidate.revision != 0 {
        return Ok(IssueWorkAdmissionResult::Conflict);
    }
    if candidate.authorization.is_some()
        || candidate.delivery_assessment.is_some()
        || candidate.delivery_revision != 0
        || !candidate.task_ids.is_empty()
        || candidate.permit_source.is_some()
        || candidate.permit_revision != 0
        || candidate.delivery_granted
        || candidate.permit_authorization_epoch != 0
    {
        bail!("first issue work admission must be an unsubmitted intent");
    }
    coordinator::validate_authorization(
        tx,
        workspace,
        assessment,
        &candidate.order.issue,
        candidate.start_authority,
        candidate.decision_id.as_ref(),
        &candidate.approved_basis,
    )?;
    coordinator::validate_handoff(tx, workspace, candidate)?;
    let day = current_day(tx, workspace, now_unix_ms)?;
    let exhausted = exhausted_limits(tx, workspace, budget, day)?;
    if !exhausted.is_empty() {
        let wait = IssueStartBudgetWait {
            exhausted,
            utc_day: day,
        };
        let (catalog_revision, changed) =
            waiting::save(tx, workspace, &candidate.order.issue, assessment, &wait)?;
        return Ok(IssueWorkAdmissionResult::BudgetExhausted {
            wait,
            catalog_revision,
            changed,
        });
    }
    // Checkpoint, immutable first admission receipt and wait removal share this transaction.
    // There is no external planning write before this transaction commits.
    let IssueWorkSaveResult::Saved {
        record,
        catalog_revision,
        ..
    } = coordinator::persist(tx, workspace, None, candidate)?
    else {
        unreachable!("checkpoint persistence has no competing CAS inside admission")
    };
    tx.execute(
            "INSERT INTO issue_work_admissions (workspace_dir,work_order_id,utc_day,admitted_at_unix_ms)
             VALUES (?1,?2,?3,?4)",
            params![workspace, record.order.work_order_id, i64::try_from(day)?, admitted_at],
        )?;
    waiting::clear(tx, workspace, &candidate.order.issue)?;
    Ok(IssueWorkAdmissionResult::Saved {
        record,
        catalog_revision,
        changed: true,
    })
}

fn current_day(tx: &Connection, workspace: &str, now_unix_ms: u64) -> Result<u64> {
    let latest: Option<i64> = tx.query_row(
        "SELECT max(utc_day) FROM issue_work_admissions WHERE workspace_dir=?1",
        [workspace],
        |row| row.get(0),
    )?;
    // A wall-clock rollback cannot reopen an older daily bucket.
    Ok((now_unix_ms / MILLIS_PER_UTC_DAY).max(latest.map(u64::try_from).transpose()?.unwrap_or(0)))
}

fn exhausted_limits(
    tx: &Connection,
    workspace: &str,
    budget: &IssueAutomationBudget,
    day: u64,
) -> Result<Vec<IssueStartBudgetLimit>> {
    let running: i64 = tx.query_row(
        "SELECT count(*) FROM issue_work_checkpoints w
         JOIN tracked_issues i ON i.workspace_dir=w.workspace_dir AND i.provider=w.provider
           AND i.server_id=w.server_id AND i.repository_id=w.repository_id AND i.issue_id=w.issue_id
         WHERE w.workspace_dir=?1 AND i.state NOT IN ('done','failed','cancelled')",
        [workspace],
        |row| row.get(0),
    )?;
    let started: i64 = tx.query_row(
        "SELECT count(*) FROM issue_work_admissions WHERE workspace_dir=?1 AND utc_day=?2",
        params![workspace, i64::try_from(day)?],
        |row| row.get(0),
    )?;
    let mut limits = Vec::new();
    if u64::try_from(running)? >= u64::from(budget.max_running) {
        limits.push(IssueStartBudgetLimit::Running);
    }
    if u64::try_from(started)? >= u64::from(budget.max_started_per_day) {
        limits.push(IssueStartBudgetLimit::StartedPerDay);
    }
    // Further implementation attempts are admitted by the planning execution limit.
    if budget.max_attempts == 0 {
        limits.push(IssueStartBudgetLimit::Attempts);
    }
    Ok(limits)
}
