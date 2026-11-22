//! Validation identity retains its original task even after runtime queue pruning.
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
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
    if let Some(owner) = load_task_membership(tx, source_task_id)? {
        // Import the whole legacy queue before a new source can claim the latest ordinal.
        // This also makes a direct lookup of any legacy key independent of lookup order.
        backfill_owned_queue_sources(tx, &owner)?;
    }
    insert_source_in_transaction(tx, key, source_task_id)
}

fn insert_source_in_transaction(
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
        Some(owner) => {
            let previous: i64 = tx.query_row(
                "SELECT COALESCE(MAX(ordinal),-1) FROM planning_work_order_validation_links WHERE work_order_id=?1",
                [owner],
                |row| row.get(0),
            )?;
            previous
                .checked_add(1)
                .context("validation source ordinal exhausted")?
        }
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
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    backfill_owned_queue_sources(&tx, work_order_id)?;
    let keys = {
        let mut statement = tx.prepare(
            "SELECT validation_record_key FROM planning_work_order_validation_links WHERE work_order_id=?1 ORDER BY ordinal",
        )?;
        statement
            .query_map(params![work_order_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    tx.commit()?;
    Ok(keys)
}

struct QueuedValidationSource {
    key: String,
    source_task_id: String,
    enqueued_at: DateTime<Utc>,
}

pub(super) fn backfill_owned_queue_sources(
    tx: &Transaction<'_>,
    work_order_id: &str,
) -> Result<()> {
    // Current queue writes already record their source atomically. SQL narrows missing
    // sources to this work's immutable lineage before any full queue record is decoded.
    let pending = {
        let mut statement = tx.prepare(
            "SELECT queue.queue_item_id, queue.content, queue.enqueued_at
             FROM runtime_distributor_queue AS queue
             WHERE NOT EXISTS (
                 SELECT 1 FROM planning_work_order_validation_links AS link
                 WHERE link.validation_record_key = queue.queue_item_id
             )
             AND json_extract(queue.content, '$.task_id') IN (
                 SELECT task_id FROM planning_work_order_tasks WHERE work_order_id = ?1
             )
             ORDER BY queue.enqueued_at, queue.queue_item_id",
        )?;
        statement
            .query_map([work_order_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if pending.is_empty() {
        // Ordinary reads do not inspect retained anchors or rewrite any existing ordinal.
        return Ok(());
    }
    let anchors = {
        let mut statement = tx.prepare(
            "SELECT link.validation_record_key, link.source_task_id, queue.content, queue.enqueued_at
             FROM planning_work_order_validation_links AS link
             LEFT JOIN runtime_distributor_queue AS queue
                 ON queue.queue_item_id = link.validation_record_key
             WHERE link.work_order_id = ?1 ORDER BY link.ordinal",
        )?;
        statement
            .query_map([work_order_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut imported = Vec::new();
    for (key, content, enqueued_at) in pending {
        let source =
            decode_owned_queue_source(tx, work_order_id, &key, &content, &enqueued_at, None)?;
        // Use the raw insertion boundary: recursive backfill would make order depend on
        // whichever key happened to be requested first. Any later error rolls this back.
        insert_source_in_transaction(tx, &source.key, &source.source_task_id)?;
        imported.push(source);
    }
    let mut existing = Vec::new();
    for (key, task_id, content, enqueued_at) in anchors {
        let content =
            content.context("legacy validation order requires every existing source queue")?;
        let enqueued_at =
            enqueued_at.context("legacy validation source has no enqueue timestamp")?;
        existing.push(decode_owned_queue_source(
            tx,
            work_order_id,
            &key,
            &content,
            &enqueued_at,
            Some(&task_id),
        )?);
    }
    if existing
        .windows(2)
        .any(|pair| pair[0].enqueued_at > pair[1].enqueued_at)
    {
        bail!("legacy validation queue chronology contradicts existing source order");
    }
    if imported.iter().any(|source| {
        existing
            .iter()
            .any(|anchor| source.enqueued_at == anchor.enqueued_at)
    }) {
        bail!("legacy validation queue chronology is ambiguous at an existing source");
    }

    let current: Vec<_> = existing
        .iter()
        .chain(&imported)
        .map(|source| source.key.clone())
        .collect();
    // Stable sorting preserves existing sources' relative order. Queue timestamps are
    // normalized to UTC, so RFC3339 offsets do not change the chronological comparison.
    let mut ordered: Vec<_> = existing.iter().chain(&imported).collect();
    ordered.sort_by(|left, right| left.enqueued_at.cmp(&right.enqueued_at));
    let keys: Vec<_> = ordered
        .into_iter()
        .map(|source| source.key.clone())
        .collect();
    if keys != current {
        reorder_sources_in_transaction(tx, work_order_id, &keys)?;
    }
    Ok(())
}

fn decode_owned_queue_source(
    tx: &Transaction<'_>,
    work_order_id: &str,
    key: &str,
    content: &str,
    enqueued_at: &str,
    expected_task_id: Option<&str>,
) -> Result<QueuedValidationSource> {
    validate_work_order_id(key)?;
    let record: PlanningAuthorityDistributorQueueRecord =
        serde_json::from_str(content).context("invalid validation source queue record")?;
    if record.queue_item_id != key {
        bail!("validation source queue identity mismatch");
    }
    validate_work_order_task_id(&record.task_id)?;
    if expected_task_id.is_some_and(|expected| record.task_id != expected)
        || load_task_membership(tx, &record.task_id)?.as_deref() != Some(work_order_id)
    {
        bail!("validation source queue belongs to a different work-order lineage");
    }
    let enqueued_at = DateTime::parse_from_rfc3339(enqueued_at)
        .context("invalid validation source enqueue timestamp")?
        .with_timezone(&Utc);
    let payload_time = DateTime::parse_from_rfc3339(&record.enqueued_at)
        .context("invalid validation source payload enqueue timestamp")?
        .with_timezone(&Utc);
    if payload_time != enqueued_at {
        bail!("validation source queue enqueue timestamp mismatch");
    }
    Ok(QueuedValidationSource {
        key: key.to_owned(),
        source_task_id: record.task_id,
        enqueued_at,
    })
}

fn reorder_sources_in_transaction(
    tx: &Transaction<'_>,
    work_order_id: &str,
    ordered_keys: &[String],
) -> Result<()> {
    let maximum: i64 = tx.query_row(
        "SELECT COALESCE(MAX(ordinal),-1) FROM planning_work_order_validation_links WHERE work_order_id=?1",
        [work_order_id], |row| row.get(0),
    )?;
    let offset = maximum
        .checked_add(1)
        .context("validation source ordinal exhausted")?;
    maximum
        .checked_add(offset)
        .context("validation source ordinal relocation exhausted")?;
    i64::try_from(ordered_keys.len()).context("too many validation sources to order")?;
    // The temporary range is strictly above every current ordinal. The final range is
    // below it, so row-by-row UNIQUE checks cannot collide during either UPDATE pass.
    let moved = tx.execute(
        "UPDATE planning_work_order_validation_links SET ordinal=ordinal+?1 WHERE work_order_id=?2",
        params![offset, work_order_id],
    )?;
    if moved != ordered_keys.len() {
        bail!("validation source membership changed while restoring queue order");
    }
    for (ordinal, key) in ordered_keys.iter().enumerate() {
        let ordinal = i64::try_from(ordinal).context("validation source ordinal exhausted")?;
        if tx.execute(
            "UPDATE planning_work_order_validation_links SET ordinal=?1 WHERE work_order_id=?2 AND validation_record_key=?3",
            params![ordinal, work_order_id, key],
        )? != 1 {
            bail!("validation source disappeared while restoring queue order");
        }
    }
    Ok(())
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
