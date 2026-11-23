//! The evidence ledger shares the planning authority transaction, never its TaskDefinition JSON.
//! Receipts and event outbox survive acceptance changes; only a current matching scope applies.
use std::collections::BTreeSet;

use anyhow::Result;
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitDecision, PlanningWorkPermitStage,
};
use crate::application::port::outbound::task_evidence_repository_port::*;
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use crate::domain::planning::evidence::*;
use crate::domain::planning::{TaskDefinition, TaskStatus};

use super::{
    SqlitePlanningAuthorityAdapter, open_authority_connection, read_metadata_i64_connection,
    table_exists,
};

mod queries;
mod schema;
mod writes;
pub(super) use schema::{retire_all_scopes, retire_task_scope, sync_task_scope};

pub(super) fn ensure_tables(connection: &mut Connection) -> Result<()> {
    schema::ensure(connection)
}

fn connection(workspace: &str) -> Result<Connection> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    open_authority_connection(&location)
}

fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| TaskEvidenceError::Unavailable)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn bump_revision(transaction: &Transaction<'_>) -> Result<u64> {
    let revision: u64 = transaction.query_row(
        "SELECT revision FROM task_evidence_state WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    let next = revision
        .checked_add(1)
        .filter(|value| *value <= MAX_TASK_EVIDENCE_REVISION)
        .ok_or(TaskEvidenceError::Limit)?;
    transaction.execute(
        "UPDATE task_evidence_state SET revision=?1 WHERE singleton=1",
        [next as i64],
    )?;
    Ok(next)
}

fn read_scope(connection: &Connection, task_id: &str) -> Result<Option<TaskEvidenceScope>> {
    let scope = connection.query_row(
        "SELECT task_id,scope_id,acceptance_revision,criteria_sha256 FROM task_evidence_scopes WHERE task_id=?1",
        [task_id], |row| Ok(TaskEvidenceScope {
            task_id: row.get(0)?, scope_id: row.get(1)?, acceptance_revision: row.get(2)?, criteria_sha256: row.get(3)?,
        })).optional()?;
    if let Some(scope) = &scope {
        scope.validate()?;
    }
    Ok(scope)
}

fn current_task(connection: &Connection, task_id: &str) -> Result<TaskDefinition> {
    let json: Option<String> = connection
        .query_row(
            "SELECT content_json FROM planning_tasks WHERE task_id=?1",
            [task_id],
            |row| row.get(0),
        )
        .optional()?;
    let task: TaskDefinition = serde_json::from_str(&json.ok_or(TaskEvidenceError::Missing)?)?;
    if task.id != task_id {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(task)
}

fn require_current_scope(
    connection: &Connection,
    expected: &TaskEvidenceScope,
    criterion: &str,
) -> Result<TaskDefinition> {
    expected.validate()?;
    validate_evidence_identifier(criterion)?;
    let task = current_task(connection, &expected.task_id)?;
    let current =
        read_scope(connection, &expected.task_id)?.ok_or(TaskEvidenceError::ScopeChanged)?;
    if current != *expected
        || task.acceptance_revision != expected.acceptance_revision
        || criteria_sha256(&task.acceptance)? != expected.criteria_sha256
    {
        return Err(TaskEvidenceError::ScopeChanged.into());
    }
    if !task
        .acceptance
        .iter()
        .any(|value| value.id.trim() == criterion)
    {
        return Err(TaskEvidenceError::Missing.into());
    }
    Ok(task)
}

fn require_running_lease(
    transaction: &Transaction<'_>,
    task: &TaskDefinition,
    expected: &ParallelModeSlotLeaseSnapshot,
) -> Result<()> {
    let generation = expected
        .lease_generation
        .as_deref()
        .ok_or(TaskEvidenceError::LeaseChanged)?;
    validate_evidence_token(generation).map_err(|_| TaskEvidenceError::LeaseChanged)?;
    if task.id != expected.task_id
        || expected.state != ParallelModeSlotLeaseState::Running
        || !matches!(task.status, TaskStatus::Ready | TaskStatus::InProgress)
    {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    let json: Option<String> = transaction
        .query_row(
            "SELECT content FROM runtime_slot_leases WHERE slot_id=?1",
            [&expected.slot_id],
            |row| row.get(0),
        )
        .optional()?;
    let current: ParallelModeSlotLeaseSnapshot =
        serde_json::from_str(&json.ok_or(TaskEvidenceError::LeaseChanged)?)?;
    if current.state != ParallelModeSlotLeaseState::Running || !current.same_generation_as(expected)
    {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    let owners: i64 = transaction.query_row(
        "SELECT count(*) FROM runtime_slot_leases WHERE json_extract(content,'$.task_id')=?1",
        [&task.id],
        |row| row.get(0),
    )?;
    if owners != 1 || super::continuation::task_preserved_in_connection(transaction, &task.id)? {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    if matches!(
        super::work_orders::check_work_order_permit_in_transaction(
            transaction,
            &task.id,
            PlanningWorkPermitStage::Dispatch,
            None
        )?,
        PlanningWorkPermitDecision::Denied(_)
    ) {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    Ok(())
}

fn insert_event(transaction: &Transaction<'_>, event: &TaskEvidenceEvent) -> Result<()> {
    event.validate()?;
    transaction.execute("INSERT INTO task_evidence_events(event_id,evidence_revision,payload_json,acknowledged) VALUES (?1,?2,?3,0)",
        params![event.event_id(), event.evidence_revision() as i64, serde_json::to_string(event)?])?;
    Ok(())
}

fn read_attempt(
    connection: &Connection,
    attempt_id: &str,
) -> Result<Option<StoredTaskEvidenceAttempt>> {
    let row: Option<(String,String,Option<String>)> = connection.query_row(
        "SELECT attempt_json,lease_json,receipt_json FROM task_evidence_attempts WHERE attempt_id=?1", [attempt_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    row.map(|(attempt, lease, receipt)| {
        let attempt: TaskEvidenceAttempt = serde_json::from_str(&attempt)?;
        attempt.validate()?;
        if attempt.attempt_id != attempt_id {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        let lease: ParallelModeSlotLeaseSnapshot = serde_json::from_str(&lease)?;
        if lease.task_id != attempt.scope.task_id
            || lease.slot_id != attempt.slot_id
            || lease.lease_generation.as_deref() != Some(attempt.lease_generation.as_str())
            || lease.session_key() != attempt.session_key
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        let receipt = receipt
            .map(|json| serde_json::from_str::<TaskEvidenceReceipt>(&json))
            .transpose()?;
        if let Some(receipt) = &receipt {
            receipt.validate()?;
            if receipt.evidence.attempt != attempt {
                return Err(TaskEvidenceError::Unavailable.into());
            }
        }
        Ok(StoredTaskEvidenceAttempt {
            attempt,
            lease,
            receipt,
        })
    })
    .transpose()
}
