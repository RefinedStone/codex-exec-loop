//! 완료 기준 증거의 공개 사실이다. 원문·경로·native capability는 복사하지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::KernelEvent;
use crate::domain::planning::evidence::{
    TaskCriterionEvidence, TaskCriterionWaiver, TaskEvidenceArtifact, TaskEvidenceEvent,
    TaskEvidenceScope, TaskEvidenceStatus, TaskEvidenceTree,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCriterionEvidenceRecord {
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub attempt_id: String,
    pub attempt: u32,
    pub status: TaskEvidenceStatus,
    pub artifact: TaskEvidenceArtifact,
    pub recorded_tree: TaskEvidenceTree,
    /// 기존 서버 caller 감사 정보다. Human 또는 Worker 인증 증명으로 쓰지 않는다.
    pub by: String,
    pub recorded_at: String,
}

impl From<&TaskCriterionEvidence> for TaskCriterionEvidenceRecord {
    fn from(evidence: &TaskCriterionEvidence) -> Self {
        Self {
            scope: evidence.attempt.scope.clone(),
            criterion_id: evidence.attempt.criterion_id.clone(),
            attempt_id: evidence.attempt.attempt_id.clone(),
            attempt: evidence.attempt.attempt,
            status: evidence.status,
            artifact: evidence.artifact.clone(),
            recorded_tree: evidence.recorded_tree.clone(),
            by: evidence.by.clone(),
            recorded_at: evidence.recorded_at.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCriterionWaiverRecord {
    pub scope: TaskEvidenceScope,
    pub criterion_id: String,
    pub reason_digest: String,
    pub by: TaskCriterionWaiverActor,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskCriterionWaiverActor {
    Human,
}

impl From<&TaskCriterionWaiver> for TaskCriterionWaiverRecord {
    fn from(waiver: &TaskCriterionWaiver) -> Self {
        Self {
            scope: waiver.scope.clone(),
            criterion_id: waiver.criterion_id.clone(),
            reason_digest: waiver.reason_digest.clone(),
            by: TaskCriterionWaiverActor::Human,
            recorded_at: waiver.recorded_at.clone(),
        }
    }
}

impl KernelEvent {
    /// 저장 transaction의 최초 영수증만 투영한다. 현재 조회로 과거 사실을 다시 만들지 않는다.
    pub fn task_evidence(event: &TaskEvidenceEvent) -> Self {
        match event {
            TaskEvidenceEvent::Recorded { receipt } => Self::CriterionEvidenceRecorded {
                event_id: receipt.event_id.clone(),
                evidence_revision: receipt.evidence_revision,
                evidence: (&receipt.evidence).into(),
            },
            TaskEvidenceEvent::Waived { receipt } => Self::CriterionWaived {
                event_id: receipt.event_id.clone(),
                evidence_revision: receipt.evidence_revision,
                waiver: (&receipt.waiver).into(),
            },
            TaskEvidenceEvent::VerificationFinished { receipt } => Self::TaskVerificationFinished {
                event_id: receipt.event_id.clone(),
                evidence_revision: receipt.evidence_revision,
                verification: Box::new(receipt.as_ref().into()),
            },
        }
    }

    pub fn task_evidence_event_id(&self) -> Option<&str> {
        match self {
            Self::CriterionEvidenceRecorded { event_id, .. }
            | Self::TaskVerificationFinished { event_id, .. }
            | Self::CriterionWaived { event_id, .. } => Some(event_id),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) fn sample_task_evidence_events() -> [TaskEvidenceEvent; 2] {
    use crate::domain::planning::evidence::{
        TaskCriterionWaiverReceipt, TaskEvidenceAttempt, TaskEvidenceReceipt,
        normalize_waiver_reason,
    };
    let scope = TaskEvidenceScope {
        task_id: "task-1".into(),
        scope_id: "a".repeat(64),
        acceptance_revision: 3,
        criteria_sha256: "b".repeat(64),
    };
    let tree = TaskEvidenceTree {
        head_commit_sha: "c".repeat(40),
        tree_sha: "d".repeat(40),
        worktree_clean: false,
    };
    let (reason, reason_digest) = normalize_waiver_reason("private waiver reason").unwrap();
    [
        TaskEvidenceEvent::Recorded {
            receipt: Box::new(TaskEvidenceReceipt {
                event_id: "criterion-evidence:fixture".into(),
                evidence_revision: 4,
                evidence: TaskCriterionEvidence {
                    attempt: TaskEvidenceAttempt {
                        attempt_id: "e".repeat(64),
                        request_id: "prepare-fixture".into(),
                        scope: scope.clone(),
                        criterion_id: "criterion with spaces".into(),
                        attempt: 1,
                        slot_id: "slot-1".into(),
                        lease_generation: "f".repeat(64),
                        session_key: "private-session".into(),
                        prepared_tree: tree.clone(),
                        prepared_at: "2026-11-23T00:00:00Z".into(),
                    },
                    status: TaskEvidenceStatus::Pass,
                    artifact: TaskEvidenceArtifact {
                        sha256: "1".repeat(64),
                        size_bytes: 42,
                    },
                    recorded_tree: tree,
                    note: Some("private evidence note".into()),
                    by: "fixture-worker".into(),
                    recorded_at: "2026-11-23T00:00:01Z".into(),
                },
            }),
        },
        TaskEvidenceEvent::Waived {
            receipt: Box::new(TaskCriterionWaiverReceipt {
                event_id: "criterion-waiver:fixture".into(),
                evidence_revision: 5,
                waiver: TaskCriterionWaiver {
                    request_id: "waive-fixture".into(),
                    scope,
                    criterion_id: "criterion with spaces".into(),
                    reason,
                    reason_digest,
                    confirmation_ref: format!("desktop-confirmation:{}", "2".repeat(32)),
                    recorded_at: "2026-11-23T00:00:02Z".into(),
                },
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_facts_preserve_original_identity_and_omit_private_contents() {
        for event in sample_task_evidence_events() {
            event.validate().unwrap();
            let record = KernelEvent::task_evidence(&event);
            assert_eq!(record.task_evidence_event_id(), Some(event.event_id()));
            assert_eq!(record.issue_work_event_id(), None);
            assert_eq!(record.op_id(), None);
            assert_eq!(record.thread_id(), "");
            let encoded = serde_json::to_string(&record).unwrap();
            for private in [
                "private evidence note",
                "private waiver reason",
                "private-session",
                "confirmation_ref",
                "artifact_path",
                "lease_generation",
            ] {
                assert!(!encoded.contains(private), "{private}");
            }
            assert_eq!(
                serde_json::from_str::<KernelEvent>(&encoded).unwrap(),
                record
            );
            assert_eq!(
                serde_json::to_value(record).unwrap()["evidence_revision"],
                event.evidence_revision()
            );
        }
    }
}
