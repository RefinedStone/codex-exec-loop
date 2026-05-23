use crate::application::service::planning::{
    ManualPromptIntakeRequest, PlanningRuntimeProjection, PlanningServices,
};
use crate::domain::planning::{
    ManualPlanningBootstrapFailureKind as DomainManualPlanningBootstrapFailureKind,
    ManualPlanningBootstrapReview as DomainManualPlanningBootstrapReview,
    ManualPromptOutcome as DomainManualPromptOutcome,
    ManualPromptRequest as DomainManualPromptRequest,
};

pub type ManualPromptPreparationRequest = DomainManualPromptRequest;
pub type ManualPlanningBootstrapReview = DomainManualPlanningBootstrapReview;
pub type ManualPlanningBootstrapFailureKind = DomainManualPlanningBootstrapFailureKind;
pub type ManualPromptPreparationResult = DomainManualPromptOutcome;

#[derive(Clone)]
pub struct ManualPromptPreparationService {
    planning: PlanningServices,
}

impl ManualPromptPreparationService {
    pub fn new(planning: PlanningServices) -> Self {
        Self { planning }
    }

    pub fn prepare(
        &self,
        request: ManualPromptPreparationRequest,
    ) -> ManualPromptPreparationResult {
        let runtime_projection = self
            .planning
            .runtime
            .load_runtime_projection_or_invalid(&request.workspace_directory);
        self.prepare_with_runtime_projection(request, runtime_projection)
    }

    fn prepare_with_runtime_projection(
        &self,
        request: ManualPromptPreparationRequest,
        mut runtime_projection: PlanningRuntimeProjection,
    ) -> ManualPromptPreparationResult {
        let transcript_text = request.raw_prompt.trim().to_string();
        if transcript_text.is_empty() {
            return ManualPromptPreparationResult::Rejected {
                transcript_text,
                runtime_projection: Box::new(runtime_projection),
                reason: "manual prompt is empty".to_string(),
            };
        }

        if !runtime_projection.workspace_present() {
            let workspace_preparation = self.ensure_manual_planning_workspace(
                &request.workspace_directory,
                &transcript_text,
                runtime_projection,
            );
            match workspace_preparation {
                ManualWorkspacePreparation::Ready(prepared_projection) => {
                    runtime_projection = *prepared_projection;
                }
                ManualWorkspacePreparation::Blocked(result) => {
                    return result;
                }
            };
        }

        let intake =
            self.planning
                .runtime
                .prepare_manual_prompt_intake(ManualPromptIntakeRequest {
                    workspace_directory: request.workspace_directory,
                    raw_prompt: transcript_text.clone(),
                    legacy_source_turn_id: None,
                    parent_thread_id: request.parent_thread_id,
                    parent_turn_id: request.parent_turn_id,
                });
        ManualPromptPreparationResult::PromptReady {
            transcript_text,
            runtime_projection: Box::new(runtime_projection),
            intake: Box::new(intake),
        }
    }

    fn ensure_manual_planning_workspace(
        &self,
        workspace_directory: &str,
        transcript_text: &str,
        initial_projection: PlanningRuntimeProjection,
    ) -> ManualWorkspacePreparation {
        let stage_result = match self
            .planning
            .workspace
            .stage_simple_mode_draft(workspace_directory)
        {
            Ok(stage_result) => stage_result,
            Err(error) => {
                return ManualWorkspacePreparation::Blocked(
                    ManualPromptPreparationResult::BootstrapFailed {
                        transcript_text: transcript_text.to_string(),
                        runtime_projection: Box::new(initial_projection),
                        kind: ManualPlanningBootstrapFailureKind::Stage,
                        reason: error.to_string(),
                    },
                );
            }
        };
        let promote_result = match self
            .planning
            .workspace
            .promote_staged_draft(workspace_directory, &stage_result.draft_name)
        {
            Ok(promote_result) => promote_result,
            Err(error) => {
                return ManualWorkspacePreparation::Blocked(
                    ManualPromptPreparationResult::BootstrapFailed {
                        transcript_text: transcript_text.to_string(),
                        runtime_projection: Box::new(initial_projection),
                        kind: ManualPlanningBootstrapFailureKind::Promote,
                        reason: error.to_string(),
                    },
                );
            }
        };
        let runtime_projection = self
            .planning
            .runtime
            .load_runtime_projection_or_invalid(workspace_directory);
        if promote_result.promoted_file_count > 0 {
            return ManualWorkspacePreparation::Ready(Box::new(runtime_projection));
        }

        ManualWorkspacePreparation::Blocked(
            ManualPromptPreparationResult::BootstrapReviewRequired {
                transcript_text: transcript_text.to_string(),
                runtime_projection: Box::new(runtime_projection),
                review: ManualPlanningBootstrapReview {
                    draft_name: stage_result.draft_name,
                    staged_file_count: stage_result.staged_file_count,
                    validation_report: stage_result.validation_report,
                },
            },
        )
    }
}

