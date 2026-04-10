use anyhow::Result;

use crate::application::service::planning_prompt_service::{
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning_reconciliation_service::{
    PlanningExecutionSnapshot, PlanningReconciliationResult, PlanningReconciliationService,
};
use crate::application::service::planning_runtime_policy_service::{
    PlanningAutoFollowBlockReason, PlanningRuntimePolicyService, PlanningRuntimePreviewView,
    PlanningRuntimeSummaryRequest, PlanningRuntimeSummaryView,
};
use crate::application::service::turn_prompt_assembly_service::{
    AutoFollowPromptAssemblyRequest, AutoFollowPromptPreviewRequest, ManualPromptAssemblyRequest,
    TurnPromptAssemblyService,
};
use crate::domain::followup_template::FollowupTemplateDefinition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeAutoFollowRequest<'a> {
    pub template: &'a FollowupTemplateDefinition,
    pub auto_turn: usize,
    pub max_auto_turns: usize,
    pub session_id: &'a str,
    pub stop_keyword: &'a str,
    pub last_message: &'a str,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimePreviewRequest<'a> {
    pub template: &'a FollowupTemplateDefinition,
    pub auto_turn: usize,
    pub max_auto_turns: usize,
    pub session_id: &'a str,
    pub stop_keyword: &'a str,
    pub last_message: Option<&'a str>,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningRuntimeAutoFollowDecision {
    QueuePrompt(String),
    Blocked(PlanningAutoFollowBlockReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeRenderedPreview {
    pub rendered_prompt: String,
    pub planning_view: PlanningRuntimePreviewView,
}

#[derive(Clone)]
pub struct PlanningRuntimeFacadeService {
    planning_prompt_service: PlanningPromptService,
    planning_reconciliation_service: PlanningReconciliationService,
    planning_runtime_policy_service: PlanningRuntimePolicyService,
    turn_prompt_assembly_service: TurnPromptAssemblyService,
}

impl PlanningRuntimeFacadeService {
    pub fn new(
        planning_prompt_service: PlanningPromptService,
        planning_reconciliation_service: PlanningReconciliationService,
        planning_runtime_policy_service: PlanningRuntimePolicyService,
        turn_prompt_assembly_service: TurnPromptAssemblyService,
    ) -> Self {
        Self {
            planning_prompt_service,
            planning_reconciliation_service,
            planning_runtime_policy_service,
            turn_prompt_assembly_service,
        }
    }

    pub fn load_runtime_snapshot_or_invalid(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.planning_prompt_service
            .load_runtime_snapshot(workspace_directory)
            .unwrap_or_else(|error| {
                PlanningRuntimeSnapshot::invalid(format!(
                    "failed to load planning workspace: {error}"
                ))
            })
    }

    pub fn build_manual_prompt(
        &self,
        operator_prompt: &str,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<String> {
        self.turn_prompt_assembly_service
            .build_manual_prompt(ManualPromptAssemblyRequest {
                operator_prompt,
                planning_prompt_fragment: snapshot.prompt_fragment(),
            })
    }

    pub fn decide_auto_followup(
        &self,
        request: PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeAutoFollowDecision {
        if let Some(block_reason) = self
            .planning_runtime_policy_service
            .auto_follow_block_reason(request.template, request.snapshot)
        {
            return PlanningRuntimeAutoFollowDecision::Blocked(block_reason);
        }

        PlanningRuntimeAutoFollowDecision::QueuePrompt(
            self.turn_prompt_assembly_service.build_auto_follow_prompt(
                AutoFollowPromptAssemblyRequest {
                    template: request.template,
                    auto_turn: request.auto_turn,
                    max_auto_turns: request.max_auto_turns,
                    session_id: request.session_id,
                    stop_keyword: request.stop_keyword,
                    last_message: request.last_message,
                    planning_prompt_fragment: request.snapshot.prompt_fragment(),
                },
            ),
        )
    }

    pub fn build_auto_follow_preview(
        &self,
        request: PlanningRuntimePreviewRequest<'_>,
    ) -> PlanningRuntimeRenderedPreview {
        PlanningRuntimeRenderedPreview {
            rendered_prompt: self
                .turn_prompt_assembly_service
                .build_auto_follow_prompt_preview(AutoFollowPromptPreviewRequest {
                    template: request.template,
                    auto_turn: request.auto_turn,
                    max_auto_turns: request.max_auto_turns,
                    session_id: request.session_id,
                    stop_keyword: request.stop_keyword,
                    last_message: request.last_message,
                    planning_prompt_fragment: request.snapshot.prompt_fragment(),
                }),
            planning_view: self
                .planning_runtime_policy_service
                .build_preview_view(request.template, request.snapshot),
        }
    }

    pub fn build_summary_view(
        &self,
        request: PlanningRuntimeSummaryRequest<'_>,
    ) -> PlanningRuntimeSummaryView {
        self.planning_runtime_policy_service
            .build_summary_view(request)
    }

    pub fn load_execution_snapshot(
        &self,
        workspace_directory: &str,
    ) -> Result<PlanningExecutionSnapshot> {
        self.planning_reconciliation_service
            .load_execution_snapshot(workspace_directory)
    }

    pub fn reconcile_after_turn(
        &self,
        workspace_directory: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        self.planning_reconciliation_service.reconcile_after_turn(
            workspace_directory,
            turn_id,
            changed_planning_file_paths,
            execution_snapshot,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, anyhow};

    use super::{
        PlanningRuntimeAutoFollowDecision, PlanningRuntimeAutoFollowRequest,
        PlanningRuntimeFacadeService, PlanningRuntimePreviewRequest,
    };
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningStagedFileRecord, PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning_prompt_service::PlanningPromptService;
    use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
    use crate::application::service::planning_runtime_policy_service::{
        PlanningAutoFollowBlockReason, PlanningRuntimePolicyService,
    };
    use crate::application::service::planning_validation_service::PlanningValidationService;
    use crate::application::service::priority_queue_service::PriorityQueueService;
    use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
    use crate::domain::followup_template::{FollowupTemplateDefinition, FollowupTemplateSource};
    use crate::domain::planning::PriorityQueueTask;

    struct FakePlanningWorkspacePort {
        load_record: Option<PlanningWorkspaceLoadRecord>,
        load_error_message: Option<String>,
    }

    impl PlanningWorkspacePort for FakePlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            Ok(PlanningDraftStageRecord {
                draft_name: draft_name.to_string(),
                draft_directory: "/tmp/drafts".to_string(),
                staged_files: vec![PlanningStagedFileRecord {
                    active_path: "task-ledger.json".to_string(),
                    staged_path: ".codex-exec-loop/planning/drafts/task-ledger.json".to_string(),
                }],
            })
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            Err(anyhow!("unused in test"))
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!("unused in test"))
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            match (self.load_record.as_ref(), self.load_error_message.as_ref()) {
                (_, Some(message)) => Err(anyhow!("{message}")),
                (Some(record), None) => Ok(record.clone()),
                (None, None) => Ok(PlanningWorkspaceLoadRecord::default()),
            }
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
            _body: Option<&str>,
        ) -> Result<()> {
            Err(anyhow!("unused in test"))
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!("unused in test"))
        }
    }

    fn runtime_facade_with_load_result(
        load_result: Result<PlanningWorkspaceLoadRecord>,
    ) -> PlanningRuntimeFacadeService {
        let (load_record, load_error_message) = match load_result {
            Ok(record) => (Some(record), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let port = Arc::new(FakePlanningWorkspacePort {
            load_record,
            load_error_message,
        });
        PlanningRuntimeFacadeService::new(
            PlanningPromptService::new(
                port.clone(),
                PlanningValidationService::new(),
                PriorityQueueService::new(),
            ),
            PlanningReconciliationService::new(
                port,
                PlanningValidationService::new(),
                PriorityQueueService::new(),
            ),
            PlanningRuntimePolicyService::new(),
            TurnPromptAssemblyService::new(),
        )
    }

    fn builtin_next_task_template() -> FollowupTemplateDefinition {
        FollowupTemplateDefinition {
            id: "builtin-next-task".to_string(),
            label: "builtin next-task".to_string(),
            body: "session={session_id}\nauto={auto_turn}/{max_auto_turns}\nlast={last_message}\nstop={stop_keyword}".to_string(),
            source: FollowupTemplateSource::Builtin,
        }
    }

    fn ready_snapshot()
    -> crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot {
        crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "next task: rank 1 / task-1".to_string(),
            Some(PriorityQueueTask {
                rank: 1,
                task_id: "task-1".to_string(),
                direction_id: "general-workstream".to_string(),
                direction_title: "General workstream".to_string(),
                task_title: "Implement planning runtime facade".to_string(),
                status: crate::domain::planning::TaskStatus::Ready,
                combined_priority: 10,
                updated_at: "2026-04-10T00:00:00Z".to_string(),
                rank_reasons: vec!["status=ready".to_string()],
            }),
        )
    }

    #[test]
    fn load_runtime_snapshot_or_invalid_converts_port_failure_into_invalid_snapshot() {
        let service = runtime_facade_with_load_result(Err(anyhow!("permission denied")));

        let snapshot = service.load_runtime_snapshot_or_invalid("/tmp/workspace");

        assert_eq!(snapshot.workspace_status(), crate::application::service::planning_prompt_service::PlanningRuntimeWorkspaceStatus::Invalid);
        assert_eq!(
            snapshot.failure_reason(),
            Some("failed to load planning workspace: permission denied")
        );
    }

    #[test]
    fn decide_auto_followup_queues_prompt_when_template_and_snapshot_are_ready() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));

        let decision = service.decide_auto_followup(PlanningRuntimeAutoFollowRequest {
            template: &builtin_next_task_template(),
            auto_turn: 1,
            max_auto_turns: 3,
            session_id: "thread-1",
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            snapshot: &ready_snapshot(),
        });

        let PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) = decision else {
            panic!("expected queued prompt");
        };
        assert!(prompt.contains("session=thread-1"));
        assert!(prompt.contains("last=latest answer"));
        assert!(prompt.contains("Planning Context"));
    }

    #[test]
    fn decide_auto_followup_blocks_builtin_next_task_when_queue_head_is_missing() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready(
                "Planning Context".to_string(),
                "next_task: none".to_string(),
                None,
            );

        assert_eq!(
            service.decide_auto_followup(PlanningRuntimeAutoFollowRequest {
                template: &builtin_next_task_template(),
                auto_turn: 1,
                max_auto_turns: 3,
                session_id: "thread-1",
                stop_keyword: "AUTO_STOP",
                last_message: "latest answer",
                snapshot: &snapshot,
            }),
            PlanningRuntimeAutoFollowDecision::Blocked(
                PlanningAutoFollowBlockReason::ActionableQueueRequired
            )
        );
    }

    #[test]
    fn build_auto_follow_preview_returns_rendered_prompt_and_planning_view() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));

        let preview = service.build_auto_follow_preview(PlanningRuntimePreviewRequest {
            template: &builtin_next_task_template(),
            auto_turn: 1,
            max_auto_turns: 3,
            session_id: "",
            stop_keyword: "AUTO_STOP",
            last_message: None,
            snapshot: &ready_snapshot(),
        });

        assert!(preview.rendered_prompt.contains("session=draft-thread"));
        assert_eq!(preview.planning_view.status_label, "ready");
        assert_eq!(
            preview.planning_view.detail.as_deref(),
            Some("next task: rank 1 / task-1")
        );
    }
}
