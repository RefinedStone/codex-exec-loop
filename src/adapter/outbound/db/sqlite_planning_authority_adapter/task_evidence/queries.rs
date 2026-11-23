use super::*;

fn selected_ids(
    connection: &Connection,
    query: &TaskEvidenceSnapshotQuery,
) -> Result<(Vec<String>, bool)> {
    if query.limit == 0 || query.limit > MAX_TASK_EVIDENCE_PAGE {
        return Err(TaskEvidenceError::Limit.into());
    }
    if query.task_ids.is_empty() {
        if let Some(after) = &query.after_task_id {
            validate_evidence_identifier(after)?;
        }
        let mut statement = connection.prepare("SELECT task_id FROM planning_tasks WHERE (?1 IS NULL OR task_id>?1) ORDER BY task_id LIMIT ?2")?;
        let mut ids = statement
            .query_map(
                params![query.after_task_id, query.limit as i64 + 1],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let truncated = ids.len() > query.limit;
        ids.truncate(query.limit);
        return Ok((ids, truncated));
    }
    if query.after_task_id.is_some() {
        return Err(TaskEvidenceError::Invalid.into());
    }
    let ids = query.task_ids.iter().cloned().collect::<BTreeSet<_>>();
    if ids.len() != query.task_ids.len() {
        return Err(TaskEvidenceError::Invalid.into());
    }
    if ids.len() > MAX_TASK_EVIDENCE_PAGE || query.task_ids.len() > MAX_TASK_EVIDENCE_PAGE {
        return Err(TaskEvidenceError::Limit.into());
    }
    if ids.len() > query.limit {
        return Err(TaskEvidenceError::Limit.into());
    }
    for id in &ids {
        validate_evidence_identifier(id)?;
    }
    let mut found = Vec::new();
    for id in ids {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM planning_tasks WHERE task_id=?1)",
            [&id],
            |row| row.get(0),
        )?;
        if exists {
            found.push(id);
        }
    }
    Ok((found, false))
}

