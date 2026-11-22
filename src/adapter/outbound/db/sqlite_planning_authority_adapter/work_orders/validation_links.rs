//! Validation identity retains its original task even after runtime queue pruning.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::{SqlitePlanningAuthorityAdapter, open_authority_connection, runtime_projection};
use super::{ensure_work_order_tables, load_task_membership, load_work_order_record};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningValidationSource, PlanningWorkOrderValidationLink, validate_work_order_id,
    validate_work_order_task_id,
};

pub(super) fn read_source(
    connection: &Connection,
    key: &str,
) -> Result<Option<PlanningValidationSource>> {
    let source = connection.query_row(
        "SELECT work_order_id, source_task_id FROM planning_work_order_validation_links WHERE validation_record_key=?1",
        params![key], |row| Ok(PlanningValidationSource {
            validation_record_key: key.to_owned(),
            work_order_id: row.get(0)?,
            source_task_id: row.get(1)?,
        }),
    ).optional()?;
    if let Some(source) = &source {
        validate_work_order_task_id(&source.source_task_id)?;
        if let Some(owner) = &source.work_order_id {
            validate_work_order_id(owner)?;
            if load_task_membership(connection, &source.source_task_id)?.as_ref() != Some(owner) {
                bail!("validation source lost its immutable work-order lineage");
            }
        }
    }
    Ok(source)
}

fn as_link(source: PlanningValidationSource) -> Option<PlanningWorkOrderValidationLink> {
    source
        .work_order_id
        .map(|work_order_id| PlanningWorkOrderValidationLink {
            validation_record_key: source.validation_record_key,
            work_order_id,
            source_task_id: source.source_task_id,
        })
}

pub(super) fn link_in_transaction(
    tx: &Transaction<'_>,
    key: &str,
    source_task_id: &str,
) -> Result<PlanningValidationSource> {
    validate_work_order_id(key)?;
    validate_work_order_task_id(source_task_id)?;
    ensure_work_order_tables(tx)?;
    let work_order_id = load_task_membership(tx, source_task_id)?;
    if let Some(owner) = &work_order_id {
        load_work_order_record(tx, owner)?.context("validation source has no work-order record")?;
    }
    if let Some(existing) = read_source(tx, key)? {
        if existing.source_task_id != source_task_id {
            bail!("a validation record cannot change its source task");
        }
        if existing.work_order_id == work_order_id {
            return Ok(existing);
        }
        bail!("a validation record cannot change its work-order ownership");
    }
    let ordinal: i64 = match &work_order_id {
        Some(owner) => tx.query_row(
            "SELECT COALESCE(MAX(ordinal),-1)+1 FROM planning_work_order_validation_links WHERE work_order_id=?1",
            [owner], |row| row.get(0),
        )?,
        None => 0,
    };
    let source = PlanningValidationSource {
        validation_record_key: key.to_owned(),
        work_order_id,
        source_task_id: source_task_id.to_owned(),
    };
    tx.execute(
        "INSERT INTO planning_work_order_validation_links(validation_record_key,work_order_id,source_task_id,ordinal) VALUES (?1,?2,?3,?4)",
        params![key,source.work_order_id,source_task_id,ordinal],
    )?;
    runtime_projection::append_runtime_event(
        tx,
        "work_order_validation_source_recorded",
        "validation_record",
        key,
        "validation source task and work-order ownership recorded",
        &serde_json::to_string(&source)?,
    )?;
    Ok(source)
}

pub(super) fn link(
    workspace: &str,
    key: &str,
    source_task_id: &str,
) -> Result<Option<PlanningWorkOrderValidationLink>> {
    validate_work_order_id(key)?;
    validate_work_order_task_id(source_task_id)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let source = link_in_transaction(&tx, key, source_task_id)?;
    tx.commit()?;
    Ok(as_link(source))
}

pub(super) fn load_source(workspace: &str, key: &str) -> Result<Option<PlanningValidationSource>> {
    validate_work_order_id(key)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(source) = read_source(&tx, key)? {
        let current = link_in_transaction(&tx, key, &source.source_task_id)?;
        tx.commit()?;
        return Ok(Some(current));
    }
    // Existing pre-upgrade queue rows have a verified task identity. A pruned/missing row
    // cannot prove legacy ownership and remains unavailable instead of silently bypassing it.
    let queued: Option<String> = tx
        .query_row(
            "SELECT content FROM runtime_distributor_queue WHERE queue_item_id=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    let Some(queued) = queued else {
        return Ok(None);
    };
    let record: PlanningAuthorityDistributorQueueRecord =
        serde_json::from_str(&queued).context("invalid validation source queue record")?;
    if record.queue_item_id != key {
        bail!("validation source queue identity mismatch");
    }
    let source = link_in_transaction(&tx, key, &record.task_id)?;
    tx.commit()?;
    Ok(Some(source))
}

pub(super) fn load(workspace: &str, key: &str) -> Result<Option<PlanningWorkOrderValidationLink>> {
    Ok(load_source(workspace, key)?.and_then(as_link))
}

pub(super) fn keys(workspace: &str, work_order_id: &str) -> Result<Vec<String>> {
    validate_work_order_id(work_order_id)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let mut statement = connection.prepare(
        "SELECT validation_record_key FROM planning_work_order_validation_links WHERE work_order_id=?1 ORDER BY ordinal",
    )?;
    Ok(statement
        .query_map(params![work_order_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn ensure_unowned_validation_source_can_be_linked(
    connection: &Connection,
    task_id: &str,
) -> Result<()> {
    let legacy: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM planning_work_order_validation_links
         WHERE source_task_id=?1 AND work_order_id IS NULL)",
        [task_id],
        |row| row.get(0),
    )?;
    if legacy {
        bail!("an executed legacy validation source cannot be rebound to a work order");
    }
    Ok(())
}
