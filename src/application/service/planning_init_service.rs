use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningStagedFileRecord,
    PlanningWorkspacePort,
};
use crate::application::service::planning_contract::{
    DIRECTIONS_FILE_PATH, PLAN_OFF_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH,
};
use crate::domain::planning::PlanningValidationReport;

use super::planning_bootstrap_service::{PlanningBootstrapMode, PlanningBootstrapService};
use super::planning_validation_service::PlanningValidationService;

#[derive(Clone)]
pub struct PlanningInitService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
    planning_validation_service: PlanningValidationService,
}

#[derive(Debug, Clone)]
pub struct PlanningInitStageResult {
    pub mode: PlanningBootstrapMode,
    pub draft_name: String,
    pub draft_directory: String,
    pub staged_files: Vec<PlanningStagedFileRecord>,
    pub staged_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

impl PlanningInitStageResult {
    pub fn is_valid(&self) -> bool {
        self.validation_report.is_valid()
    }

    pub fn status_text(&self) -> String {
        format!(
            "planning init staged / mode: {} / draft: {} / files: {} / validation: {}",
            match self.mode {
                PlanningBootstrapMode::Detail => "detail",
                PlanningBootstrapMode::Simple => "simple",
            },
            self.draft_name,
            self.staged_file_count,
            if self.is_valid() {
                "ok"
            } else {
                "needs attention"
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftEditorFile {
    pub active_path: String,
    pub staged_path: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftEditorSession {
    pub draft_name: String,
    pub draft_directory: String,
    pub editable_files: Vec<PlanningDraftEditorFile>,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftSaveResult {
    pub draft_name: String,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftPromoteResult {
    pub draft_name: String,
    pub promoted_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

impl PlanningInitService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
        planning_validation_service: PlanningValidationService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_bootstrap_service,
            planning_validation_service,
        }
    }

    pub fn stage_simple_mode_draft(&self, workspace_dir: &str) -> Result<PlanningInitStageResult> {
        self.stage_draft(workspace_dir, PlanningBootstrapMode::Simple)
    }

    pub fn stage_manual_editor_session(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningDraftEditorSession> {
        self.stage_editor_session(workspace_dir, PlanningBootstrapMode::Detail)
    }

    pub fn load_manual_editor_session(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftEditorSession> {
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)?;
        let validation_report = self.validate_loaded_draft(&loaded);

        Ok(PlanningDraftEditorSession {
            draft_name: loaded.draft_name,
            draft_directory: loaded.draft_directory,
            editable_files: loaded
                .staged_files
                .into_iter()
                .filter(|file| is_operator_editable_draft_path(file.active_path.as_str()))
                .map(|file| PlanningDraftEditorFile {
                    active_path: file.active_path,
                    staged_path: file.staged_path,
                    body: file.body,
                })
                .collect(),
            validation_report,
        })
    }

    pub fn has_planning_workspace(&self, workspace_dir: &str) -> Result<bool> {
        Ok(self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?
            .has_any_files())
    }

    pub fn set_plan_enabled(&self, workspace_dir: &str, enabled: bool) -> Result<()> {
        if enabled {
            if !self.has_planning_workspace(workspace_dir)? {
                anyhow::bail!("planning workspace is unavailable; initialize planning first");
            }
            return self
                .planning_workspace_port
                .replace_planning_workspace_file(workspace_dir, PLAN_OFF_FILE_PATH, None);
        }

        if !self.has_planning_workspace(workspace_dir)? {
            anyhow::bail!("planning workspace is unavailable; initialize planning first");
        }

        self.planning_workspace_port
            .replace_planning_workspace_file(workspace_dir, PLAN_OFF_FILE_PATH, Some("plan off\n"))
    }

    pub fn save_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftSaveResult> {
        let loaded = self.replace_and_load_draft_editor_files(workspace_dir, draft_name, files)?;
        Ok(PlanningDraftSaveResult {
            draft_name: draft_name.to_string(),
            validation_report: self.validate_loaded_draft(&loaded),
        })
    }

    pub fn promote_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftPromoteResult> {
        let loaded = self.replace_and_load_draft_editor_files(workspace_dir, draft_name, files)?;
        self.promote_loaded_draft(workspace_dir, draft_name, loaded)
    }

    pub fn promote_staged_draft(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftPromoteResult> {
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)?;
        self.promote_loaded_draft(workspace_dir, draft_name, loaded)
    }

    fn replace_and_load_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftLoadRecord> {
        for file in files {
            self.planning_workspace_port.replace_planning_draft_file(
                workspace_dir,
                draft_name,
                &file.active_path,
                &file.body,
            )?;
        }

        self.planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)
    }

    fn promote_loaded_draft(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        loaded: PlanningDraftLoadRecord,
    ) -> Result<PlanningDraftPromoteResult> {
        let validation_report = self.validate_loaded_draft(&loaded);
        if !validation_report.is_valid() {
            return Ok(PlanningDraftPromoteResult {
                draft_name: draft_name.to_string(),
                promoted_file_count: 0,
                validation_report,
            });
        }
        for file in &loaded.staged_files {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    &file.active_path,
                    Some(file.body.as_str()),
                )?;
        }
        self.set_plan_enabled(workspace_dir, true)?;

        Ok(PlanningDraftPromoteResult {
            draft_name: draft_name.to_string(),
            promoted_file_count: loaded.staged_files.len(),
            validation_report,
        })
    }

    fn stage_draft(
        &self,
        workspace_dir: &str,
        mode: PlanningBootstrapMode,
    ) -> Result<PlanningInitStageResult> {
        let artifacts = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(mode);
        let mut validation_result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions_toml: &artifacts.directions_toml,
                task_ledger_json: &artifacts.task_ledger_json,
                task_ledger_schema_json: &artifacts.task_ledger_schema_json,
                result_output_markdown: &artifacts.result_output_markdown,
            },
        );
        if let Some(directions) = validation_result.directions.as_ref() {
            let staged_supporting_paths = artifacts
                .supplemental_files
                .iter()
                .map(|file| file.active_path.as_str())
                .collect::<Vec<_>>();
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| staged_supporting_paths.contains(&path),
                    &mut validation_result.report,
                );
        }

        let draft_name = build_bootstrap_draft_name(Utc::now());
        let mut staged_files = vec![
            PlanningDraftFileRecord {
                active_path: artifacts.directions_path,
                body: artifacts.directions_toml,
            },
            PlanningDraftFileRecord {
                active_path: artifacts.task_ledger_path,
                body: artifacts.task_ledger_json,
            },
            PlanningDraftFileRecord {
                active_path: artifacts.task_ledger_schema_path,
                body: artifacts.task_ledger_schema_json,
            },
            PlanningDraftFileRecord {
                active_path: artifacts.result_output_path,
                body: artifacts.result_output_markdown,
            },
        ];
        staged_files.extend(artifacts.supplemental_files.into_iter().map(Into::into));
        let stage_record = self.planning_workspace_port.stage_planning_draft_files(
            workspace_dir,
            &draft_name,
            &staged_files,
        )?;

        Ok(PlanningInitStageResult {
            mode,
            draft_name: stage_record.draft_name,
            draft_directory: stage_record.draft_directory,
            staged_files: stage_record.staged_files.clone(),
            staged_file_count: stage_record.staged_files.len(),
            validation_report: validation_result.report,
        })
    }

    fn stage_editor_session(
        &self,
        workspace_dir: &str,
        mode: PlanningBootstrapMode,
    ) -> Result<PlanningDraftEditorSession> {
        let staged = self.stage_draft(workspace_dir, mode)?;
        self.load_manual_editor_session(workspace_dir, &staged.draft_name)
    }

    fn validate_loaded_draft(&self, loaded: &PlanningDraftLoadRecord) -> PlanningValidationReport {
        let staged_file_map = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<HashMap<_, _>>();
        let mut result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions_toml: staged_file_map
                    .get(DIRECTIONS_FILE_PATH)
                    .copied()
                    .unwrap_or_default(),
                task_ledger_json: staged_file_map
                    .get(TASK_LEDGER_FILE_PATH)
                    .copied()
                    .unwrap_or_default(),
                task_ledger_schema_json: staged_file_map
                    .get(TASK_LEDGER_SCHEMA_FILE_PATH)
                    .copied()
                    .unwrap_or_default(),
                result_output_markdown: staged_file_map
                    .get(RESULT_OUTPUT_FILE_PATH)
                    .copied()
                    .unwrap_or_default(),
            },
        );
        if let Some(directions) = result.directions.as_ref() {
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| staged_file_map.contains_key(path),
                    &mut result.report,
                );
        }

