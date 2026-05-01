use std::path::PathBuf;

use anyhow::Result;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
    PlanningWorkspaceLoadRecord, RepoScopedPlanningWorkspacePort,
};

use super::SqlitePlanningAuthorityAdapter;

impl RepoScopedPlanningWorkspacePort for SqlitePlanningAuthorityAdapter {
    fn is_git_backed_workspace(&self, workspace_dir: &str) -> bool {
        Self::is_git_backed_workspace(workspace_dir)
    }

    fn resolve_active_workspace_root(&self, workspace_dir: &str) -> PathBuf {
        Self::resolve_active_workspace_root(workspace_dir)
    }

    fn stage_repo_scoped_draft_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord> {
        Self::stage_repo_scoped_draft_files(workspace_dir, draft_name, files)
    }

    fn load_repo_scoped_draft_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord> {
        Self::load_repo_scoped_draft_files(workspace_dir, draft_name)
    }

    fn replace_repo_scoped_draft_file(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        active_path: &str,
        body: &str,
    ) -> Result<String> {
        Self::replace_repo_scoped_draft_file(workspace_dir, draft_name, active_path, body)
    }

    fn load_active_workspace_files(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        Self::load_active_workspace_files(workspace_dir)
    }

    fn commit_active_workspace_files(
        &self,
        workspace_dir: &str,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        Self::commit_active_workspace_files(workspace_dir, record)
    }

    fn load_active_planning_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        Self::load_active_planning_file(workspace_dir, relative_path)
    }

    fn replace_active_planning_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()> {
        Self::replace_active_planning_file(workspace_dir, relative_path, body)
    }

    fn remove_active_planning_entry(&self, workspace_dir: &str, relative_path: &str) -> Result<()> {
        Self::remove_active_planning_entry(workspace_dir, relative_path)
    }
}
