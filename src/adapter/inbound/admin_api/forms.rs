use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::application::service::planning::{
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminOverview,
    PlanningAdminSessionView,
};

#[derive(Debug, Clone, Deserialize)]
pub(super) struct EditorQuery {
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    #[serde(default)]
    pub(super) notice: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CreateDraftForm {
    pub(super) csrf_token: String,
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DraftMutationForm {
    pub(super) csrf_token: String,
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    #[serde(flatten)]
    pub(super) values: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ResetForm {
    pub(super) csrf_token: String,
    pub(super) target: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DirectionMutationForm {
    pub(super) csrf_token: String,
    #[serde(default)]
    pub(super) id: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) summary: String,
    #[serde(default)]
    pub(super) success_criteria_text: String,
    #[serde(default)]
    pub(super) scope_hints_text: String,
    #[serde(default)]
    pub(super) detail_doc_path: String,
    #[serde(default)]
    pub(super) state: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct IdDeleteForm {
    pub(super) csrf_token: String,
    pub(super) id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TaskMutationForm {
    pub(super) csrf_token: String,
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) direction_id: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) base_priority: String,
    #[serde(default)]
    pub(super) dynamic_priority_delta: String,
    #[serde(default)]
    pub(super) priority_reason: String,
    #[serde(default)]
    pub(super) depends_on_text: String,
    #[serde(default)]
    pub(super) blocked_by_text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct FileSyncForm {
    pub(super) csrf_token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CreateDraftRequest {
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SaveDraftRequest {
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    #[serde(default)]
    pub(super) files: Vec<PlanningAdminDraftFileUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ResetRequest {
    pub(super) target: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct OverviewApiResponse {
    pub(super) csrf_token: String,
    pub(super) overview: PlanningAdminOverview,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DraftPromoteApiResponse {
    pub(super) promoted_file_count: usize,
    pub(super) is_valid: bool,
    pub(super) session: PlanningAdminSessionView,
}
