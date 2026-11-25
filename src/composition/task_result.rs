//! Compose existing task, runtime, evidence and immutable PR lineage reads. No scheduler or write.
use anyhow::Result;

use crate::adapter::inbound::kernel_protocol::{
    TaskResultDelivery, TaskResultDispatchBlock, TaskResultPrRelation, TaskResultPullRequest,
    TaskResultReadStatus, TaskResultSession, TaskResultState, TaskResultWorkOrder,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::inbound::task_evidence_query_port::TaskEvidenceSnapshotQuery;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningValidationSource, PlanningWorkOrderRecord, PlanningWorkOrderRepositoryPort,
};
use crate::domain::parallel_mode::{PrValidationRecord, PrValidationRecordKey};
use crate::domain::planning::evidence::validate_evidence_identifier;

pub(crate) fn query(workspace: &str, task_id: &str) -> Result<TaskResultState> {
    validate_evidence_identifier(task_id).map_err(anyhow::Error::msg)?;
    let snapshot = super::planning_task_list::query(workspace)?;
    let mut result = TaskResultState::unavailable(task_id.to_owned());
    if !snapshot.available {
        return Ok(result);
    }
    result.planning_revision = snapshot.planning_revision;
    let Some(task) = snapshot
        .tasks
        .into_iter()
        .find(|task| task.task_id == task_id)
    else {
        result.status = TaskResultReadStatus::Missing;
        return Ok(result);
    };
    result.status = TaskResultReadStatus::Available;
    result.task = Some(task);
    let store = SqlitePlanningAuthorityAdapter::new();
    if let Ok(runtime) = store.load_runtime_projections(workspace) {
        project_runtime(&mut result, &runtime);
    }
    let order = match store.load_work_order_for_task(workspace, task_id) {
        Ok(order)
            if order.as_ref().is_none_or(|order| {
                order.validate().is_ok() && order.task_ids.iter().any(|id| id == task_id)
            }) =>
        {
            result.lineage_available = true;
            result.work_order = order.as_ref().map(|order| TaskResultWorkOrder {
                work_order_id: order.work_order_id.clone(),
                related_task_ids: order
                    .task_ids
                    .iter()
                    .filter(|id| id.as_str() != task_id)
                    .cloned()
                    .collect(),
                external_reference_url: order.link_reference.as_ref().map(|link| link.url.clone()),
                close_on_merge: order
                    .link_reference
                    .as_ref()
                    .is_some_and(|link| link.close_on_merge),
            });
            order
        }
        _ => None,
    };
    if let Ok(records) = store.load_runtime_pr_validation_records(workspace) {
        // A source can outlive its PR record just as it outlives the delivery queue.
        // Such a gap must not turn into a confident no-PR result for this task.
        let record_keys: std::collections::BTreeSet<_> =
            records.iter().map(|record| record.key().as_str()).collect();
        result.pr_lineage_complete = store
            .task_validation_record_keys(workspace, task_id)
            .is_ok_and(|keys| keys.iter().all(|key| record_keys.contains(key.as_str())));
        for record in records {
            // Missing source metadata is unknown. It is never evidence that this task has no PR.
            let source =
                match store.load_work_order_validation_source(workspace, record.key().as_str()) {
                    Ok(Some(source))
                        if source.validation_record_key == record.key().as_str()
                            && validate_evidence_identifier(&source.source_task_id).is_ok() =>
                    {
                        source
                    }
                    _ => {
                        result.pr_lineage_complete = false;
                        continue;
                    }
                };
            let Some(relation) = relation(&record, &source, task_id) else {
                continue;
            };
            if !source_matches_order(&source, order.as_ref()) {
                result.pr_lineage_complete = false;
                continue;
            }
            let successor = current_record_key(&store, workspace, &record);
            if successor.is_err() {
                result.pr_lineage_complete = false;
            }
            result
                .pull_requests
                .push(project_pr(&record, &source, relation, successor.ok()));
        }
    }
    let evidence_query = TaskEvidenceSnapshotQuery {
        task_ids: vec![task_id.to_owned()],
        after_task_id: None,
        limit: 1,
    };
    if let Ok(evidence) = super::task_evidence::query(workspace, &evidence_query) {
        // Never present evidence from a newer acceptance snapshot beside older task fields.
        if evidence.planning_revision == result.planning_revision {
            result.evidence = evidence;
        }
    }
    Ok(result)
}

