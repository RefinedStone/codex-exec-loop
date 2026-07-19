use crate::domain::planning::QueueIdlePolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionsSupportingFileStatus {
    MissingMapping,
    Ready,
    BrokenMapping,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceDirectionSnapshot {
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_status: DirectionsSupportingFileStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceSummarySnapshot {
    pub directions: Vec<DirectionsMaintenanceDirectionSnapshot>,
    pub missing_detail_doc_count: usize,
    pub broken_detail_doc_count: usize,
    pub queue_idle_policy: QueueIdlePolicy,
    pub queue_idle_prompt_path: Option<String>,
    pub queue_idle_prompt_status: DirectionsSupportingFileStatus,
    pub parse_error: Option<String>,
}
