use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningWorkspacePort,
};
use crate::domain::planning::PlanningValidationReport;

use super::planning_bootstrap_service::PlanningBootstrapService;
use super::planning_validation_service::PlanningValidationService;

#[derive(Clone)]
pub struct PlanningInitService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
    planning_validation_service: PlanningValidationService,
}

#[derive(Debug, Clone)]
pub struct PlanningInitStageResult {
    pub draft_name: String,
    pub draft_directory: String,
    pub staged_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

impl PlanningInitStageResult {
    pub fn is_valid(&self) -> bool {
        self.validation_report.is_valid()
    }

    pub fn status_text(&self) -> String {
        format!(
            "planning init staged / draft: {} / files: {} / validation: {}",
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

    pub fn stage_bootstrap_draft(&self, workspace_dir: &str) -> Result<PlanningInitStageResult> {
        let artifacts = self.planning_bootstrap_service.build_artifacts();
        let validation_result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions_toml: &artifacts.directions_toml,
                task_ledger_json: &artifacts.task_ledger_json,
                task_ledger_schema_json: &artifacts.task_ledger_schema_json,
                result_output_markdown: &artifacts.result_output_markdown,
            },
        );

        let draft_name = build_bootstrap_draft_name(Utc::now());
        let stage_record = self.planning_workspace_port.stage_planning_draft_files(
            workspace_dir,
            &draft_name,
            &[
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
            ],
        )?;

        Ok(PlanningInitStageResult {
            draft_name: stage_record.draft_name,
            draft_directory: stage_record.draft_directory,
            staged_file_count: stage_record.staged_files.len(),
            validation_report: validation_result.report,
        })
    }
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
    use std::sync::Arc;

    use anyhow::Result;
    use chrono::{TimeZone, Timelike, Utc};

    use super::{PlanningInitService, build_bootstrap_draft_name};
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftStageRecord, PlanningStagedFileRecord,
        PlanningWorkspacePort,
    };
    use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
    use crate::application::service::planning_validation_service::PlanningValidationService;

    #[derive(Default)]
    struct FakePlanningWorkspacePort {
        staged_files: std::sync::Mutex<Vec<PlanningDraftFileRecord>>,
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
            .stage_bootstrap_draft("/tmp/workspace")
            .expect("bootstrap draft should stage");

        assert!(result.draft_name.starts_with("bootstrap-"));
        assert_eq!(result.staged_file_count, 4);
        assert!(result.is_valid(), "{:?}", result.validation_report.issues);
        let staged_files = workspace_port
            .staged_files
            .lock()
            .expect("staged_files mutex should not be poisoned");
        assert_eq!(staged_files.len(), 4);
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
