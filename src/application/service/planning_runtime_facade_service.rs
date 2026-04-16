use crate::application::service::planning_auto_follow_copy::BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT;
use crate::application::service::planning_prompt_service::{
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning_reconciliation_service::{
    PlanningExecutionSnapshot, PlanningReconciliationResult, PlanningReconciliationService,
};
use crate::application::service::planning_runtime_policy_service::{
    PlanningAutoFollowBlockReason, PlanningAutoFollowPolicyDecision, PlanningAutoFollowPromptMode,
    PlanningRuntimePolicyService,
};
use crate::application::service::turn_prompt_assembly_service::{
    ManualPromptAssemblyRequest, PlanningAutoFollowOperation,
    PlanningAutoFollowPromptAssemblyRequest, PlanningAutoFollowPromptPreviewRequest,
    TurnPromptAssemblyService,
};
use crate::domain::planning::PriorityQueueTask;
use anyhow::Result;

pub use crate::application::service::planning_runtime_policy_service::{
    PlanningRuntimeRepairAttempt, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeAutoFollowRequest<'a> {
    pub stop_keyword: &'a str,
    pub last_message: &'a str,
    pub snapshot: &'a PlanningRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimePreviewRequest<'a> {
    pub stop_keyword: &'a str,
    pub last_message: Option<&'a str>,
    pub snapshot: &'a PlanningRuntimeSnapshot,
    pub has_running_turn: bool,
    pub is_repairing: bool,
    pub repair_failure_summary: Option<&'a str>,
    pub max_detail_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningRuntimeAutoFollowDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Blocked(PlanningAutoFollowBlockReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeQueuedAutoFollowPrompt {
    pub prompt: String,
    pub transcript_text: String,
    pub handoff_task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeRenderedPreview {
    pub rendered_prompt: String,
    pub current_state_line: String,
    pub cause_line: String,
    pub next_action_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningMainSessionHandoff {
    pub prompt: String,
    pub transcript_text: String,
    pub task: PlanningTaskHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskHandoff {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub status_label: String,
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
        _snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<String> {
        self.turn_prompt_assembly_service
            .build_manual_prompt(ManualPromptAssemblyRequest {
                operator_prompt,
                planning_prompt_fragment: None,
            })
    }

    pub fn build_builtin_next_task_handoff(
        &self,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<PlanningMainSessionHandoff> {
        let queue_head = snapshot.queue_head()?;
        Some(PlanningMainSessionHandoff {
            prompt: render_builtin_next_task_handoff_prompt(queue_head),
            transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
            task: PlanningTaskHandoff {
                task_id: queue_head.task_id.trim().to_string(),
                task_title: queue_head.task_title.trim().to_string(),
                direction_id: queue_head.direction_id.trim().to_string(),
                combined_priority: queue_head.combined_priority,
                updated_at: queue_head.updated_at.trim().to_string(),
                status_label: queue_head.status.label().to_string(),
            },
        })
    }

    pub fn builtin_next_task_preview_prompt(&self, snapshot: &PlanningRuntimeSnapshot) -> String {
        self.build_builtin_next_task_handoff(snapshot)
            .map(|handoff| handoff.prompt)
            .unwrap_or_else(|| {
                match snapshot.queue_idle_policy() {
                    crate::domain::planning::QueueIdlePolicy::Stop => {
                        "The current planning queue has no actionable head and queue-idle policy is stop, so post-turn automation will end after the current turn.".to_string()
                    }
                    crate::domain::planning::QueueIdlePolicy::ReviewAndEnqueue => {
                        "A queue-manager planning worker reviews the direction goals after the current turn and re-enqueues follow-up work only when a justified actionable task exists.".to_string()
                    }
                }
            })
    }

    pub fn decide_auto_followup(
        &self,
        request: PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeAutoFollowDecision {
        match self
            .planning_runtime_policy_service
            .decide_auto_follow(request.snapshot)
        {
            PlanningAutoFollowPolicyDecision::Blocked(block_reason) => {
                PlanningRuntimeAutoFollowDecision::Blocked(block_reason)
            }
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::ContinueQueuedTask,
            ) => match self.build_builtin_next_task_handoff(request.snapshot) {
                Some(handoff) => PlanningRuntimeAutoFollowDecision::QueuePrompt(
                    PlanningRuntimeQueuedAutoFollowPrompt {
                        prompt: handoff.prompt,
                        transcript_text: handoff.transcript_text,
                        handoff_task: Some(handoff.task),
                    },
                ),
                None => PlanningRuntimeAutoFollowDecision::Blocked(
                    PlanningAutoFollowBlockReason::ActionableQueueRequired,
                ),
            },
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::RefreshPlanningQueue,
            ) => PlanningRuntimeAutoFollowDecision::QueuePrompt(
                self.build_refresh_queue_prompt(&request),
            ),
        }
    }

    pub fn build_auto_follow_preview(
        &self,
        request: PlanningRuntimePreviewRequest<'_>,
    ) -> PlanningRuntimeRenderedPreview {
        let policy_decision = self
            .planning_runtime_policy_service
            .decide_auto_follow(request.snapshot);
        let rendered_prompt = match policy_decision {
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::RefreshPlanningQueue,
            ) => self
                .turn_prompt_assembly_service
                .build_planning_auto_follow_prompt_preview(
                    PlanningAutoFollowPromptPreviewRequest {
                        operation: PlanningAutoFollowOperation::RefreshQueueFromLatestAnswer,
                        stop_keyword: request.stop_keyword,
                        last_message: request.last_message,
                        planning_prompt_fragment: request.snapshot.prompt_fragment(),
                    },
                ),
            PlanningAutoFollowPolicyDecision::Blocked(_)
            | PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::ContinueQueuedTask,
            ) => self.builtin_next_task_preview_prompt(request.snapshot),
        };
        let planning_projection = self.build_followup_status_projection(
            PlanningRuntimeStatusProjectionRequest {
                snapshot: request.snapshot,
                has_running_turn: request.has_running_turn,
                is_repairing: request.is_repairing,
                repair_failure_summary: request.repair_failure_summary,
                repair_attempt: None,
                max_detail_len: request.max_detail_len,
            },
        );
        PlanningRuntimeRenderedPreview {
            rendered_prompt,
            current_state_line: planning_projection.current_state_line,
            cause_line: planning_projection.cause_line,
            next_action_line: planning_projection.next_action_line,
        }
    }

    pub fn build_summary_line(
        &self,
        request: PlanningRuntimeSummaryLineRequest<'_>,
    ) -> Option<String> {
        self.planning_runtime_policy_service
            .build_summary_line(request)
    }

    pub fn build_followup_status_projection(
        &self,
        request: PlanningRuntimeStatusProjectionRequest<'_>,
    ) -> PlanningRuntimeStatusProjection {
        self.planning_runtime_policy_service
            .build_status_projection(request)
    }

    fn build_refresh_queue_prompt(
        &self,
        request: &PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeQueuedAutoFollowPrompt {
        PlanningRuntimeQueuedAutoFollowPrompt {
            prompt: self
                .turn_prompt_assembly_service
                .build_planning_auto_follow_prompt(PlanningAutoFollowPromptAssemblyRequest {
                    operation: PlanningAutoFollowOperation::RefreshQueueFromLatestAnswer,
                    stop_keyword: request.stop_keyword,
                    last_message: request.last_message.trim(),
                    planning_prompt_fragment: request.snapshot.prompt_fragment(),
                }),
            transcript_text: self
                .planning_runtime_policy_service
                .auto_follow_transcript_text(
                    request.snapshot,
                    PlanningAutoFollowPromptMode::RefreshPlanningQueue,
                ),
            handoff_task: None,
        }
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

fn render_builtin_next_task_handoff_prompt(queue_head: &PriorityQueueTask) -> String {
    let rank_reason = queue_head
        .rank_reasons
        .iter()
        .find(|reason| !reason.trim().is_empty())
        .map(String::as_str)
        .unwrap_or("this is the highest-priority actionable task");
    format!(
        "Continue the next highest-priority task.\n\nTask: {}\nDirection: {}\nPriority: rank {} / combined priority {}\nWhy now: {}\n\nWork from the current repository state and focus on this task only. Treat `.codex-exec-loop/planning` and other planning control files as internal runtime state. Do not inspect, mention, or update them unless the user explicitly asked for planning maintenance or this task strictly requires it. Do not describe planning queue refresh logic in commentary or in the final answer. When you finish, summarize what you completed and what remains.",
        queue_head.task_title.trim(),
        queue_head.direction_title.trim(),
        queue_head.rank,
        queue_head.combined_priority,
        rank_reason.trim(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, anyhow};

    use super::{
        PlanningRuntimeAutoFollowDecision, PlanningRuntimeAutoFollowRequest,
        PlanningRuntimeFacadeService, PlanningRuntimePreviewRequest, PlanningRuntimeRepairAttempt,
        PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    };
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningStagedFileRecord, PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning_contract::{
        DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH,
        RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
    };
    use crate::application::service::planning_prompt_service::PlanningPromptService;
    use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
    use crate::application::service::planning_runtime_policy_service::PlanningRuntimePolicyService;
    use crate::application::service::planning_validation_service::PlanningValidationService;
    use crate::application::service::priority_queue_service::PriorityQueueService;
    use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
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

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            let Some(record) = self.load_record.as_ref() else {
                return Ok(None);
            };
            let body = match relative_path {
                DIRECTIONS_FILE_PATH => record.directions_toml.clone(),
                TASK_LEDGER_FILE_PATH => record.task_ledger_json.clone(),
                TASK_LEDGER_SCHEMA_FILE_PATH => record.task_ledger_schema_json.clone(),
                QUEUE_SNAPSHOT_FILE_PATH => record.queue_snapshot_json.clone(),
                RESULT_OUTPUT_FILE_PATH => record.result_output_markdown.clone(),
                _ => None,
            };
            Ok(body)
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
    fn decide_auto_followup_queues_prompt_when_snapshot_is_ready() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));

        let decision = service.decide_auto_followup(PlanningRuntimeAutoFollowRequest {
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            snapshot: &ready_snapshot(),
        });

        let PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) = decision else {
            panic!("expected queued prompt");
        };
        assert!(
            prompt
                .prompt
                .contains("Continue the next highest-priority task.")
        );
        assert!(prompt.prompt.contains("Implement planning runtime facade"));
        assert!(prompt.prompt.contains("General workstream"));
        assert_eq!(
            prompt.transcript_text,
            "다음 queued task 1개를 이어서 진행합니다."
        );
        assert_eq!(
            prompt
                .handoff_task
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("task-1")
        );
    }

    #[test]
    fn decide_auto_followup_blocks_when_queue_head_is_missing() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready(
                "Planning Context".to_string(),
                "next_task: none".to_string(),
                None,
            );

        let decision = service.decide_auto_followup(PlanningRuntimeAutoFollowRequest {
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            snapshot: &snapshot,
        });

        let PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) = decision else {
            panic!("expected refresh prompt");
        };
        assert!(
            prompt
                .prompt
                .contains("planning priority queue를 갱신하세요.")
        );
        assert!(prompt.handoff_task.is_none());
    }

    #[test]
    fn decide_auto_followup_refreshes_when_only_proposals_exist_without_queue_head() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready_with_details(
                "Planning Context\nRuntime Follow-up Proposal Rules".to_string(),
                "queue idle: no executable planning task".to_string(),
                Some(
                    "2 promotable follow-up proposals available: Draft roadmap | +1 more"
                        .to_string(),
                ),
                None,
            );

        let decision = service.decide_auto_followup(PlanningRuntimeAutoFollowRequest {
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            snapshot: &snapshot,
        });

        let PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) = decision else {
            panic!("expected refresh prompt");
        };
        assert!(
            prompt
                .prompt
                .contains("planning priority queue를 갱신하세요.")
        );
        assert!(prompt.prompt.contains("latest answer"));
        assert!(prompt.handoff_task.is_none());
    }

    #[test]
    fn build_auto_follow_preview_returns_rendered_prompt_and_planning_view() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));

        let preview = service.build_auto_follow_preview(PlanningRuntimePreviewRequest {
            stop_keyword: "AUTO_STOP",
            last_message: None,
            snapshot: &ready_snapshot(),
            has_running_turn: false,
            is_repairing: false,
            repair_failure_summary: None,
            max_detail_len: 48,
        });

        assert!(
            preview
                .rendered_prompt
                .contains("Continue the next highest-priority task.")
        );
        assert!(
            preview
                .rendered_prompt
                .contains("Implement planning runtime facade")
        );
        assert_eq!(preview.current_state_line, "current state: ready");
        assert!(
            preview
                .cause_line
                .starts_with("cause: the next queued task is Implement planning")
        );
        assert!(
            preview
                .next_action_line
                .starts_with("next action: let automation continue, or run the queued")
        );
    }

    #[test]
    fn build_auto_follow_preview_uses_planning_refresh_prompt_when_queue_is_idle() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready(
                "Planning Context".to_string(),
                "queue idle: no executable planning task".to_string(),
                None,
            )
            .with_queue_idle_policy(
                crate::domain::planning::QueueIdlePolicy::ReviewAndEnqueue,
                Some(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()),
            );

        let preview = service.build_auto_follow_preview(PlanningRuntimePreviewRequest {
            stop_keyword: "AUTO_STOP",
            last_message: None,
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: false,
            repair_failure_summary: None,
            max_detail_len: 48,
        });

        assert!(
            preview
                .rendered_prompt
                .contains("planning priority queue를 갱신하세요.")
        );
        assert_eq!(preview.current_state_line, "current state: waiting");
        assert_eq!(preview.cause_line, "cause: planning is valid but has no next task yet");
        assert!(
            preview
                .next_action_line
                .starts_with("next action: finish the next turn or review the queue")
        );
    }

    #[test]
    fn build_summary_line_uses_current_state_cause_and_next_action() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot = ready_snapshot();

        let summary_line = service.build_summary_line(PlanningRuntimeSummaryLineRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: true,
            repair_failure_summary: Some(
                "task-ledger.json is missing direction_id and contains extra trailing data",
            ),
            repair_attempt: Some(PlanningRuntimeRepairAttempt {
                attempts_used: 1,
                max_attempts: 2,
            }),
            has_notice: true,
            max_detail_len: 24,
            always_show: true,
        });

        let summary_line = summary_line.expect("summary line should be projected");
        assert!(summary_line.contains("current state: repairing"));
        assert!(summary_line.contains("cause: planning needs rep"));
        assert!(summary_line.contains("next action: wait for repair"));
        assert!(summary_line.contains("repair attempt: 1/2"));
    }

    #[test]
    fn build_summary_line_projects_review_needed_when_queue_has_only_proposals() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready_with_details(
                "Planning Context".to_string(),
                "queue idle: no executable planning task".to_string(),
                Some(
                    "2 promotable follow-up proposals available: Draft roadmap | Draft checklist"
                        .to_string(),
                ),
                None,
            );

        let summary_line = service.build_summary_line(PlanningRuntimeSummaryLineRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: false,
            repair_failure_summary: None,
            repair_attempt: None,
            has_notice: false,
            max_detail_len: 36,
            always_show: true,
        });

        let summary_line = summary_line.expect("summary line should be projected");
        assert!(summary_line.contains("current state: review needed"));
        assert!(summary_line.contains("cause: planning has proposals but no"));
        assert!(summary_line.contains("next action: review the queue and promote"));
    }

    #[test]
    fn build_followup_status_projection_formats_planning_lines() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let projection =
            service.build_followup_status_projection(PlanningRuntimeStatusProjectionRequest {
                snapshot: &ready_snapshot(),
                has_running_turn: false,
                is_repairing: true,
                repair_failure_summary: Some("task-ledger.json is missing direction_id"),
                repair_attempt: Some(PlanningRuntimeRepairAttempt {
                    attempts_used: 1,
                    max_attempts: 2,
                }),
                max_detail_len: 30,
            });

        assert_eq!(
            projection.planning_status_line,
            "planning status: repairing"
        );
        assert_eq!(projection.current_state_line, "current state: repairing");
        assert!(projection.cause_line.contains("planning needs repair"));
        assert!(
            projection
                .next_action_line
                .starts_with("next action: wait for repair to finish")
        );
        assert_eq!(
            projection.repair_attempt_line.as_deref(),
            Some("planning repair attempt: 1/2")
        );
        assert_eq!(
            projection.queue_head_line.as_deref(),
            Some("planning queue head: next task: rank 1 / task-1")
        );
        assert!(
            projection
                .failure_line
                .as_deref()
                .expect("failure line should exist")
                .starts_with("last planning failure: task-ledger.json is mi")
        );
    }

    #[test]
    fn build_followup_status_projection_surfaces_proposal_line() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready_with_details(
                "Planning Context".to_string(),
                "queue idle: no executable planning task".to_string(),
                Some(
                    "1 promotable follow-up proposal available: Draft sushi roadmap"
                        .to_string(),
                ),
                None,
            );

        let projection =
            service.build_followup_status_projection(PlanningRuntimeStatusProjectionRequest {
                snapshot: &snapshot,
                has_running_turn: false,
                is_repairing: false,
                repair_failure_summary: None,
                repair_attempt: None,
                max_detail_len: 48,
            });

        assert!(projection.proposal_line.as_deref().is_some_and(|line| {
            line.starts_with("planning proposals: 1 promotable follow-up proposal")
        }));
    }
}