enum ManualWorkspacePreparation {
    Ready(Box<PlanningRuntimeProjection>),
    Blocked(ManualPromptPreparationResult),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
        PlanningDraftStageRecord, PlanningStagedFileRecord, PlanningWorkspaceLoadRecord,
        PlanningWorkspacePort,
    };
    use crate::application::service::planning::PlanningRuntimeWorkspaceStatus;
    use anyhow::{Result, anyhow};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Clone, Copy)]
    enum WorkspaceBehavior {
        Normal,
        StageFailure,
        PromoteLoadFailure,
        InvalidPromoteDraft,
    }

    #[derive(Debug)]
    struct TestPlanningWorkspacePort {
        behavior: WorkspaceBehavior,
        staged_drafts: Mutex<HashMap<String, PlanningDraftLoadRecord>>,
        active_files: Mutex<HashMap<String, String>>,
    }

    impl TestPlanningWorkspacePort {
        fn new(behavior: WorkspaceBehavior) -> Self {
            Self {
                behavior,
                staged_drafts: Mutex::new(HashMap::new()),
                active_files: Mutex::new(HashMap::new()),
            }
        }
    }

    impl PlanningWorkspacePort for TestPlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            draft_name: &str,
            files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            if matches!(self.behavior, WorkspaceBehavior::StageFailure) {
                return Err(anyhow!("stage unavailable"));
            }
            let staged_files = files
                .iter()
                .map(|file| PlanningStagedFileRecord {
                    active_path: file.active_path.clone(),
                    staged_path: format!("{draft_name}/{}", file.active_path),
                })
                .collect::<Vec<_>>();
            let loaded_files = match self.behavior {
                WorkspaceBehavior::InvalidPromoteDraft => Vec::new(),
                _ => files
                    .iter()
                    .map(|file| PlanningDraftLoadFileRecord {
                        active_path: file.active_path.clone(),
                        staged_path: format!("{draft_name}/{}", file.active_path),
                        body: file.body.clone(),
                    })
                    .collect(),
            };
            self.staged_drafts
                .lock()
                .expect("staged draft store should not be poisoned")
                .insert(
                    draft_name.to_string(),
                    PlanningDraftLoadRecord {
                        draft_name: draft_name.to_string(),
                        draft_directory: format!("/tmp/{draft_name}"),
                        staged_files: loaded_files,
                    },
                );
            Ok(PlanningDraftStageRecord {
                draft_name: draft_name.to_string(),
                draft_directory: format!("/tmp/{draft_name}"),
                staged_files,
            })
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            if matches!(self.behavior, WorkspaceBehavior::PromoteLoadFailure) {
                return Err(anyhow!("draft load unavailable"));
            }
            self.staged_drafts
                .lock()
                .expect("staged draft store should not be poisoned")
                .get(draft_name)
                .cloned()
                .ok_or_else(|| anyhow!("missing draft {draft_name}"))
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!("draft replacement is not used by these tests"))
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(PlanningWorkspaceLoadRecord {
                result_output_markdown: self
                    .active_files
                    .lock()
                    .expect("active file store should not be poisoned")
                    .get(crate::application::service::planning::RESULT_OUTPUT_FILE_PATH)
                    .cloned(),
            })
        }

        fn load_planning_workspace_candidate_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(PlanningWorkspaceLoadRecord::default())
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            let mut active_files = self
                .active_files
                .lock()
                .expect("active file store should not be poisoned");
            if let Some(body) = record.result_output_markdown.as_ref() {
                active_files.insert(
                    crate::application::service::planning::RESULT_OUTPUT_FILE_PATH.to_string(),
                    body.clone(),
                );
            }
            Ok(())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            Ok(self
                .active_files
                .lock()
                .expect("active file store should not be poisoned")
                .get(relative_path)
                .cloned())
        }

        fn load_optional_planning_candidate_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            Ok(None)
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
            body: Option<&str>,
        ) -> Result<()> {
            let mut active_files = self
                .active_files
                .lock()
                .expect("active file store should not be poisoned");
            match body {
                Some(body) => {
                    active_files.insert(relative_path.to_string(), body.to_string());
                }
                None => {
                    active_files.remove(relative_path);
                }
            }
            Ok(())
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<()> {
            self.active_files
                .lock()
                .expect("active file store should not be poisoned")
                .remove(relative_path);
            Ok(())
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            archive_name: &str,
            active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Ok(format!("{archive_name}/{active_path}"))
        }
    }

    fn service_for(behavior: WorkspaceBehavior) -> ManualPromptPreparationService {
        ManualPromptPreparationService::new(PlanningServices::from_ports(
            Arc::new(TestPlanningWorkspacePort::new(behavior)),
            Arc::new(NoopPlanningAuthorityPort::default()),
            Arc::new(NoopPlanningTaskRepositoryPort),
            Arc::new(NoopPlanningWorkerPort),
        ))
    }

    fn unique_workspace(label: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        format!("/tmp/akra-manual-prompt-preparation-{label}-{nanos}")
    }

    fn request(raw_prompt: &str) -> ManualPromptPreparationRequest {
        ManualPromptPreparationRequest {
            workspace_directory: unique_workspace("prepare"),
            raw_prompt: raw_prompt.to_string(),
            parent_thread_id: Some("thread-parent".to_string()),
            parent_turn_id: Some("turn-parent".to_string()),
        }
    }

    #[test]
    fn test_workspace_port_supports_remaining_trait_edges() {
        let port = TestPlanningWorkspacePort::new(WorkspaceBehavior::Normal);
        let workspace = unique_workspace("port-edges");

        assert!(
            port.replace_planning_draft_file(&workspace, "draft", "active.md", "body")
                .is_err()
        );
        assert_eq!(
            port.load_planning_workspace_candidate_files(&workspace)
                .expect("candidate load should succeed"),
            PlanningWorkspaceLoadRecord::default()
        );
        assert_eq!(
            port.load_optional_planning_candidate_file(&workspace, "candidate.md")
                .expect("candidate file load should succeed"),
            None
        );

        port.replace_planning_workspace_file(&workspace, "scratch.md", Some("body"))
            .expect("active write should succeed");
        assert_eq!(
            port.load_optional_planning_file(&workspace, "scratch.md")
                .expect("active file should load")
                .as_deref(),
            Some("body")
        );
        port.replace_planning_workspace_file(&workspace, "scratch.md", None)
            .expect("active delete should succeed");
        assert_eq!(
            port.load_optional_planning_file(&workspace, "scratch.md")
                .expect("deleted active file lookup should succeed"),
            None
        );

        port.replace_planning_workspace_file(&workspace, "remove.md", Some("body"))
            .expect("active write should succeed");
        port.remove_planning_workspace_entry(&workspace, "remove.md")
            .expect("active removal should succeed");
        assert_eq!(
            port.load_optional_planning_file(&workspace, "remove.md")
                .expect("removed active file lookup should succeed"),
            None
        );
        assert_eq!(
            port.archive_rejected_planning_file(&workspace, "archive", "result.md", "body")
                .expect("archive path should render"),
            "archive/result.md"
        );
    }

    #[test]
    fn prepare_rejects_blank_prompt_after_projection_load() {
        let outcome = service_for(WorkspaceBehavior::Normal).prepare(request(" \n\t "));

        assert!(matches!(
            outcome,
            ManualPromptPreparationResult::Rejected {
                transcript_text,
                reason,
                runtime_projection,
            } if transcript_text.is_empty()
                && reason == "manual prompt is empty"
                && runtime_projection.workspace_present()
        ));
    }

    #[test]
    fn prepare_trims_prompt_and_returns_intake_payload() {
        let outcome = service_for(WorkspaceBehavior::Normal).prepare(request("  ship it  "));

        assert!(matches!(
            outcome,
            ManualPromptPreparationResult::PromptReady {
                transcript_text,
                runtime_projection,
                intake,
            } if transcript_text == "ship it"
                && runtime_projection.workspace_present()
                && std::mem::discriminant(intake.as_ref())
                    != std::mem::discriminant(
                        &crate::application::service::planning::ManualPromptIntakeOutcome::Rejected {
                            reason: String::new(),
                        },
                    )
        ));
    }

    #[test]
    fn prepare_bootstraps_explicit_uninitialized_projection_before_intake() {
        let outcome = service_for(WorkspaceBehavior::Normal).prepare_with_runtime_projection(
            request("  bootstrap planning  "),
            PlanningRuntimeProjection::uninitialized(),
        );

        assert!(matches!(
            outcome,
            ManualPromptPreparationResult::PromptReady {
                transcript_text,
                runtime_projection,
                ..
            } if transcript_text == "bootstrap planning"
                && runtime_projection.workspace_present()
        ));
    }

    #[test]
    fn prepare_returns_bootstrap_blocker_for_explicit_uninitialized_projection() {
        let outcome = service_for(WorkspaceBehavior::StageFailure).prepare_with_runtime_projection(
            request("bootstrap planning"),
            PlanningRuntimeProjection::uninitialized(),
        );

        assert!(matches!(
            outcome,
            ManualPromptPreparationResult::BootstrapFailed {
                kind: ManualPlanningBootstrapFailureKind::Stage,
                transcript_text,
                reason,
                ..
            } if transcript_text == "bootstrap planning"
                && reason == "stage unavailable"
        ));
    }

    #[test]
    fn manual_workspace_bootstrap_reports_stage_failure() {
        let service = service_for(WorkspaceBehavior::StageFailure);

        let result = service.ensure_manual_planning_workspace(
            &unique_workspace("stage-failure"),
            "start planning",
            PlanningRuntimeProjection::uninitialized(),
        );

        assert!(matches!(
            result,
            ManualWorkspacePreparation::Blocked(ManualPromptPreparationResult::BootstrapFailed {
                transcript_text,
                runtime_projection,
                kind: ManualPlanningBootstrapFailureKind::Stage,
                reason,
                ..
            }) if transcript_text == "start planning"
                && runtime_projection.workspace_status()
                    == PlanningRuntimeWorkspaceStatus::Uninitialized
                && reason == "stage unavailable"
        ));
    }

    #[test]
    fn manual_workspace_bootstrap_reports_promote_failure() {
        let service = service_for(WorkspaceBehavior::PromoteLoadFailure);

        let result = service.ensure_manual_planning_workspace(
            &unique_workspace("promote-failure"),
            "start planning",
            PlanningRuntimeProjection::uninitialized(),
        );

        assert!(matches!(
            result,
            ManualWorkspacePreparation::Blocked(ManualPromptPreparationResult::BootstrapFailed {
                transcript_text,
                kind: ManualPlanningBootstrapFailureKind::Promote,
                reason,
                ..
            }) if transcript_text == "start planning"
                && reason == "draft load unavailable"
        ));
    }

    #[test]
    fn manual_workspace_bootstrap_surfaces_review_when_nothing_promotes() {
        let service = service_for(WorkspaceBehavior::InvalidPromoteDraft);

        let result = service.ensure_manual_planning_workspace(
            &unique_workspace("review"),
            "start planning",
            PlanningRuntimeProjection::uninitialized(),
        );

        assert!(matches!(
            result,
            ManualWorkspacePreparation::Blocked(
                ManualPromptPreparationResult::BootstrapReviewRequired {
                    transcript_text,
                    runtime_projection,
                    review,
                }
            ) if transcript_text == "start planning"
                && runtime_projection.workspace_present()
                && review.draft_name.starts_with("bootstrap-")
                && review.staged_file_count > 0
                && review.validation_report.is_valid()
        ));
    }

    #[test]
    fn manual_workspace_bootstrap_ready_after_successful_promotion() {
        let service = service_for(WorkspaceBehavior::Normal);

        let result = service.ensure_manual_planning_workspace(
            &unique_workspace("ready"),
            "start planning",
            PlanningRuntimeProjection::uninitialized(),
        );

        assert!(matches!(
            result,
            ManualWorkspacePreparation::Ready(runtime_projection)
                if runtime_projection.workspace_present()
                    && runtime_projection.workspace_status()
                        != PlanningRuntimeWorkspaceStatus::Uninitialized
        ));
    }
}