fn task_snapshot(connection: &Connection, task_id: &str) -> Result<TaskEvidenceTaskSnapshot> {
    let task = current_task(connection, task_id)?;
    let scope = read_scope(connection, task_id)?.ok_or(TaskEvidenceError::ScopeChanged)?;
    if scope.acceptance_revision != task.acceptance_revision
        || scope.criteria_sha256 != criteria_sha256(&task.acceptance)?
    {
        return Err(TaskEvidenceError::ScopeChanged.into());
    }
    let leases: Vec<ParallelModeSlotLeaseSnapshot> = read_runtime_rows(
        connection,
        "SELECT content FROM runtime_slot_leases WHERE json_extract(content,'$.task_id')=?1 ORDER BY slot_id LIMIT 257",
        task_id,
    )?;
    let queue_records: Vec<PlanningAuthorityDistributorQueueRecord> = read_runtime_rows(
        connection,
        "SELECT content FROM runtime_distributor_queue WHERE json_extract(content,'$.task_id')=?1 ORDER BY queue_item_id LIMIT 257",
        task_id,
    )?;
    let mut evidence = Vec::new();
    let mut waivers = Vec::new();
    let mut host_verifications = Vec::new();
    if task.acceptance.len() > MAX_TASK_EVIDENCE_PAGE {
        return Err(TaskEvidenceError::Limit.into());
    }
    for criterion in &task.acceptance {
        if let Some(record) =
            verification::latest(connection, task_id, &criterion.id, &scope.scope_id)?
        {
            host_verifications.push(record);
        }
        let record: Option<(String,u64)> = connection.query_row(
            "SELECT receipt_json,record_revision FROM task_evidence_attempts WHERE task_id=?1 AND criterion_id=?2 AND receipt_json IS NOT NULL
            ORDER BY (scope_id=?3) DESC,CASE WHEN scope_id=?3 THEN ordinal ELSE 0 END DESC,record_revision DESC LIMIT 1",
            params![task_id,criterion.id,scope.scope_id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((json, revision)) = record {
            let receipt: TaskEvidenceReceipt = serde_json::from_str(&json)?;
            receipt.validate()?;
            if receipt.evidence.attempt.scope.task_id != task_id
                || receipt.evidence.attempt.criterion_id != criterion.id
                || receipt.evidence_revision != revision
            {
                return Err(TaskEvidenceError::Unavailable.into());
            }
            evidence.push(receipt);
        }
        let waiver: Option<(String,u64)> = connection.query_row(
            "SELECT receipt_json,evidence_revision FROM task_criterion_waivers WHERE task_id=?1 AND criterion_id=?2 ORDER BY evidence_revision DESC LIMIT 1",
            params![task_id,criterion.id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((json, revision)) = waiver {
            let receipt: TaskCriterionWaiverReceipt = serde_json::from_str(&json)?;
            receipt.validate()?;
            if receipt.waiver.scope.task_id != task_id
                || receipt.waiver.criterion_id != criterion.id
                || receipt.evidence_revision != revision
            {
                return Err(TaskEvidenceError::Unavailable.into());
            }
            waivers.push(receipt);
        }
    }
    Ok(TaskEvidenceTaskSnapshot {
        task,
        scope,
        leases,
        queue_records,
        evidence,
        waivers,
        host_verifications,
    })
}

fn read_runtime_rows<T: for<'de> Deserialize<'de>>(
    connection: &Connection,
    sql: &str,
    task_id: &str,
) -> Result<Vec<T>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement
        .query_map([task_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() > MAX_TASK_EVIDENCE_PAGE {
        return Err(TaskEvidenceError::Limit.into());
    }
    rows.into_iter()
        .map(|json| serde_json::from_str(&json).map_err(Into::into))
        .collect()
}

pub(super) fn snapshot(
    workspace: &str,
    query: &TaskEvidenceSnapshotQuery,
) -> Result<TaskEvidenceStoredSnapshot> {
    let connection = connection(workspace)?;
    let transaction = connection.unchecked_transaction()?;
    let planning_revision =
        read_metadata_i64_connection(&transaction, "planning_revision")?.unwrap_or(0);
    let evidence_revision: u64 = transaction.query_row(
        "SELECT revision FROM task_evidence_state WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if evidence_revision > MAX_TASK_EVIDENCE_REVISION {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    let (ids, truncated) = selected_ids(&transaction, query)?;
    let next_task_id = truncated.then(|| ids.last().cloned()).flatten();
    let tasks = ids
        .iter()
        .map(|id| task_snapshot(&transaction, id))
        .collect::<Result<Vec<_>>>()?;
    transaction.commit()?;
    Ok(TaskEvidenceStoredSnapshot {
        planning_revision,
        evidence_revision,
        tasks,
        truncated,
        next_task_id,
    })
}

pub(super) fn require_event(connection: &Connection, event_id: &str) -> Result<TaskEvidenceEvent> {
    let row: Option<(String, u64)> = connection
        .query_row(
            "SELECT payload_json,evidence_revision FROM task_evidence_events WHERE event_id=?1",
            [event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (json, revision) = row.ok_or(TaskEvidenceError::Missing)?;
    let event: TaskEvidenceEvent = serde_json::from_str(&json)?;
    event.validate()?;
    if event.event_id() != event_id || event.evidence_revision() != revision {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    match &event {
        TaskEvidenceEvent::Recorded { receipt } => {
            let original = read_attempt(connection, &receipt.evidence.attempt.attempt_id)?
                .and_then(|record| record.receipt);
            if original.as_ref() != Some(receipt.as_ref()) {
                return Err(TaskEvidenceError::Unavailable.into());
            }
        }
        TaskEvidenceEvent::Waived { receipt } => {
            let original: Option<String> = connection
                .query_row(
                    "SELECT receipt_json FROM task_criterion_waivers WHERE request_id=?1",
                    [&receipt.waiver.request_id],
                    |row| row.get(0),
                )
                .optional()?;
            let original = original
                .map(|json| serde_json::from_str::<TaskCriterionWaiverReceipt>(&json))
                .transpose()?;
            if original.as_ref() != Some(receipt.as_ref()) {
                return Err(TaskEvidenceError::Unavailable.into());
            }
        }
        TaskEvidenceEvent::VerificationFinished { receipt } => {
            let original = verification::original_receipt(connection, &receipt.attempt.attempt_id)?;
            if original.as_ref() != Some(receipt.as_ref()) {
                return Err(TaskEvidenceError::Unavailable.into());
            }
        }
    }
    Ok(event)
}

pub(super) fn pending(workspace: &str, limit: usize) -> Result<Vec<TaskEvidenceEvent>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let connection = connection(workspace)?;
    let transaction = connection.unchecked_transaction()?;
    let ids = {
        let mut statement = transaction.prepare("SELECT event_id FROM task_evidence_events WHERE acknowledged=0 ORDER BY evidence_revision LIMIT ?1")?;
        statement
            .query_map([limit.min(MAX_TASK_EVIDENCE_PAGE) as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let events = ids
        .iter()
        .map(|id| require_event(&transaction, id))
        .collect::<Result<Vec<_>>>()?;
    transaction.commit()?;
    Ok(events)
}
