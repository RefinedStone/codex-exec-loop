//! A delivery observation describes one frozen source. It is never an execution permit.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::evidence::TaskEvidenceScope;

mod validation;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskEvidenceDeliveryMode {
    #[default]
    Observe,
    Enforce,
}

impl TaskEvidenceDeliveryMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "observe" => Ok(Self::Observe),
            "enforce" => Ok(Self::Enforce),
            _ => Err("task evidence delivery mode must be exactly `observe` or `enforce`".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskEvidenceDeliveryStage {
    Push,
    PullRequest,
    Integrate,
}

impl TaskEvidenceDeliveryStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::PullRequest => "pull_request",
            Self::Integrate => "integrate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskDeliveryEvidenceReason {
    CriteriaMissing,
    CriteriaNotMet,
    EvidenceMissing,
    EvidenceStale,
    EvidenceUnavailable,
    IntegrationSourceMismatch,
    AutoRequiresEnforce,
    AutoRequiresMachineCriterion,
}

impl TaskDeliveryEvidenceReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::CriteriaMissing => "criteria_missing",
            Self::CriteriaNotMet => "criteria_not_met",
            Self::EvidenceMissing => "evidence_missing",
            Self::EvidenceStale => "evidence_stale",
            Self::EvidenceUnavailable => "evidence_unavailable",
            Self::IntegrationSourceMismatch => "integration_source_mismatch",
            Self::AutoRequiresEnforce => "auto_requires_enforce",
            Self::AutoRequiresMachineCriterion => "auto_requires_machine_criterion",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceDeliveryBinding {
    pub scope: TaskEvidenceScope,
    pub lease_generation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceIntegrationResult {
    pub base_commit_sha: String,
    pub result_commit_sha: String,
    pub result_tree_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvidenceDeliverySource {
    pub base_commit_sha: String,
    pub base_tree_sha: String,
    pub source_commit_sha: String,
    pub source_tree_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration: Option<TaskEvidenceIntegrationResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskDeliveryCriterionState {
    HostPassed,
    Waived,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskDeliveryCriterionObservation {
    pub criterion_id: String,
    pub state: TaskDeliveryCriterionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<TaskDeliveryEvidenceReason>,
    /// The immutable host or human-waiver receipt, never a worker-reported Pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskDeliveryEvidenceObservation {
    pub stage: TaskEvidenceDeliveryStage,
    pub mode: TaskEvidenceDeliveryMode,
    pub scope: Option<TaskEvidenceScope>,
    pub evidence_revision: u64,
    pub source: Option<TaskEvidenceDeliverySource>,
    pub criteria: Vec<TaskDeliveryCriterionObservation>,
    pub reasons: Vec<TaskDeliveryEvidenceReason>,
    pub observed_at: String,
}

impl TaskDeliveryEvidenceObservation {
    pub fn satisfied(&self) -> bool {
        !self.criteria.is_empty()
            && self.reasons.is_empty()
            && self
                .criteria
                .iter()
                .all(|row| row.state != TaskDeliveryCriterionState::Blocked)
    }

    pub fn allows_delivery(&self) -> bool {
        !self.reasons.iter().any(|reason| {
            matches!(
                reason,
                TaskDeliveryEvidenceReason::AutoRequiresEnforce
                    | TaskDeliveryEvidenceReason::AutoRequiresMachineCriterion
            )
        }) && (self.mode == TaskEvidenceDeliveryMode::Observe || self.satisfied())
    }

    /// Time is diagnostic. Re-reading unchanged evidence must not append another audit event.
    pub fn same_assessment_as(&self, other: &Self) -> bool {
        self.stage == other.stage
            && self.mode == other.mode
            && self.scope == other.scope
            && self.evidence_revision == other.evidence_revision
            && self.source == other.source
            && self.criteria == other.criteria
            && self.reasons == other.reasons
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskDeliveryEvidenceError {
    Invalid,
    Missing,
    Conflict,
    ClaimLost,
    ControlDenied,
    Unavailable,
}

impl std::fmt::Display for TaskDeliveryEvidenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "task delivery evidence input is invalid",
            Self::Missing => "the frozen task delivery source was not found",
            Self::Conflict => "task delivery evidence changed during observation",
            Self::ClaimLost => "the distributor no longer owns this delivery claim",
            Self::ControlDenied => "current task authority does not permit delivery",
            Self::Unavailable => "task delivery evidence could not be observed",
        })
    }
}

impl std::error::Error for TaskDeliveryEvidenceError {}

#[cfg(test)]
mod tests;
