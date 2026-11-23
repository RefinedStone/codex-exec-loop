//! Internal delivery boundary. A decision is valid only for the immediately following effect.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::application::port::outbound::task_delivery_evidence_repository_port::TaskDeliveryEvidenceOperation;
use crate::domain::planning::delivery_evidence::{
    TaskDeliveryEvidenceReason, TaskEvidenceDeliveryStage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDeliveryEvidenceIntegration {
    pub base_commit_sha: String,
    pub result_commit_sha: String,
    /// The exact permit condition used by the final PR readiness check. A real effect must
    /// provide it; a typed wait recheck has no fresh PR observation and may leave it absent.
    pub preflight_requires_human_review: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct TaskDeliveryEvidenceDecision {
    pub updated_queue: PlanningAuthorityDistributorQueueRecord,
    /// Recheck never authorizes an effect; inspect the returned typed wait to see a rearm.
    pub allowed: bool,
    pub reason: Option<TaskDeliveryEvidenceReason>,
}

pub trait TaskDeliveryEvidenceGatePort: Send + Sync {
    /// An allowed Effect must end with a transaction that rechecks the current distributor
    /// claim, task permit/control, queue and evidence after all filesystem observations.
    /// Callers perform only their in-memory epoch check before invoking the external effect.
    fn evaluate(
        &self,
        workspace: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
        stage: TaskEvidenceDeliveryStage,
        integration: Option<&TaskDeliveryEvidenceIntegration>,
        operation: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String>;
}

/// Missing production wiring cannot silently select Observe.
#[derive(Default)]
pub struct UnavailableTaskDeliveryEvidenceGate;

impl TaskDeliveryEvidenceGatePort for UnavailableTaskDeliveryEvidenceGate {
    fn evaluate(
        &self,
        _: &str,
        _: &PlanningAuthorityDistributorQueueRecord,
        _: TaskEvidenceDeliveryStage,
        _: Option<&TaskDeliveryEvidenceIntegration>,
        _: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String> {
        Err("evidence_unavailable: delivery evidence policy is not initialized".into())
    }
}

/// Existing delivery fixtures opt into this fake explicitly. Production never constructs it.
#[cfg(test)]
#[derive(Default)]
pub struct ObserveTaskDeliveryEvidenceGate;

#[cfg(test)]
impl TaskDeliveryEvidenceGatePort for ObserveTaskDeliveryEvidenceGate {
    fn evaluate(
        &self,
        _: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
        _: TaskEvidenceDeliveryStage,
        _: Option<&TaskDeliveryEvidenceIntegration>,
        operation: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String> {
        Ok(TaskDeliveryEvidenceDecision {
            updated_queue: record.clone(),
            allowed: matches!(operation, TaskDeliveryEvidenceOperation::Effect { .. }),
            reason: None,
        })
    }
}
