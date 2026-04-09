use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftFileRecord {
    pub active_path: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningStagedFileRecord {
    pub active_path: String,
    pub staged_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftStageRecord {
    pub draft_name: String,
    pub draft_directory: String,
    pub staged_files: Vec<PlanningStagedFileRecord>,
}

pub trait PlanningWorkspacePort: Send + Sync {
    fn stage_planning_draft_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord>;
}
