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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftLoadFileRecord {
    pub active_path: String,
    pub staged_path: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftLoadRecord {
    pub draft_name: String,
    pub draft_directory: String,
    pub staged_files: Vec<PlanningDraftLoadFileRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanningWorkspaceLoadRecord {
    pub result_output_markdown: Option<String>,
}

impl PlanningWorkspaceLoadRecord {
    pub fn has_any_files(&self) -> bool {
        self.result_output_markdown.is_some()
    }
}

pub trait PlanningWorkspacePort: Send + Sync {
    fn stage_planning_draft_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord>;

    fn load_planning_draft_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord>;

    fn replace_planning_draft_file(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        active_path: &str,
        body: &str,
    ) -> Result<String>;

    fn load_planning_workspace_files(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    fn load_planning_workspace_candidate_files(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    fn commit_planning_workspace_files(
        &self,
        workspace_dir: &str,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()>;

    fn load_optional_planning_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>>;

    fn load_optional_planning_candidate_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>>;

    fn replace_planning_workspace_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()>;

    fn remove_planning_workspace_entry(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<()>;

    fn archive_rejected_planning_file(
        &self,
        workspace_dir: &str,
        archive_name: &str,
        active_path: &str,
        body: &str,
    ) -> Result<String>;
}
