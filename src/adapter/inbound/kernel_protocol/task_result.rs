//! Read-only task result projection. Correlation uses immutable IDs, never titles or branches.
use super::{PlanningTaskDetails, TaskEvidenceState};
use crate::domain::planning::delivery_evidence::{
    TaskDeliveryEvidenceObservation, TaskEvidenceDeliveryStage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskResultReadStatus {
    Available,
    Missing,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskResultState {
    pub task_id: String,
    pub status: TaskResultReadStatus,
    pub planning_revision: Option<i64>,
    pub task: Option<PlanningTaskDetails>,
    pub runtime_available: bool,
    pub lineage_available: bool,
    pub pr_lineage_complete: bool,
    pub work_order: Option<TaskResultWorkOrder>,
    pub sessions: Vec<TaskResultSession>,
    pub dispatch_blocks: Vec<TaskResultDispatchBlock>,
    pub deliveries: Vec<TaskResultDelivery>,
    pub pull_requests: Vec<TaskResultPullRequest>,
    pub evidence: TaskEvidenceState,
}

impl TaskResultState {
    pub fn unavailable(task_id: String) -> Self {
        Self {
            task_id,
            status: TaskResultReadStatus::Unavailable,
            planning_revision: None,
            task: None,
            runtime_available: false,
            lineage_available: false,
            pr_lineage_complete: false,
            work_order: None,
            sessions: Vec::new(),
            dispatch_blocks: Vec::new(),
            deliveries: Vec::new(),
            pull_requests: Vec::new(),
            evidence: TaskEvidenceState::unavailable(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskResultWorkOrder {
    pub work_order_id: String,
    pub related_task_ids: Vec<String>,
    pub external_reference_url: Option<String>,
    pub close_on_merge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskResultSession {
    pub session_key: String,
    pub agent_id: String,
    pub thread_id: Option<String>,
    pub branch: String,
    pub state: String,
    pub completion_state: String,
    pub worker_validation_summary: String,
    pub delivery_outcome: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskResultDispatchBlock {
    pub reason: String,
    pub blocked_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskResultDelivery {
    pub queue_item_id: String,
    pub state: String,
    pub branch: String,
    pub source_commit_sha: String,
    pub integration_state: String,
    pub integration_commit_sha: Option<String>,
    pub pull_request_number: Option<u64>,
    pub pull_request_url: Option<String>,
    pub note: Option<String>,
    pub evidence_wait: Option<TaskEvidenceDeliveryStage>,
    pub evidence_available: bool,
    pub evidence_observations: Vec<TaskDeliveryEvidenceObservation>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskResultPrRelation {
    Source,
    Remediation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskResultPullRequest {
    pub record_id: String,
    pub source_task_id: String,
    pub work_order_id: Option<String>,
    pub relation: TaskResultPrRelation,
    pub number: u64,
    pub url: String,
    pub state: String,
    pub phase: String,
    pub reason: String,
    pub blocker: Option<String>,
    pub next_action: String,
    pub current: Option<bool>,
    pub successor_record_id: Option<String>,
    pub merge_sha: Option<String>,
    pub integration_method: Option<String>,
    pub integration_evidence_sha: Option<String>,
}
