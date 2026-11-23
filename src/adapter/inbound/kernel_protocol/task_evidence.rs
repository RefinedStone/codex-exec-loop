//! 완료 기준 증거의 읽기 투영이다. artifact 내용과 경로는 준비 명령의 결과 외에는 내보내지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{KernelProtocolError, TaskAcceptanceCriterion};
use crate::application::port::inbound::task_evidence_query_port::{
    self as projection, TaskEvidenceSnapshotQuery,
};
use crate::domain::kernel_event::{TaskCriterionEvidenceRecord, TaskCriterionWaiverRecord};
use crate::domain::planning::evidence::{
    MAX_TASK_EVIDENCE_PAGE, TaskCriterionEvidenceState, TaskCriterionWaiverReceipt,
    TaskEvidenceError, TaskEvidencePrepared, TaskEvidenceReceipt, TaskEvidenceScope,
    TaskEvidenceTree, validate_evidence_identifier,
};

pub(super) fn query_request(
    task_ids: Option<&[String]>,
    after_task_id: Option<&str>,
    limit: Option<usize>,
) -> Result<TaskEvidenceSnapshotQuery, KernelProtocolError> {
    let invalid = || KernelProtocolError::invalid_params("invalid task_evidence page selection");
    let limit = limit.unwrap_or(MAX_TASK_EVIDENCE_PAGE);
    if !(1..=MAX_TASK_EVIDENCE_PAGE).contains(&limit)
        || (task_ids.is_some() && after_task_id.is_some())
        || after_task_id.is_some_and(|id| validate_evidence_identifier(id).is_err())
    {
        return Err(invalid());
    }
    if let Some(ids) = task_ids {
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        if ids.is_empty()
            || ids.len() > limit
            || unique.len() != ids.len()
            || ids
                .iter()
                .any(|id| validate_evidence_identifier(id).is_err())
        {
            return Err(invalid());
        }
    }
    Ok(TaskEvidenceSnapshotQuery {
        task_ids: task_ids.unwrap_or_default().to_vec(),
        after_task_id: after_task_id.map(str::to_string),
        limit,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidencePreparedState {
    pub attempt_id: String,
    pub task_id: String,
    pub criterion_id: String,
    pub scope_id: String,
    pub acceptance_revision: u64,
    pub attempt: u32,
    pub artifact_path: String,
    pub changed: bool,
}

impl From<TaskEvidencePrepared> for TaskEvidencePreparedState {
    fn from(prepared: TaskEvidencePrepared) -> Self {
        Self {
            attempt_id: prepared.attempt.attempt_id,
            task_id: prepared.attempt.scope.task_id,
            criterion_id: prepared.attempt.criterion_id,
            scope_id: prepared.attempt.scope.scope_id,
            acceptance_revision: prepared.attempt.scope.acceptance_revision,
            attempt: prepared.attempt.attempt,
            artifact_path: prepared.artifact_path,
            changed: prepared.changed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceReceiptState {
    pub event_id: String,
    pub evidence_revision: u64,
    pub evidence: TaskCriterionEvidenceRecord,
}

impl From<&TaskEvidenceReceipt> for TaskEvidenceReceiptState {
    fn from(receipt: &TaskEvidenceReceipt) -> Self {
        Self {
            event_id: receipt.event_id.clone(),
            evidence_revision: receipt.evidence_revision,
            evidence: (&receipt.evidence).into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCriterionWaiverReceiptState {
    pub event_id: String,
    pub evidence_revision: u64,
    pub waiver: TaskCriterionWaiverRecord,
}

impl From<&TaskCriterionWaiverReceipt> for TaskCriterionWaiverReceiptState {
    fn from(receipt: &TaskCriterionWaiverReceipt) -> Self {
        Self {
            event_id: receipt.event_id.clone(),
            evidence_revision: receipt.evidence_revision,
            waiver: (&receipt.waiver).into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planning_revision: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<TaskEvidenceError>,
    pub tasks: Vec<TaskEvidenceTaskState>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_task_id: Option<String>,
}

impl TaskEvidenceState {
    pub fn unavailable() -> Self {
        Self {
            planning_revision: None,
            evidence_revision: None,
            unavailable: Some(TaskEvidenceError::Unavailable),
            tasks: Vec::new(),
            truncated: false,
            next_task_id: None,
        }
    }
}

impl From<projection::TaskEvidencePlanningSnapshot> for TaskEvidenceState {
    fn from(snapshot: projection::TaskEvidencePlanningSnapshot) -> Self {
        Self {
            planning_revision: snapshot.planning_revision,
            evidence_revision: snapshot.evidence_revision,
            unavailable: snapshot.unavailable,
            tasks: snapshot.tasks.into_iter().map(Into::into).collect(),
            truncated: snapshot.truncated,
            next_task_id: snapshot.next_task_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceTaskState {
    pub scope: TaskEvidenceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TaskEvidenceSourceState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<TaskEvidenceError>,
    pub criteria: Vec<PlanningCriterionEvidenceState>,
}

impl From<projection::TaskEvidenceTaskProjection> for TaskEvidenceTaskState {
    fn from(task: projection::TaskEvidenceTaskProjection) -> Self {
        Self {
            scope: task.scope,
            source: task.source.map(|source| TaskEvidenceSourceState {
                kind: match source.kind {
                    projection::TaskEvidenceSourceKind::RunningLease => {
                        TaskEvidenceSourceKind::RunningLease
                    }
                    projection::TaskEvidenceSourceKind::QueuedSource => {
                        TaskEvidenceSourceKind::QueuedSource
                    }
                },
                tree: source.tree,
            }),
            unavailable: task.unavailable,
            criteria: task.criteria.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceSourceState {
    pub kind: TaskEvidenceSourceKind,
    pub tree: TaskEvidenceTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskEvidenceSourceKind {
    RunningLease,
    QueuedSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCriterionEvidenceState {
    pub criterion: TaskAcceptanceCriterion,
    /// 현재 source를 읽을 수 없으면 None이다. 과거 Pass를 현재 결과로 올리지 않는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<TaskCriterionEvidenceState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<TaskEvidenceReceiptState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiver: Option<TaskCriterionWaiverReceiptState>,
}

impl From<projection::TaskCriterionEvidenceProjection> for PlanningCriterionEvidenceState {
    fn from(criterion: projection::TaskCriterionEvidenceProjection) -> Self {
        Self {
            criterion: criterion.criterion.into(),
            state: criterion.state,
            evidence: criterion.evidence.map(|receipt| TaskEvidenceReceiptState {
                event_id: receipt.event_id,
                evidence_revision: receipt.evidence_revision,
                evidence: TaskCriterionEvidenceRecord {
                    scope: receipt.scope,
                    criterion_id: receipt.criterion_id,
                    attempt_id: receipt.attempt_id,
                    attempt: receipt.attempt,
                    status: receipt.status,
                    artifact: receipt.artifact,
                    recorded_tree: receipt.recorded_tree,
                    by: receipt.by,
                    recorded_at: receipt.recorded_at,
                },
            }),
            waiver: criterion
                .waiver
                .map(|receipt| TaskCriterionWaiverReceiptState {
                    event_id: receipt.event_id,
                    evidence_revision: receipt.evidence_revision,
                    waiver: TaskCriterionWaiverRecord {
                        scope: receipt.scope,
                        criterion_id: receipt.criterion_id,
                        reason_digest: receipt.reason_digest,
                        by: crate::domain::kernel_event::TaskCriterionWaiverActor::Human,
                        recorded_at: receipt.recorded_at,
                    },
                }),
        }
    }
}

#[cfg(test)]
mod tests;
