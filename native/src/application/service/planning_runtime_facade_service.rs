use anyhow::Result;

use crate::application::service::planning_prompt_service::{
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning_reconciliation_service::{
    PlanningExecutionSnapshot, PlanningReconciliationResult, PlanningReconciliationService,
};
use crate::application::service::planning_runtime_policy_service::{
    PlanningAutoFollowBlockReason, PlanningRuntimePolicyService, PlanningRuntimeSummaryRequest,
    PlanningRuntimeSummaryView,
};
use crate::application::service::turn_prompt_assembly_service::{
    AutoFollowPromptAssemblyRequest, AutoFollowPromptPreviewRequest, ManualPromptAssemblyRequest,
    PlanningAutoFollowOperation, PlanningAutoFollowPromptAssemblyRequest,
    PlanningAutoFollowPromptPreviewRequest, TurnPromptAssemblyService,
};
use crate::domain::followup_template::FollowupTemplateDefinition;
use crate::domain::planning::PlanningWorkspaceState;
use crate::domain::text::compact_whitespace_detail;

const BUILTIN_NEXT_TASK_TEMPLATE_ID: &str = "builtin-next-task";

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
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Blocked(PlanningAutoFollowBlockReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeQueuedAutoFollowPrompt {
    pub prompt: String,
    pub transcript_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeRenderedPreview {
    pub rendered_prompt: String,
    pub planning_status_line: String,
    pub planning_detail_line: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRuntimeRepairAttempt {
    pub attempts_used: usize,
    pub max_attempts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRuntimeSummaryLineRequest<'a> {
    pub snapshot: &'a PlanningRuntimeSnapshot,
    pub has_running_turn: bool,
    pub is_repairing: bool,
    pub repair_failure_summary: Option<&'a str>,
    pub repair_attempt: Option<PlanningRuntimeRepairAttempt>,
    pub has_notice: bool,
    pub max_detail_len: usize,
    pub always_show: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRuntimeStatusProjectionRequest<'a> {
    pub snapshot: &'a PlanningRuntimeSnapshot,
    pub has_running_turn: bool,
    pub is_repairing: bool,
    pub repair_failure_summary: Option<&'a str>,
    pub repair_attempt: Option<PlanningRuntimeRepairAttempt>,
    pub max_detail_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeStatusProjection {
    pub planning_status_line: String,
    pub repair_attempt_line: Option<String>,
    pub queue_head_line: Option<String>,
    pub proposal_line: Option<String>,
    pub failure_line: Option<String>,
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
        if request.template.id == BUILTIN_NEXT_TASK_TEMPLATE_ID
            && request.snapshot.workspace_status()
                == crate::application::service::planning_prompt_service::PlanningRuntimeWorkspaceStatus::ReadyNoTask
        {
            return PlanningRuntimeAutoFollowDecision::QueuePrompt(
                self.build_planning_queue_refresh_prompt(&request),
            );
        }

        if let Some(block_reason) = self
            .planning_runtime_policy_service
            .auto_follow_block_reason(request.template, request.snapshot)
        {
            return PlanningRuntimeAutoFollowDecision::Blocked(block_reason);
        }

        PlanningRuntimeAutoFollowDecision::QueuePrompt(
            self.build_template_auto_follow_prompt(&request),
        )
    }

    pub fn build_auto_follow_preview(
        &self,
        request: PlanningRuntimePreviewRequest<'_>,
    ) -> PlanningRuntimeRenderedPreview {
        let planning_view = self
            .planning_runtime_policy_service
            .build_preview_view(request.template, request.snapshot);
        let rendered_prompt = if request.template.id == BUILTIN_NEXT_TASK_TEMPLATE_ID
            && request.snapshot.workspace_status()
                == crate::application::service::planning_prompt_service::PlanningRuntimeWorkspaceStatus::ReadyNoTask
        {
            self.turn_prompt_assembly_service
                .build_planning_auto_follow_prompt_preview(
                    PlanningAutoFollowPromptPreviewRequest {
                        operation: PlanningAutoFollowOperation::RefreshQueueFromLatestAnswer,
                        auto_turn: request.auto_turn,
                        max_auto_turns: request.max_auto_turns,
                        session_id: request.session_id,
                        stop_keyword: request.stop_keyword,
                        last_message: request.last_message,
                        planning_prompt_fragment: request.snapshot.prompt_fragment(),
                    },
                )
        } else {
            self.turn_prompt_assembly_service
                .build_auto_follow_prompt_preview(AutoFollowPromptPreviewRequest {
                    template: request.template,
                    auto_turn: request.auto_turn,
                    max_auto_turns: request.max_auto_turns,
                    session_id: request.session_id,
                    stop_keyword: request.stop_keyword,
                    last_message: request.last_message,
                    planning_prompt_fragment: request.snapshot.prompt_fragment(),
                })
        };
        PlanningRuntimeRenderedPreview {
            rendered_prompt,
            planning_status_line: format!("planning: {}", planning_view.status_label),
            planning_detail_line: planning_view
                .detail
                .map(|detail| format!("planning detail: {detail}")),
        }
    }

    pub fn build_summary_view(
        &self,
        request: PlanningRuntimeSummaryRequest<'_>,
    ) -> PlanningRuntimeSummaryView {
        self.planning_runtime_policy_service
            .build_summary_view(request)
    }

    pub fn build_summary_line(
        &self,
        request: PlanningRuntimeSummaryLineRequest<'_>,
    ) -> Option<String> {
        let summary = self.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: request.snapshot,
            has_running_turn: request.has_running_turn,
            is_repairing: request.is_repairing,
            repair_failure_summary: request.repair_failure_summary,
        });
        let planning_state = summary.workspace_state;
        if !request.always_show
            && planning_state == PlanningWorkspaceState::Uninitialized
            && !request.has_notice
        {
            return None;
        }

        let mut segments = vec![format!("planning: {}", summary.status_label)];
        if let Some(repair_attempt) = request.repair_attempt {
            segments.push(format!(
                "repair: {}/{}",
                repair_attempt.attempts_used, repair_attempt.max_attempts
            ));
        }

        match planning_state {
            PlanningWorkspaceState::Ready | PlanningWorkspaceState::Executing => {
                if let Some(queue_summary) = summary.queue_summary.as_deref() {
                    segments.push(format!(
                        "queue: {}",
                        compact_projection_detail(queue_summary, request.max_detail_len)
                    ));
                }
                if let Some(proposal_summary) = summary.proposal_summary.as_deref() {
                    segments.push(format!(
                        "proposals: {}",
                        compact_projection_detail(proposal_summary, request.max_detail_len)
                    ));
                }
            }
            PlanningWorkspaceState::Repairing => {
                if let Some(failure_summary) = summary.failure_summary.as_deref() {
                    segments.push(format!(
                        "failure: {}",
                        compact_projection_detail(failure_summary, request.max_detail_len)
                    ));
                }
                if let Some(queue_summary) = summary.queue_summary.as_deref() {
                    segments.push(format!(
                        "queue: {}",
                        compact_projection_detail(queue_summary, request.max_detail_len)
                    ));
                }
                if let Some(proposal_summary) = summary.proposal_summary.as_deref() {
                    segments.push(format!(
                        "proposals: {}",
                        compact_projection_detail(proposal_summary, request.max_detail_len)
                    ));
                }
            }
            PlanningWorkspaceState::BlockedInvalid => {
                if let Some(failure_summary) = summary.failure_summary.as_deref() {
                    segments.push(format!(
                        "failure: {}",
                        compact_projection_detail(failure_summary, request.max_detail_len)
                    ));
                }
            }
            PlanningWorkspaceState::Uninitialized | PlanningWorkspaceState::Authoring => {}
        }

        Some(segments.join("  |  "))
    }

    pub fn build_followup_status_projection(
        &self,
        request: PlanningRuntimeStatusProjectionRequest<'_>,
    ) -> PlanningRuntimeStatusProjection {
        let summary = self.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: request.snapshot,
            has_running_turn: request.has_running_turn,
            is_repairing: request.is_repairing,
            repair_failure_summary: request.repair_failure_summary,
        });

        PlanningRuntimeStatusProjection {
            planning_status_line: format!("planning status: {}", summary.status_label),
            repair_attempt_line: request.repair_attempt.map(|repair_attempt| {
                format!(
                    "planning repair attempt: {}/{}",
                    repair_attempt.attempts_used, repair_attempt.max_attempts
                )
            }),
            queue_head_line: summary.queue_summary.as_deref().map(|queue_summary| {
                let queue_label = if request.snapshot.queue_head().is_some() {
                    "planning queue head"
                } else {
                    "planning queue"
                };
                format!(
                    "{queue_label}: {}",
                    compact_projection_detail(queue_summary, request.max_detail_len)
                )
            }),
            proposal_line: summary.proposal_summary.as_deref().map(|proposal_summary| {
                format!(
                    "planning proposals: {}",
                    compact_projection_detail(proposal_summary, request.max_detail_len)
                )
            }),
            failure_line: summary.failure_summary.as_deref().map(|failure_summary| {
                format!(
                    "last planning failure: {}",
                    compact_projection_detail(failure_summary, request.max_detail_len)
                )
            }),
        }
    }

    fn build_template_auto_follow_prompt(
        &self,
        request: &PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeQueuedAutoFollowPrompt {
        PlanningRuntimeQueuedAutoFollowPrompt {
            prompt: self.turn_prompt_assembly_service.build_auto_follow_prompt(
                AutoFollowPromptAssemblyRequest {
                    template: request.template,
                    auto_turn: request.auto_turn,
                    max_auto_turns: request.max_auto_turns,
                    session_id: request.session_id,
                    stop_keyword: request.stop_keyword,
                    last_message: request.last_message.trim(),
                    planning_prompt_fragment: request.snapshot.prompt_fragment(),
                },
            ),
            transcript_text: default_auto_follow_transcript_text(request.template),
        }
    }

    fn build_planning_queue_refresh_prompt(
        &self,
        request: &PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeQueuedAutoFollowPrompt {
        PlanningRuntimeQueuedAutoFollowPrompt {
            prompt: self
                .turn_prompt_assembly_service
                .build_planning_auto_follow_prompt(PlanningAutoFollowPromptAssemblyRequest {
                    operation: PlanningAutoFollowOperation::RefreshQueueFromLatestAnswer,
                    auto_turn: request.auto_turn,
                    max_auto_turns: request.max_auto_turns,
                    session_id: request.session_id,
                    stop_keyword: request.stop_keyword,
                    last_message: request.last_message.trim(),
                    planning_prompt_fragment: request.snapshot.prompt_fragment(),
                }),
            transcript_text: planning_queue_refresh_transcript_text(request.snapshot),
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

fn compact_projection_detail(text: &str, max_len: usize) -> String {
    compact_whitespace_detail(text, max_len)
}

fn default_auto_follow_transcript_text(template: &FollowupTemplateDefinition) -> String {
    if template.id == BUILTIN_NEXT_TASK_TEMPLATE_ID {
        "priority queue의 현재 next task 1개를 이어서 진행합니다.".to_string()
    } else {
        format!(
            "selected auto follow-up template `{}` 기준으로 다음 작업 1개를 진행합니다.",
            template.label
        )
    }
}

fn planning_queue_refresh_transcript_text(snapshot: &PlanningRuntimeSnapshot) -> String {
    if snapshot.has_proposal_candidates() {
        "previous answer와 existing proposal을 정리해 priority queue를 갱신하고, 가장 높은 우선순위 1개만 수행한 뒤 남은 proposal을 정리합니다.".to_string()
    } else {
        "previous answer에서 실행 가능한 후속 작업을 추출해 priority queue에 반영하고, 가장 높은 우선순위 1개만 수행한 뒤 남은 proposal을 정리합니다.".to_string()
    }
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
    use crate::application::service::planning_prompt_service::PlanningPromptService;
    use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
    use crate::application::service::planning_runtime_policy_service::PlanningRuntimePolicyService;
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
        assert!(prompt.prompt.contains("session=thread-1"));
        assert!(prompt.prompt.contains("last=latest answer"));
        assert!(prompt.prompt.contains("Planning Context"));
        assert_eq!(
            prompt.transcript_text,
            "priority queue의 현재 next task 1개를 이어서 진행합니다."
        );
    }

    #[test]
    fn decide_auto_followup_queues_planning_refresh_when_queue_head_is_missing() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready(
                "Planning Context".to_string(),
                "next_task: none".to_string(),
                None,
            );

        let decision = service.decide_auto_followup(PlanningRuntimeAutoFollowRequest {
            template: &builtin_next_task_template(),
            auto_turn: 1,
            max_auto_turns: 3,
            session_id: "thread-1",
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            snapshot: &snapshot,
        });

        let PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) = decision else {
            panic!("expected planning refresh prompt");
        };
        assert!(prompt.prompt.contains("planning priority queue"));
        assert!(prompt.prompt.contains("latest answer"));
        assert!(prompt.prompt.contains("Planning Context"));
        assert!(
            prompt.transcript_text.contains(
                "previous answer에서 실행 가능한 후속 작업을 추출해 priority queue에 반영"
            )
        );
    }

    #[test]
    fn decide_auto_followup_queues_prompt_when_proposals_exist_without_queue_head() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready_with_details(
                "Planning Context\nRuntime Follow-up Proposal Rules".to_string(),
                "queue idle: no executable planning task".to_string(),
                Some(
                    "2 proposed follow-up tasks waiting for promotion: Draft roadmap | +1 more"
                        .to_string(),
                ),
                None,
            );

        let decision = service.decide_auto_followup(PlanningRuntimeAutoFollowRequest {
            template: &builtin_next_task_template(),
            auto_turn: 1,
            max_auto_turns: 3,
            session_id: "thread-1",
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            snapshot: &snapshot,
        });

        let PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) = decision else {
            panic!("expected queued prompt when proposals can be promoted");
        };
        assert!(prompt.prompt.contains("planning priority queue"));
        assert!(prompt.prompt.contains("latest answer"));
        assert!(prompt.prompt.contains("Planning Context"));
        assert!(prompt.prompt.contains("Runtime Follow-up Proposal Rules"));
        assert!(
            prompt
                .transcript_text
                .contains("existing proposal을 정리해 priority queue를 갱신")
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
        assert_eq!(preview.planning_status_line, "planning: ready");
        assert_eq!(
            preview.planning_detail_line.as_deref(),
            Some("planning detail: next task: rank 1 / task-1")
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
            );

        let preview = service.build_auto_follow_preview(PlanningRuntimePreviewRequest {
            template: &builtin_next_task_template(),
            auto_turn: 1,
            max_auto_turns: 3,
            session_id: "",
            stop_keyword: "AUTO_STOP",
            last_message: None,
            snapshot: &snapshot,
        });

        assert!(preview.rendered_prompt.contains("planning priority queue"));
        assert!(
            preview
                .rendered_prompt
                .contains("(waiting for next agent reply)")
        );
        assert_eq!(preview.planning_status_line, "planning: ready");
        assert_eq!(
            preview.planning_detail_line.as_deref(),
            Some("planning detail: queue idle: no executable planning task")
        );
    }

    #[test]
    fn build_summary_line_compacts_queue_and_failure_segments() {
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
        assert!(summary_line.contains("planning: repairing"));
        assert!(summary_line.contains("repair: 1/2"));
        assert!(summary_line.contains("failure: task-ledger.json is m..."));
        assert!(summary_line.contains("queue: next task: rank 1 / t..."));
    }

    #[test]
    fn build_summary_line_includes_proposals_when_queue_is_idle() {
        let service = runtime_facade_with_load_result(Ok(PlanningWorkspaceLoadRecord::default()));
        let snapshot =
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::ready_with_details(
                "Planning Context".to_string(),
                "queue idle: no executable planning task".to_string(),
                Some(
                    "2 proposed follow-up tasks waiting for promotion: Draft roadmap | Draft checklist"
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
        assert!(summary_line.contains("queue: queue idle:"));
        assert!(summary_line.contains("proposals: 2 proposed"));
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
                    "1 proposed follow-up task waiting for promotion: Draft sushi roadmap"
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

        assert!(
            projection.proposal_line.as_deref().is_some_and(
                |line| line.starts_with("planning proposals: 1 proposed follow-up task")
            )
        );
    }
}