fn project_runtime(
    result: &mut TaskResultState,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
) {
    result.runtime_available = true;
    result.sessions = runtime
        .session_details
        .iter()
        .filter(|session| session.task_id == result.task_id)
        .map(|session| TaskResultSession {
            session_key: session.session_key.clone(),
            agent_id: session.agent_id.clone(),
            thread_id: session.thread_id.clone(),
            branch: session.branch_name.clone(),
            state: session.state_label.clone(),
            completion_state: session.completion_state_label.clone(),
            worker_validation_summary: session.validation_summary.clone(),
            delivery_outcome: session.distributor_outcome.clone(),
            updated_at: session.updated_at.clone(),
        })
        .collect();
    result.dispatch_blocks = runtime
        .task_dispatch_blocks
        .iter()
        .filter(|block| block.task_id == result.task_id)
        .map(|block| TaskResultDispatchBlock {
            reason: block.reason.label().to_owned(),
            blocked_at: block.blocked_at.clone(),
        })
        .collect();
    result.deliveries = runtime.distributor_queue_records.iter()
        .filter(|row| row.task_id == result.task_id)
        .map(|row| {
            let belongs = |observation: &&crate::domain::planning::delivery_evidence::TaskDeliveryEvidenceObservation| {
                observation.scope.as_ref().is_none_or(|scope| scope.task_id == result.task_id)
            };
            TaskResultDelivery {
                queue_item_id: row.queue_item_id.clone(),
                state: row.queue_state.label().to_owned(),
                branch: row.branch_name.clone(),
                source_commit_sha: row.source_commit_sha.clone(),
                integration_state: row.integration_state.clone(),
                integration_commit_sha: row.integration_commit_sha.clone(),
                pull_request_number: row.pull_request_number,
                pull_request_url: row.pull_request_url.clone(),
                note: row.recovery_note.clone().or_else(|| (!row.integration_note.is_empty()).then(|| row.integration_note.clone())),
                evidence_wait: row.evidence_wait,
                evidence_available: row.evidence_observations.iter().all(|entry| belongs(&entry)),
                evidence_observations: row.evidence_observations.iter().filter(belongs).cloned().collect(),
                updated_at: row.updated_at.clone(),
            }
        }).collect();
}

fn relation(
    record: &PrValidationRecord,
    source: &PlanningValidationSource,
    task_id: &str,
) -> Option<TaskResultPrRelation> {
    if source.source_task_id == task_id {
        Some(TaskResultPrRelation::Source)
    } else if record.remediation_for_task(task_id).is_some() {
        Some(TaskResultPrRelation::Remediation)
    } else {
        None
    }
}

fn source_matches_order(
    source: &PlanningValidationSource,
    order: Option<&PlanningWorkOrderRecord>,
) -> bool {
    // None is a verified legacy source, not missing lineage metadata.
    source.work_order_id.as_ref().is_none_or(|work_order_id| {
        order.is_some_and(|order| {
            &order.work_order_id == work_order_id && order.task_ids.contains(&source.source_task_id)
        })
    })
}

fn current_record_key(
    store: &dyn PlanningAuthorityPort,
    workspace: &str,
    record: &PrValidationRecord,
) -> Result<PrValidationRecordKey> {
    store.resolve_runtime_pr_validation_successor_key(workspace, record.key())
}

fn project_pr(
    record: &PrValidationRecord,
    source: &PlanningValidationSource,
    relation: TaskResultPrRelation,
    successor: Option<PrValidationRecordKey>,
) -> TaskResultPullRequest {
    let summary = record.operator_summary();
    let phase = summary.phase_label().to_owned();
    let current = successor.as_ref().map(|key| key == record.key());
    let successor_record_id = successor
        .filter(|key| key != record.key())
        .map(|key| key.as_str().to_owned());
    TaskResultPullRequest {
        record_id: record.key().as_str().to_owned(),
        source_task_id: source.source_task_id.clone(),
        work_order_id: source.work_order_id.clone(),
        relation,
        number: summary.pull_request_number,
        url: summary.canonical_pr_url,
        state: summary.state.label().to_owned(),
        phase,
        reason: summary.reason,
        blocker: summary.blocker,
        next_action: summary.recovery_action,
        current,
        successor_record_id,
        merge_sha: record.merge_sha().map(|sha| sha.as_str().to_owned()),
        integration_method: record
            .integration_attestation()
            .map(|proof| proof.method().label().to_owned()),
        integration_evidence_sha: record
            .integration_attestation()
            .map(|proof| proof.evidence_sha().as_str().to_owned()),
    }
}

#[cfg(test)]
mod tests;
