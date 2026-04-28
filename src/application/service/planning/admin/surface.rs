use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningAdminDraftKind {
    FullPlanning,
    QueueIdlePrompt,
    DirectionDetail,
}

impl PlanningAdminDraftKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::FullPlanning => "full planning",
            Self::QueueIdlePrompt => "queue-idle prompt",
            Self::DirectionDetail => "direction detail",
        }
    }

    pub fn editor_heading(self) -> &'static str {
        match self {
            Self::FullPlanning => "Full Planning Draft",
            Self::QueueIdlePrompt => "Queue-Idle Prompt Draft",
            Self::DirectionDetail => "Direction Detail Draft",
        }
    }

    pub fn return_path(self) -> &'static str {
        match self {
            Self::FullPlanning => "/admin",
            Self::QueueIdlePrompt | Self::DirectionDetail => "/admin/directions",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::FullPlanning => "full_planning",
            Self::QueueIdlePrompt => "queue_idle_prompt",
            Self::DirectionDetail => "direction_detail",
        }
    }
}

impl fmt::Display for PlanningAdminDraftKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PlanningAdminFileKey {
    ResultOutput,
    QueueIdlePrompt,
    DirectionDetail,
}

impl PlanningAdminFileKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::ResultOutput => "Result Output",
            Self::QueueIdlePrompt => "Queue-Idle Prompt",
            Self::DirectionDetail => "Direction Detail",
        }
    }

    pub fn editor_language(self) -> &'static str {
        match self {
            Self::ResultOutput | Self::QueueIdlePrompt | Self::DirectionDetail => "markdown",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::ResultOutput => "result_output",
            Self::QueueIdlePrompt => "queue_idle_prompt",
            Self::DirectionDetail => "direction_detail",
        }
    }
}

impl fmt::Display for PlanningAdminFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftLoadRequest {
    pub draft_name: String,
    pub kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub direction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftFileUpdate {
    pub key: PlanningAdminFileKey,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftMutationRequest {
    pub draft_name: String,
    pub kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub direction_id: Option<String>,
    #[serde(default)]
    pub files: Vec<PlanningAdminDraftFileUpdate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminValidationIssueView {
    pub severity: String,
    pub file_kind: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminValidationView {
    pub is_valid: bool,
    pub error_count: usize,
    pub warning_count: usize,
    pub issues: Vec<PlanningAdminValidationIssueView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueueTaskView {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueueHeadView {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub rank_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueuePreview {
    pub queue_summary: String,
    pub proposal_summary: Option<String>,
    pub queue_head: Option<PlanningAdminQueueHeadView>,
    pub visible_tasks: Vec<PlanningAdminQueueTaskView>,
    pub proposed_tasks: Vec<PlanningAdminQueueTaskView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDraftFileView {
    pub key: PlanningAdminFileKey,
    pub label: String,
    pub active_path: String,
    pub editor_language: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminSessionView {
    pub kind: PlanningAdminDraftKind,
    pub direction_id: Option<String>,
    pub draft_name: String,
    pub draft_directory: String,
    pub editor_heading: String,
    pub return_path: String,
    pub files: Vec<PlanningAdminDraftFileView>,
    pub validation: PlanningAdminValidationView,
    pub queue_preview: Option<PlanningAdminQueuePreview>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDirectionSummaryView {
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_status: String,
    pub needs_attention: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDirectionsSummaryView {
    pub missing_detail_doc_count: usize,
    pub broken_detail_doc_count: usize,
    pub queue_idle_policy: String,
    pub queue_idle_prompt_path: Option<String>,
    pub queue_idle_prompt_status: String,
    pub parse_error: Option<String>,
    pub directions: Vec<PlanningAdminDirectionSummaryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDirectionManagementView {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub success_criteria_text: String,
    pub scope_hints_text: String,
    pub detail_doc_path: String,
    pub state: String,
    pub task_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminTaskManagementView {
    pub id: String,
    pub direction_id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub base_priority: i32,
    pub dynamic_priority_delta: i32,
    pub priority_reason: String,
    pub depends_on_text: String,
    pub blocked_by_text: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminManagementView {
    pub default_direction_id: String,
    pub directions: Vec<PlanningAdminDirectionManagementView>,
    pub tasks: Vec<PlanningAdminTaskManagementView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDirectionMutationRequest {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub success_criteria_text: String,
    #[serde(default)]
    pub scope_hints_text: String,
    #[serde(default)]
    pub detail_doc_path: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDirectionDeleteRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminTaskMutationRequest {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub direction_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub base_priority: String,
    #[serde(default)]
    pub dynamic_priority_delta: String,
    #[serde(default)]
    pub priority_reason: String,
    #[serde(default)]
    pub depends_on_text: String,
    #[serde(default)]
    pub blocked_by_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminTaskDeleteRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminCrudOutcome {
    pub notice: String,
    pub management: PlanningAdminManagementView,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminFileSyncOutcome {
    pub notice: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDoctorSummary {
    pub planning_state: String,
    pub queue_idle_policy: Option<String>,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub health: Option<String>,
    pub issue: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminRuntimeSummary {
    pub workspace_present: bool,
    pub preview_status_label: String,
    pub preview_detail: Option<String>,
    pub queue_head: Option<PlanningAdminQueueHeadView>,
    pub visible_tasks: Vec<PlanningAdminQueueTaskView>,
    pub proposed_tasks: Vec<PlanningAdminQueueTaskView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminOverview {
    pub workspace_dir: String,
    pub doctor: PlanningAdminDoctorSummary,
    pub runtime: PlanningAdminRuntimeSummary,
    pub directions: Option<PlanningAdminDirectionsSummaryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminResetOutcome {
    pub target: String,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub doctor: PlanningAdminDoctorSummary,
}
