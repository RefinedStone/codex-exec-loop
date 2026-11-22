//! 자율 수준 intent가 새로 예약한 제어만 복구에서 재사용한다.
use super::{IssueAutonomyIntent, SqliteIssueAutomationStoreAdapter, read, transaction};
use crate::domain::issue_automation::MAX_ISSUE_AUTOMATION_REVISION;
use crate::domain::issue_automation::autonomy::ISSUE_AUTONOMY_CONTROL_PREFIX;
use crate::domain::issue_automation::coordinator::IssueWorkCheckpoint;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionRequest, StoredIssueIntervention,
};
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

pub(super) fn claim(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueAutonomyIntent,
    work: &IssueWorkCheckpoint,
) -> Result<StoredIssueIntervention> {
    intent.validate().map_err(|reason| anyhow!(reason))?;
    work.validate().map_err(|reason| anyhow!(reason))?;
    super::super::validate_reference_scope(workspace, &work.order.issue)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let selection = read(tx, workspace)?;
            if selection.pending.as_ref() != Some(intent)
                || selection.revision != intent.expected_revision
            {
                bail!("the autonomy fence has no matching pending intent");
            }
            let current = super::super::coordinator::read(tx, workspace, &work.order.issue)?
                .ok_or_else(|| anyhow!("the autonomy fence has no registered work"))?;
            if current.order.work_order_id != work.order.work_order_id {
                bail!("the autonomy fence belongs to another work");
            }
            if let Some(owned) = read_owned(tx, workspace, intent, work)? {
                return Ok(owned);
            }
            // A historical public intervention may already use the deterministic prefix.
            // Never adopt it: reserve a new row and its ownership link in this transaction.
            let base = format!(
                "{}{:x}",
                ISSUE_AUTONOMY_CONTROL_PREFIX,
                Sha256::digest(serde_json::to_vec(&(
                    &intent.command_id,
                    &work.order.work_order_id,
                ))?),
            );
            let mut collision = 0_u64;
            let command_id = loop {
                let candidate = if collision == 0 {
                    base.clone()
                } else {
                    format!("{base}-{collision}")
                };
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM issue_intervention_commands
                     WHERE workspace_dir=?1 AND command_id=?2)",
                    params![workspace, candidate],
                    |row| row.get(0),
                )?;
                if !exists {
                    break candidate;
                }
                collision = collision
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("the autonomy fence identifier is exhausted"))?;
            };
            let request = IssueInterventionRequest {
                command_id,
                issue: work.order.issue.clone(),
                action: IssueInterventionAction::Retriage,
            };
            let stored = super::super::intervention::begin_in_transaction(tx, workspace, &request)?;
            if stored.receipt.is_some() {
                bail!("a newly reserved autonomy fence already has a receipt");
            }
            tx.execute(
                "INSERT INTO issue_autonomy_work_fences
                 (workspace_dir,autonomy_command_id,work_order_id,control_command_id)
                 VALUES (?1,?2,?3,?4)",
                params![
                    workspace,
                    intent.command_id,
                    work.order.work_order_id,
                    request.command_id
                ],
            )?;
            Ok(stored)
        })
    })
}

fn read_owned(
    connection: &Connection,
    workspace: &str,
    intent: &IssueAutonomyIntent,
    work: &IssueWorkCheckpoint,
) -> Result<Option<StoredIssueIntervention>> {
    let id: Option<String> = connection
        .query_row(
            "SELECT control_command_id FROM issue_autonomy_work_fences
         WHERE workspace_dir=?1 AND autonomy_command_id=?2 AND work_order_id=?3",
            params![workspace, intent.command_id, work.order.work_order_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(None);
    };
    if !id.starts_with(ISSUE_AUTONOMY_CONTROL_PREFIX) {
        bail!("the owned autonomy fence identifier is invalid");
    }
    let stored = super::super::intervention::read(connection, workspace, &id)?
        .ok_or_else(|| anyhow!("the owned autonomy fence is missing"))?;
    let expected = IssueInterventionRequest {
        command_id: id,
        issue: work.order.issue.clone(),
        action: IssueInterventionAction::Retriage,
    };
    if stored.request != expected
        || stored.receipt.as_ref().is_some_and(|receipt| {
            receipt.command_id != expected.command_id
                || receipt.issue != expected.issue
                || receipt.action != expected.action.label()
                || receipt.work_order_id.as_deref() != Some(work.order.work_order_id.as_str())
                || receipt.authorization_epoch > MAX_ISSUE_AUTOMATION_REVISION
                || receipt.delivery.is_some()
                || receipt.handoff.is_some()
        })
    {
        bail!("the owned autonomy fence does not match its intent");
    }
    Ok(Some(stored))
}

pub(super) fn require_completed(
    connection: &Connection,
    workspace: &str,
    intent: &IssueAutonomyIntent,
) -> Result<()> {
    let missing: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM issue_work_checkpoints AS work
            WHERE work.workspace_dir=?1 AND NOT EXISTS (
                SELECT 1 FROM issue_autonomy_work_fences AS fence
                JOIN issue_intervention_commands AS control
                  ON control.workspace_dir=fence.workspace_dir
                 AND control.command_id=fence.control_command_id
                WHERE fence.workspace_dir=work.workspace_dir
                  AND fence.work_order_id=work.work_order_id
                  AND fence.autonomy_command_id=?2
                  AND control.receipt_json IS NOT NULL
            )
        )",
        params![workspace, intent.command_id],
        |row| row.get(0),
    )?;
    if missing {
        bail!("every registered work must complete its owned autonomy fence");
    }
    Ok(())
}