        result.report
    }
}

fn is_operator_editable_draft_path(active_path: &str) -> bool {
    matches!(
        active_path,
        DIRECTIONS_FILE_PATH | TASK_LEDGER_FILE_PATH | RESULT_OUTPUT_FILE_PATH
    )
}

fn build_bootstrap_draft_name(now: chrono::DateTime<Utc>) -> String {
    format!(
        "bootstrap-{}Z-{:09}",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use anyhow::Result;
    use chrono::{TimeZone, Timelike, Utc};

    use super::{PlanningInitService, build_bootstrap_draft_name};
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
        PlanningDraftStageRecord, PlanningStagedFileRecord, PlanningWorkspaceLoadRecord,
        PlanningWorkspacePort,
    };
    use crate::application::service::planning_bootstrap_service::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning_contract::{
        DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH,
        RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
    };
    use crate::application::service::planning_validation_service::PlanningValidationService;

    #[derive(Default)]
    struct FakePlanningWorkspacePort {
        staged_files: std::sync::Mutex<Vec<PlanningDraftFileRecord>>,
        draft_file_bodies: std::sync::Mutex<HashMap<String, String>>,
        active_file_bodies: std::sync::Mutex<HashMap<String, String>>,
    }

    impl PlanningWorkspacePort for FakePlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            draft_name: &str,
            files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            self.staged_files
                .lock()
                .expect("staged_files mutex should not be poisoned")
                .extend(files.iter().cloned());
            let mut draft_file_bodies = self
                .draft_file_bodies
                .lock()
                .expect("draft_file_bodies mutex should not be poisoned");
            for file in files {
                draft_file_bodies.insert(file.active_path.clone(), file.body.clone());
            }

            Ok(PlanningDraftStageRecord {
                draft_name: draft_name.to_string(),
                draft_directory: format!("/tmp/{draft_name}"),
                staged_files: files
                    .iter()
                    .map(|file| PlanningStagedFileRecord {
                        active_path: file.active_path.clone(),
                        staged_path: format!("/tmp/{draft_name}/{}", file.active_path),
                    })
                    .collect(),
            })
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            let draft_file_bodies = self
                .draft_file_bodies
                .lock()
                .expect("draft_file_bodies mutex should not be poisoned");
            let mut active_paths = draft_file_bodies.keys().cloned().collect::<Vec<_>>();
            active_paths.sort();
            Ok(PlanningDraftLoadRecord {
                draft_name: draft_name.to_string(),
                draft_directory: format!("/tmp/{draft_name}"),
                staged_files: active_paths
                    .into_iter()
                    .map(|active_path| PlanningDraftLoadFileRecord {
                        staged_path: format!("/tmp/{draft_name}/{active_path}"),
                        body: draft_file_bodies
                            .get(&active_path)
                            .cloned()
                            .unwrap_or_default(),
                        active_path,
                    })
                    .collect(),
            })
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            draft_name: &str,
            active_path: &str,
            body: &str,
        ) -> Result<String> {
            self.draft_file_bodies
                .lock()
                .expect("draft_file_bodies mutex should not be poisoned")
                .insert(active_path.to_string(), body.to_string());
            Ok(format!("/tmp/{draft_name}/{active_path}"))
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            let active_file_bodies = self
                .active_file_bodies
                .lock()
                .expect("active_file_bodies mutex should not be poisoned");
            Ok(PlanningWorkspaceLoadRecord {
                directions_toml: active_file_bodies.get(DIRECTIONS_FILE_PATH).cloned(),
                task_ledger_json: active_file_bodies.get(TASK_LEDGER_FILE_PATH).cloned(),
                task_ledger_schema_json: active_file_bodies
                    .get(TASK_LEDGER_SCHEMA_FILE_PATH)
                    .cloned(),
                queue_snapshot_json: active_file_bodies.get(QUEUE_SNAPSHOT_FILE_PATH).cloned(),
                result_output_markdown: active_file_bodies.get(RESULT_OUTPUT_FILE_PATH).cloned(),
            })
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            Ok(self
                .active_file_bodies
                .lock()
                .expect("active_file_bodies mutex should not be poisoned")
                .get(relative_path)
                .cloned())
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
            body: Option<&str>,
        ) -> Result<()> {
            self.active_file_bodies
                .lock()
                .expect("active_file_bodies mutex should not be poisoned")
                .insert(
                    relative_path.to_string(),
                    body.unwrap_or_default().to_string(),
                );
            Ok(())
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            unreachable!("archive writes are not used in planning init service tests")
        }
    }

    #[test]
    fn stage_bootstrap_draft_writes_expected_files_and_validates_them() {
        let workspace_port = Arc::new(FakePlanningWorkspacePort::default());
        let service = PlanningInitService::new(
            workspace_port.clone(),
            PlanningBootstrapService::new(),
            PlanningValidationService::new(),
        );

        let result = service
            .stage_draft("/tmp/workspace", PlanningBootstrapMode::Detail)
            .expect("bootstrap draft should stage");

        assert_eq!(result.mode, PlanningBootstrapMode::Detail);
        assert!(result.draft_name.starts_with("bootstrap-"));
        assert_eq!(result.staged_file_count, 4);
        assert_eq!(result.staged_files.len(), 4);
        assert!(result.is_valid(), "{:?}", result.validation_report.issues);
        let staged_files = workspace_port
            .staged_files
            .lock()
            .expect("staged_files mutex should not be poisoned");
        assert_eq!(staged_files.len(), 4);
    }

    #[test]
    fn stage_simple_mode_draft_uses_simple_bootstrap_contract() {
        let workspace_port = Arc::new(FakePlanningWorkspacePort::default());
        let service = PlanningInitService::new(
            workspace_port.clone(),
            PlanningBootstrapService::new(),
            PlanningValidationService::new(),
        );

        let result = service
            .stage_simple_mode_draft("/tmp/workspace")
            .expect("simple mode draft should stage");

        assert_eq!(result.mode, PlanningBootstrapMode::Simple);
        assert!(result.is_valid(), "{:?}", result.validation_report.issues);
        assert_eq!(result.staged_file_count, 5);
        let staged_files = workspace_port
            .staged_files
            .lock()
            .expect("staged_files mutex should not be poisoned");
        let directions_body = staged_files
            .iter()
            .find(|file| file.active_path.ends_with("directions.toml"))
            .map(|file| file.body.as_str())
            .expect("directions.toml should be staged");
        assert!(directions_body.contains("general-workstream"));
        assert!(directions_body.contains(r#"policy = "review_and_enqueue""#));
        assert!(
            staged_files
                .iter()
                .any(|file| file.active_path == DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
        );
    }

    #[test]
    fn stage_manual_editor_session_returns_only_editable_files() {
        let workspace_port = Arc::new(FakePlanningWorkspacePort::default());
        let service = PlanningInitService::new(
            workspace_port,
            PlanningBootstrapService::new(),
            PlanningValidationService::new(),
        );

        let session = service
            .stage_manual_editor_session("/tmp/workspace")
            .expect("manual editor session should stage");

        assert_eq!(session.editable_files.len(), 3);
        assert!(
            session
                .editable_files
                .iter()
                .any(|file| file.active_path == DIRECTIONS_FILE_PATH)
        );
        assert!(
            session
                .editable_files
                .iter()
                .any(|file| file.active_path == TASK_LEDGER_FILE_PATH)
        );
        assert!(
            session
                .editable_files
                .iter()
                .any(|file| file.active_path == RESULT_OUTPUT_FILE_PATH)
        );
    }

    #[test]
    fn stage_simple_editor_session_returns_promotable_simple_draft() {
        let workspace_port = Arc::new(FakePlanningWorkspacePort::default());
        let service = PlanningInitService::new(
            workspace_port,
            PlanningBootstrapService::new(),
            PlanningValidationService::new(),
        );

        let session = service
            .stage_editor_session("/tmp/workspace", PlanningBootstrapMode::Simple)
            .expect("simple editor session should stage");

        assert!(session.validation_report.is_valid());
        assert_eq!(session.editable_files.len(), 3);
        let directions = session
            .editable_files
            .iter()
            .find(|file| file.active_path == DIRECTIONS_FILE_PATH)
            .map(|file| file.body.as_str())
            .expect("directions file should remain editable");
        assert!(directions.contains("general-workstream"));
    }

    #[test]
    fn promote_draft_editor_files_writes_active_planning_files_when_valid() {
        let workspace_port = Arc::new(FakePlanningWorkspacePort::default());
        let service = PlanningInitService::new(
            workspace_port.clone(),
            PlanningBootstrapService::new(),
            PlanningValidationService::new(),
        );

        let session = service
            .stage_manual_editor_session("/tmp/workspace")
            .expect("manual editor session should stage");

        let result = service
            .promote_draft_editor_files(
                "/tmp/workspace",
                &session.draft_name,
                &session.editable_files,
            )
            .expect("valid staged draft should promote");

        assert!(result.validation_report.is_valid());
        assert_eq!(result.promoted_file_count, 4);
        let active_files = workspace_port
            .active_file_bodies
            .lock()
            .expect("active_file_bodies mutex should not be poisoned");
        assert!(active_files.contains_key(DIRECTIONS_FILE_PATH));
        assert!(active_files.contains_key(TASK_LEDGER_FILE_PATH));
        assert!(active_files.contains_key(TASK_LEDGER_SCHEMA_FILE_PATH));
        assert!(active_files.contains_key(RESULT_OUTPUT_FILE_PATH));
    }

    #[test]
    fn promote_staged_draft_writes_active_planning_files_without_editor_buffers() {
        let workspace_port = Arc::new(FakePlanningWorkspacePort::default());
        let service = PlanningInitService::new(
            workspace_port.clone(),
            PlanningBootstrapService::new(),
            PlanningValidationService::new(),
        );

        let staged = service
            .stage_simple_mode_draft("/tmp/workspace")
            .expect("simple staged draft should be created");

        let result = service
            .promote_staged_draft("/tmp/workspace", &staged.draft_name)
            .expect("valid staged draft should promote");

        assert!(result.validation_report.is_valid());
        assert_eq!(result.promoted_file_count, 5);
        let active_files = workspace_port
            .active_file_bodies
            .lock()
            .expect("active_file_bodies mutex should not be poisoned");
        assert!(active_files.contains_key(DIRECTIONS_FILE_PATH));
        assert!(active_files.contains_key(TASK_LEDGER_FILE_PATH));
        assert!(active_files.contains_key(TASK_LEDGER_SCHEMA_FILE_PATH));
        assert!(active_files.contains_key(RESULT_OUTPUT_FILE_PATH));
        assert!(active_files.contains_key(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH));
    }

    #[test]
    fn bootstrap_draft_name_keeps_same_second_runs_distinct() {
        let first_timestamp = Utc
            .with_ymd_and_hms(2026, 4, 9, 12, 0, 0)
            .single()
            .expect("timestamp should be valid")
            .with_nanosecond(123_456_789)
            .expect("nanoseconds should be valid");
        let second_timestamp = Utc
            .with_ymd_and_hms(2026, 4, 9, 12, 0, 0)
            .single()
            .expect("timestamp should be valid")
            .with_nanosecond(987_654_321)
            .expect("nanoseconds should be valid");

        let first_name = build_bootstrap_draft_name(first_timestamp);
        let second_name = build_bootstrap_draft_name(second_timestamp);

        assert_ne!(first_name, second_name);
        assert!(first_name.starts_with("bootstrap-20260409T120000Z-"));
        assert!(second_name.starts_with("bootstrap-20260409T120000Z-"));
    }
}
