//! The first launch of each task consumes one durable work-order execution attempt.
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::{SqlitePlanningAuthorityAdapter, open_authority_connection, runtime_projection};
use super::ensure_work_order_tables;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::application::port::outbound::planning_work_order_repository_port::validate_work_order_id;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState,
};

pub(super) fn set_limit(workspace: &str, work_order_id: &str, max_attempts: u32) -> Result<()> {
    validate_work_order_id(work_order_id)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let previous = limit(&tx, work_order_id)?;
    if previous.is_none() {
        seed_existing_attempts(&tx, work_order_id)?;
    }
    if previous == Some(max_attempts) {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO planning_work_order_execution_limits (work_order_id,max_attempts) VALUES (?1,?2)
         ON CONFLICT(work_order_id) DO UPDATE SET max_attempts=excluded.max_attempts",
        params![work_order_id,max_attempts],
    )?;
    runtime_projection::append_runtime_event(
        &tx,
        "work_order_execution_limit_set",
        "work_order",
        work_order_id,
        "work-order execution limit updated",
        &serde_json::json!({"work_order_id":work_order_id,"max_attempts":max_attempts}).to_string(),
    )?;
    tx.commit()?;
    Ok(())
}

pub(super) fn started_task_ids(workspace: &str, work_order_id: &str) -> Result<Vec<String>> {
    validate_work_order_id(work_order_id)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let mut statement = connection.prepare(
        "SELECT started.task_id FROM planning_work_order_started_tasks AS started
         JOIN planning_work_order_tasks AS member ON member.task_id=started.task_id
         WHERE started.work_order_id=?1 AND member.work_order_id=?1 ORDER BY member.ordinal",
    )?;
    Ok(statement
        .query_map(params![work_order_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn limit(connection: &Connection, work_order_id: &str) -> Result<Option<u32>> {
    Ok(connection
        .query_row(
            "SELECT max_attempts FROM planning_work_order_execution_limits WHERE work_order_id=?1",
            params![work_order_id],
            |row| row.get(0),
        )
        .optional()?)
}

pub(super) fn task_attempt_allowed(
    connection: &Connection,
    work_order_id: &str,
    task_id: &str,
) -> Result<bool> {
    let started:bool=connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM planning_work_order_started_tasks WHERE work_order_id=?1 AND task_id=?2)",
        params![work_order_id,task_id], |row|row.get(0),
    )?;
    if started {
        return Ok(true);
    }
    let Some(max_attempts) = limit(connection, work_order_id)? else {
        return Ok(true);
    };
    let count: u64 = connection.query_row(
        "SELECT COUNT(*) FROM planning_work_order_started_tasks WHERE work_order_id=?1",
        params![work_order_id],
        |row| row.get(0),
    )?;
    Ok(count < u64::from(max_attempts))
}

fn insert_started(
    connection: &Connection,
    work_order_id: &str,
    task_id: &str,
    session_key: &str,
) -> Result<bool> {
    Ok(connection.execute(
        "INSERT OR IGNORE INTO planning_work_order_started_tasks
         (work_order_id,task_id,first_session_key) VALUES (?1,?2,?3)",
        params![work_order_id, task_id, session_key],
    )? == 1)
}

pub(super) fn claim_task_attempt(
    tx: &Transaction<'_>,
    work_order_id: &str,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<()> {
    if !task_attempt_allowed(tx, work_order_id, &lease.task_id)? {
        bail!("work-order execution attempt limit reached");
    }
    if insert_started(tx, work_order_id, &lease.task_id, &lease.session_key())? {
        runtime_projection::append_runtime_event(
            tx,
            "work_order_execution_started",
            "work_order",
            work_order_id,
            "first worker launch claimed for work-order task",
            &serde_json::json!({"work_order_id":work_order_id,"task_id":lease.task_id,
                "session_key":lease.session_key()})
            .to_string(),
        )?;
    }
    Ok(())
}

/// Upgrade existing work under the same IMMEDIATE transaction as the first limit installation.
fn seed_existing_attempts(tx: &Transaction<'_>, work_order_id: &str) -> Result<()> {
    let mut leases=tx.prepare(
        "SELECT lease.content FROM runtime_slot_leases AS lease
         JOIN planning_work_order_tasks AS member ON json_extract(lease.content,'$.task_id')=member.task_id
         WHERE member.work_order_id=?1",
    )?;
    let rows = leases
        .query_map(params![work_order_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(leases);
    for json in rows {
        let lease: ParallelModeSlotLeaseSnapshot = serde_json::from_str(&json)?;
        if lease.state == ParallelModeSlotLeaseState::Running {
            insert_started(tx, work_order_id, &lease.task_id, &lease.session_key())?;
        }
    }
    let mut details=tx.prepare(
        "SELECT detail.content FROM runtime_session_details AS detail
         JOIN planning_work_order_tasks AS member ON json_extract(detail.content,'$.task_id')=member.task_id
         WHERE member.work_order_id=?1",
    )?;
    let rows = details
        .query_map(params![work_order_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(details);
    for json in rows {
        let detail: ParallelModeAgentSessionDetailSnapshot = serde_json::from_str(&json)?;
        if detail
            .thread_id
            .as_ref()
            .is_some_and(|thread| !thread.trim().is_empty())
        {
            insert_started(tx, work_order_id, &detail.task_id, &detail.session_key)?;
        }
    }
    let mut queues=tx.prepare(
        "SELECT queue.content FROM runtime_distributor_queue AS queue
         JOIN planning_work_order_tasks AS member ON json_extract(queue.content,'$.task_id')=member.task_id
         WHERE member.work_order_id=?1",
    )?;
    let rows = queues
        .query_map(params![work_order_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(queues);
    for json in rows {
        let queue: PlanningAuthorityDistributorQueueRecord = serde_json::from_str(&json)?;
        insert_started(tx, work_order_id, &queue.task_id, &queue.session_key)?;
    }
    Ok(())
}
