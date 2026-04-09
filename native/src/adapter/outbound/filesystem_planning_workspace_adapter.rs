use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftStageRecord, PlanningStagedFileRecord,
    PlanningWorkspacePort,
};
use crate::domain::planning::PLANNING_DRAFTS_DIRECTORY;

#[derive(Default)]
pub struct FilesystemPlanningWorkspaceAdapter;

impl FilesystemPlanningWorkspaceAdapter {
    pub fn new() -> Self {
        Self
    }

    fn draft_directory(workspace_dir: &str, draft_name: &str) -> PathBuf {
        Path::new(workspace_dir)
            .join(PLANNING_DRAFTS_DIRECTORY)
            .join(draft_name)
    }
}

impl PlanningWorkspacePort for FilesystemPlanningWorkspaceAdapter {
    fn stage_planning_draft_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord> {
        let draft_directory = Self::draft_directory(workspace_dir, draft_name);
        fs::create_dir_all(&draft_directory)
            .with_context(|| format!("failed to create {}", draft_directory.display()))?;

        let staged_files = files
            .iter()
            .map(|file| {
                let active_relative_path = Path::new(file.active_path.as_str());
                let file_name = active_relative_path
                    .file_name()
                    .with_context(|| {
                        format!("planning draft file has no file name: {}", file.active_path)
                    })?
                    .to_owned();
                let staged_path = draft_directory.join(file_name);
                fs::write(&staged_path, &file.body)
                    .with_context(|| format!("failed to write {}", staged_path.display()))?;

                Ok(PlanningStagedFileRecord {
                    active_path: file.active_path.clone(),
                    staged_path: staged_path.display().to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(PlanningDraftStageRecord {
            draft_name: draft_name.to_string(),
            draft_directory: draft_directory.display().to_string(),
            staged_files,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningWorkspacePort,
    };

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    #[test]
    fn stages_planning_files_under_drafts_directory() {
        let workspace_dir = create_temp_workspace("planning-workspace-adapter");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let result = adapter
            .stage_planning_draft_files(
                &workspace_dir,
                "bootstrap-20260409T120000Z",
                &[
                    PlanningDraftFileRecord {
                        active_path: ".codex-exec-loop/planning/directions.toml".to_string(),
                        body: "version = 1".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: ".codex-exec-loop/planning/task-ledger.json".to_string(),
                        body: "{\"version\":1,\"tasks\":[]}".to_string(),
                    },
                ],
            )
            .expect("planning draft files should stage");

        assert!(
            result
                .draft_directory
                .contains(".codex-exec-loop/planning/drafts/bootstrap-20260409T120000Z")
        );
        assert_eq!(result.staged_files.len(), 2);
        for staged_file in result.staged_files {
            assert!(Path::new(&staged_file.staged_path).exists());
        }

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
