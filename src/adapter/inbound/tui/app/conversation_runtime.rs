use super::conversation_model::{AutoFollowupSkipReason, PlanningRepairState};
use super::*;
use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;

#[derive(Debug, Clone)]
pub(super) enum ConversationRuntimeEvent {
    SubmitPrompt {
        prompt: String,
        origin: PromptOrigin,
    },
    StreamUpdated(ConversationStreamEvent),
    StreamExecutionObserved {
        notice: String,
    },
    PostTurnEvaluated {
        evaluation: Box<ConversationPostTurnEvaluation>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConversationRuntimeEffect {
    StartStream {
        workspace_directory: String,
        thread_id: Option<String>,
        prompt: String,
    },
    EvaluateAutoFollowup {
        workspace_directory: String,
        queued_from_turn_id: String,
        changed_planning_file_paths: Vec<String>,
    },
    QueueAutoPrompt {
        prompt: String,
        queued_from_turn_id: String,
        template_label: String,
        transcript_text: String,
        handoff_task: Option<PlanningTaskHandoff>,
    },
    QueuePlanningRepairPrompt {
        prompt: String,
        queued_from_turn_id: String,
        attempt_number: usize,
        max_attempts: usize,
    },
}

#[derive(Debug, Clone)]
pub(super) struct ConversationPostTurnEvaluation {
    pub planning_runtime_snapshot: PlanningRuntimeSnapshot,
    pub planning_repair_state: Option<PlanningRepairState>,
    pub runtime_notices: Vec<String>,
    pub action: ConversationPostTurnAction,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(super) enum ConversationPostTurnAction {
    QueuePlanningRepair {
        prompt: String,
        queued_from_turn_id: String,
        attempt_number: usize,
        max_attempts: usize,
    },
    PausePlanningRepair {
        attempt_number: usize,
        max_attempts: usize,
    },
    QueueAutoPrompt {
        prompt: String,
        queued_from_turn_id: String,
        template_label: String,
        transcript_text: String,
        handoff_task: Option<PlanningTaskHandoff>,
    },
    SkipAutoFollowup {
        reason: AutoFollowupSkipReason,
    },
}

#[derive(Debug, Clone)]
pub(super) struct ConversationRuntimeReduction {
    pub state: ConversationViewModel,
    pub effects: Vec<ConversationRuntimeEffect>,
}

pub(super) fn reduce_conversation_runtime(
    mut state: ConversationViewModel,
    event: ConversationRuntimeEvent,
) -> ConversationRuntimeReduction {
    let mut effects = Vec::new();

    match event {
        ConversationRuntimeEvent::SubmitPrompt { prompt, origin } => {
            let prompt = prompt.trim().to_string();
            if prompt.is_empty() || state.has_running_turn() {
                return ConversationRuntimeReduction { state, effects };
            }

            let thread_id = state.has_active_thread().then(|| state.thread_id.clone());
            let workspace_directory = state.planning_workspace_directory().to_string();
            match &origin {
                PromptOrigin::Manual => {
                    state.planning_repair_state = None;
                    state.auto_follow_state.reset_for_manual_turn();
                    state.clear_auto_followup_skip();
                    state.clear_last_planning_task_handoff();
                }
                PromptOrigin::AutoFollow(context) => {
                    state.auto_follow_state.mark_auto_turn_submitted();
                    state.record_auto_followup_submission(
                        &context.queued_from_turn_id,
                        &context.template_label,
                        context.handoff_task.as_ref(),
                    );
                }
                PromptOrigin::PlanningRepair(context) => {
                    state.record_planning_repair_submission(
                        context.attempt_number,
                        context.max_attempts,
                    );
                }
            }
            let auto_follow_progress = state.auto_follow_state.progress_label();
            let transcript_message = match &origin {
                PromptOrigin::AutoFollow(context) => ConversationMessage::new(
                    ConversationMessageKind::User,
                    context.transcript_text.clone(),
                    None,
                    None,
                )
                .with_display_label("Auto Follow-up"),
                _ => ConversationMessage::new(
                    ConversationMessageKind::User,
                    prompt.clone(),
                    None,
                    None,
                ),
            };
            state.record_submitted_prompt(transcript_message, workspace_directory.clone());
            state.status_text = match origin {
                PromptOrigin::Manual => "starting turn".to_string(),
                PromptOrigin::AutoFollow(context) => format!(
                    "auto follow-up submitted / turn {auto_follow_progress} / template: {}",
                    context.template_label
                ),
                PromptOrigin::PlanningRepair(context) => format!(
                    "planning repair submitted / retry {}/{}",
                    context.attempt_number, context.max_attempts
                ),
            };
            effects.push(ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
            });
        }
        ConversationRuntimeEvent::StreamUpdated(event) => match event {
            ConversationStreamEvent::ThreadPrepared {
                thread_id,
                title,
                cwd,
            } => {
                state.record_thread_prepared(thread_id, title, cwd);
            }
            ConversationStreamEvent::TurnStarted { turn_id } => {
                state.record_turn_started(turn_id);
            }
            ConversationStreamEvent::StatusUpdated { text } => {
                state.status_text = text;
            }
            ConversationStreamEvent::AgentMessageDelta {
                item_id,
                phase,
                delta,
            } => {
                state.push_live_agent_delta(item_id, phase, delta);
            }
            ConversationStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
            } => {
                state.complete_live_agent_message(item_id, phase, text);
            }
            ConversationStreamEvent::ToolActivity { activity } => {
                state.turn_activity.register_tool_activity(&activity);
                state.buffer_tool_message(activity.text);
            }
            ConversationStreamEvent::ApprovalReviewUpdated { review } => {
                state.update_approval_review(review);
            }
            ConversationStreamEvent::TurnCompleted {
                turn_id,
                changed_planning_file_paths,
            } => {
                let workspace_directory = state.finish_turn(&turn_id, &changed_planning_file_paths);
                effects.push(ConversationRuntimeEffect::EvaluateAutoFollowup {
                    workspace_directory,
                    queued_from_turn_id: turn_id,
                    changed_planning_file_paths,
                });
            }
            ConversationStreamEvent::Failed { message } => {
                state.fail_turn(message);
            }
        },
        ConversationRuntimeEvent::StreamExecutionObserved { notice } => {
            state.extend_runtime_notices([notice]);
        }
        ConversationRuntimeEvent::PostTurnEvaluated { evaluation } => {
            let evaluation = *evaluation;
            state.replace_planning_runtime_snapshot(evaluation.planning_runtime_snapshot);
            state.planning_repair_state = evaluation.planning_repair_state;
            state.extend_runtime_notices(evaluation.runtime_notices);

            let action = match evaluation.action {
                ConversationPostTurnAction::QueuePlanningRepair {
                    prompt: _,
                    queued_from_turn_id: _,
                    attempt_number,
                    max_attempts,
                } if !state.input_buffer.trim().is_empty() => {
                    ConversationPostTurnAction::PausePlanningRepair {
                        attempt_number,
                        max_attempts,
                    }
                }
                ConversationPostTurnAction::QueueAutoPrompt { .. }
                    if !state.input_buffer.trim().is_empty() =>
                {
                    ConversationPostTurnAction::SkipAutoFollowup {
                        reason: AutoFollowupSkipReason::ManualInputBuffered,
                    }
                }
                action => action,
            };

            match action {
                ConversationPostTurnAction::QueuePlanningRepair {
                    prompt,
                    queued_from_turn_id,
                    attempt_number,
                    max_attempts,
                } => {
                    state.record_planning_repair_queue(attempt_number, max_attempts);
                    state.status_text = format!(
                        "turn completed / queued planning repair {attempt_number}/{max_attempts}"
                    );
                    state.append_status_message(state.status_text.clone());
                    effects.push(ConversationRuntimeEffect::QueuePlanningRepairPrompt {
                        prompt,
                        queued_from_turn_id,
                        attempt_number,
                        max_attempts,
                    });
                }
                ConversationPostTurnAction::PausePlanningRepair {
                    attempt_number,
                    max_attempts,
                } => {
                    state.status_text = format!(
                        "turn completed / planning repair paused: manual input buffered ({attempt_number}/{max_attempts})"
                    );
                    state.append_status_message(state.status_text.clone());
                }
                ConversationPostTurnAction::QueueAutoPrompt {
                    prompt,
                    queued_from_turn_id,
                    template_label,
                    transcript_text,
                    handoff_task,
                } => {
                    state.clear_auto_followup_skip();
                    state.record_auto_followup_queue(&queued_from_turn_id, &template_label);
                    state.status_text = format!(
                        "turn completed / queued auto follow-up with template {template_label}"
                    );
                    state.append_status_message(state.status_text.clone());
                    effects.push(ConversationRuntimeEffect::QueueAutoPrompt {
                        prompt,
                        queued_from_turn_id,
                        template_label,
                        transcript_text,
                        handoff_task,
                    });
                }
                ConversationPostTurnAction::SkipAutoFollowup { reason } => {
                    state.record_auto_followup_skip(reason);
                    state.status_text = reason.runtime_status(&state.auto_follow_state);
                    state.append_status_message(state.status_text.clone());
                }
            }
        }
    }

    ConversationRuntimeReduction { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::conversation_model::PlanningRepairState;
    use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
    use crate::application::service::planning_reconciliation_service::PlanningRepairRequest;
    use crate::domain::conversation::{
        ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationToolActivity,
        ConversationToolActivityKind,
    };
    use crate::domain::followup_template::{
        FollowupTemplateCatalog, FollowupTemplateDefinition, FollowupTemplateSource,
    };

    #[test]
    fn submit_prompt_moves_state_to_submitting_and_emits_stream_effect() {
        let state = sample_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "ship it".to_string(),
                origin: PromptOrigin::Manual,
            },
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::SubmittingTurn
        );
        assert!(reduced.state.input_buffer.is_empty());
        assert_eq!(reduced.state.messages.len(), 1);
        assert_eq!(reduced.state.messages[0].text, "ship it");
        assert_eq!(
            reduced.effects,
            vec![ConversationRuntimeEffect::StartStream {
                workspace_directory: "/tmp/workspace".to_string(),
                thread_id: Some("thread-1".to_string()),
                prompt: "ship it".to_string(),
            }]
        );
    }

    #[test]
    fn auto_follow_submit_records_submission_activity_with_queue_context() {
        let mut state = sample_conversation();
        state.input_buffer = "continue from the last result".to_string();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "continue from the last result".to_string(),
                origin: PromptOrigin::AutoFollow(AutoFollowupSubmitContext {
                    queued_from_turn_id: "turn-1".to_string(),
                    template_label: "builtin next-task".to_string(),
                    transcript_text: "다음 queued task 1개를 이어서 진행합니다."
                        .to_string(),
                    handoff_task: None,
                }),
            },
        );

        assert_eq!(
            reduced.state.status_text,
            "auto follow-up submitted / turn 1/3 / template: builtin next-task"
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("submitted auto turn 1/3")
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.detail.as_str()),
            Some(
                "queued after the previous turn completed; submitted with template builtin next-task"
            )
        );
        assert_eq!(
            reduced.state.messages[0].text,
            "다음 queued task 1개를 이어서 진행합니다."
        );
        assert_eq!(reduced.state.messages[0].label(), "Auto Follow-up");
    }

    #[test]
    fn planning_repair_submit_records_retry_without_advancing_auto_follow_progress() {
        let mut state = sample_conversation();
        state.input_buffer = "repair the invalid task ledger".to_string();
        state.auto_follow_state.completed_auto_turns = 1;

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "repair the invalid task ledger".to_string(),
                origin: PromptOrigin::PlanningRepair(PlanningRepairSubmitContext {
                    queued_from_turn_id: "turn-1".to_string(),
                    attempt_number: 1,
                    max_attempts: 2,
                }),
            },
        );

        assert_eq!(reduced.state.auto_follow_state.completed_auto_turns, 1);
        assert_eq!(
            reduced.state.status_text,
            "planning repair submitted / retry 1/2"
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("submitted planning repair 1/2")
        );
        assert_eq!(
            reduced.effects,
            vec![ConversationRuntimeEffect::StartStream {
                workspace_directory: "/tmp/workspace".to_string(),
                thread_id: Some("thread-1".to_string()),
                prompt: "repair the invalid task ledger".to_string(),
            }]
        );
    }

    #[test]
    fn manual_submit_clears_pending_planning_repair_state() {
        let mut state = sample_conversation();
        state.planning_repair_state = Some(PlanningRepairState {
            root_turn_id: "turn-root".to_string(),
            attempts_used: 1,
            max_attempts: 2,
            latest_request: PlanningRepairRequest {
                failure_summary: "failed to parse task-ledger.json".to_string(),
                validation_errors: vec!["failed to parse task-ledger.json".to_string()],
                directions_toml: "version = 1".to_string(),
                task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
                accepted_task_ledger_json: "{\"version\":1,\"tasks\":[]}".to_string(),
                rejected_task_ledger_json: Some("{ invalid json".to_string()),
                rejected_archive_path: None,
            },
        });

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "operator override".to_string(),
                origin: PromptOrigin::Manual,
            },
        );

        assert!(reduced.state.planning_repair_state.is_none());
    }

    #[test]
    fn turn_completed_queues_auto_prompt_effect_when_allowed() {
        let mut state = sample_conversation();
        state.input_buffer.clear();
        state.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        state.input_state = ConversationInputState::StreamingTurn;
        state.active_turn_id = Some("turn-1".to_string());

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: Vec::new(),
            }),
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::ReadyToContinue
        );
        assert!(matches!(
            reduced.effects.as_slice(),
            [ConversationRuntimeEffect::EvaluateAutoFollowup {
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
            }] if workspace_directory == "/tmp/workspace"
                && queued_from_turn_id == "turn-1"
                && changed_planning_file_paths.is_empty()
        ));
        assert!(reduced.state.last_auto_followup_activity.is_none());
        assert_eq!(reduced.state.messages.len(), 1);
        assert_eq!(
            reduced.state.messages[0].kind,
            ConversationMessageKind::Agent
        );
    }

    #[test]
    fn approval_review_update_sets_status_and_summary_state() {
        let state = sample_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(
                ConversationStreamEvent::ApprovalReviewUpdated {
                    review: ConversationApprovalReview {
                        target_item_id: "command-1".to_string(),
                        status: ConversationApprovalReviewStatus::InProgress,
                        risk_level: Some("high".to_string()),
                        rationale: None,
                    },
                },
            ),
        );

        assert_eq!(
            reduced.state.status_text,
            "approval review in progress / target: command-1 / risk: high"
        );
        assert_eq!(
            reduced
                .state
                .approval_review
                .as_ref()
                .map(|review| review.status.clone()),
            Some(ConversationApprovalReviewStatus::InProgress)
        );
    }

    #[test]
    fn turn_started_clears_previous_approval_review_state() {
        let mut state = sample_conversation();
        state.approval_review = Some(ConversationApprovalReview {
            target_item_id: "command-1".to_string(),
            status: ConversationApprovalReviewStatus::Approved,
            risk_level: Some("medium".to_string()),
            rationale: None,
        });

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnStarted {
                turn_id: "turn-2".to_string(),
            }),
        );

        assert!(reduced.state.approval_review.is_none());
        assert_eq!(
            reduced.state.messages.last().map(|message| message.kind),
            Some(ConversationMessageKind::Status)
        );
        assert_eq!(
            reduced
                .state
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some("turn started")
        );
    }

    #[test]
    fn thread_prepared_appends_open_marker_to_history() {
        let state = sample_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-2".to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/loaded".to_string(),
            }),
        );

        assert_eq!(reduced.state.thread_id, "thread-2");
        assert_eq!(reduced.state.title, "Loaded thread");
        assert_eq!(reduced.state.cwd, "/tmp/loaded");
        assert_eq!(reduced.state.status_text, "thread started");
        assert_eq!(
            reduced.state.messages.last().map(|message| message.kind),
            Some(ConversationMessageKind::Status)
        );
        assert_eq!(
            reduced
                .state
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some("thread opened / Loaded thread")
        );
        assert_eq!(
            reduced.state.cached_conversation_lines,
            format_conversation_lines(&reduced.state.messages)
        );
    }

    #[test]
    fn submit_prompt_ignores_blank_prompt_after_trim() {
        let state = sample_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "   \n\t  ".to_string(),
                origin: PromptOrigin::Manual,
            },
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::ReadyToContinue
        );
        assert_eq!(reduced.state.input_buffer, "ship it");
        assert!(reduced.state.messages.is_empty());
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn submit_prompt_ignores_requests_while_turn_is_running() {
        let mut state = sample_conversation();
        state.input_state = ConversationInputState::StreamingTurn;
        state.active_turn_id = Some("turn-1".to_string());

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "ship it".to_string(),
                origin: PromptOrigin::Manual,
            },
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::StreamingTurn
        );
        assert_eq!(reduced.state.active_turn_id.as_deref(), Some("turn-1"));
        assert!(reduced.state.messages.is_empty());
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn turn_completed_emits_auto_followup_evaluation_effect_when_auto_followup_cannot_continue() {
        let state = sample_active_turn_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: Vec::new(),
            }),
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::ReadyToContinue
        );
        assert!(matches!(
            reduced.effects.as_slice(),
            [ConversationRuntimeEffect::EvaluateAutoFollowup {
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
            }] if workspace_directory == "/tmp/workspace"
                && queued_from_turn_id == "turn-1"
                && changed_planning_file_paths.is_empty()
        ));
        assert!(reduced.state.last_auto_followup_activity.is_none());
        assert!(reduced.state.messages.is_empty());
    }

    #[test]
    fn turn_completed_defers_no_file_change_stop_decision_to_evaluation_effect() {
        let mut state = sample_active_turn_conversation();
        state.auto_follow_state.stop_rules.stop_on_no_file_changes = true;
        state.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: Vec::new(),
            }),
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::ReadyToContinue
        );
        assert!(matches!(
            reduced.effects.as_slice(),
            [ConversationRuntimeEffect::EvaluateAutoFollowup {
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
            }] if workspace_directory == "/tmp/workspace"
                && queued_from_turn_id == "turn-1"
                && changed_planning_file_paths.is_empty()
        ));
        assert!(reduced.state.last_auto_followup_activity.is_none());
    }

    #[test]
    fn turn_completed_preserves_changed_planning_file_paths_for_followup_evaluation() {
        let state = sample_active_turn_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec![
                    crate::domain::planning::DIRECTIONS_FILE_PATH.to_string(),
                    crate::domain::planning::TASK_LEDGER_FILE_PATH.to_string(),
                ],
            }),
        );

        assert_eq!(
            reduced.effects,
            vec![ConversationRuntimeEffect::EvaluateAutoFollowup {
                workspace_directory: "/tmp/workspace".to_string(),
                queued_from_turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec![
                    crate::domain::planning::DIRECTIONS_FILE_PATH.to_string(),
                    crate::domain::planning::TASK_LEDGER_FILE_PATH.to_string(),
                ],
            }]
        );
        assert_eq!(
            reduced
                .state
                .turn_activity
                .last_completed_changed_planning_file_paths(),
            [
                crate::domain::planning::DIRECTIONS_FILE_PATH.to_string(),
                crate::domain::planning::TASK_LEDGER_FILE_PATH.to_string(),
            ]
        );
    }

    #[test]
    fn tool_activity_updates_recent_summary_and_turn_counters() {
        let state = sample_active_turn_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::ToolActivity {
                activity: ConversationToolActivity {
                    kind: ConversationToolActivityKind::CommandExecution,
                    text: "command: cargo test [completed]".to_string(),
                    file_change_count: 0,
                },
            }),
        );

        assert_eq!(reduced.state.turn_activity.current_turn_command_count, 1);
        assert_eq!(
            reduced.state.turn_activity.current_turn_file_change_count,
            0
        );
        assert_eq!(
            reduced.state.turn_activity.activity_summary(true),
            "command: cargo test [completed]"
        );
        assert!(reduced.state.messages.is_empty());
        assert_eq!(reduced.state.buffered_tool_messages.len(), 1);
        assert_eq!(
            reduced.state.buffered_tool_messages[0].kind,
            ConversationMessageKind::Tool
        );
    }

    #[test]
    fn turn_completed_flushes_buffered_tool_messages_into_stable_history() {
        let mut state = sample_active_turn_conversation();
        state.turn_activity.current_turn_command_count = 1;
        state.buffer_tool_message("command: cargo test [completed]");

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: Vec::new(),
            }),
        );

        assert!(reduced.state.buffered_tool_messages.is_empty());
        assert!(
            reduced
                .state
                .messages
                .iter()
                .any(|message| message.kind == ConversationMessageKind::Tool
                    && message.text == "command: cargo test [completed]")
        );
        assert_eq!(
            reduced.state.cached_conversation_lines,
            format_conversation_lines(&reduced.state.messages)
        );
    }

    #[test]
    fn agent_message_delta_stays_in_live_region_until_completion() {
        let state = sample_active_turn_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::AgentMessageDelta {
                item_id: "agent-1".to_string(),
                phase: Some("final_answer".to_string()),
                delta: "partial answer".to_string(),
            }),
        );

        assert!(reduced.state.messages.is_empty());
        assert_eq!(
            reduced
                .state
                .live_agent_message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("partial answer")
        );
        assert_eq!(
            reduced.state.cached_conversation_lines,
            format_conversation_lines(&[])
        );
    }

    #[test]
    fn agent_message_completion_commits_live_output_to_stable_history() {
        let mut state = sample_active_turn_conversation();
        state.live_agent_message = Some(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "partial answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(
                ConversationStreamEvent::AgentMessageCompleted {
                    item_id: "agent-1".to_string(),
                    phase: Some("final_answer".to_string()),
                    text: "completed answer".to_string(),
                },
            ),
        );

        assert!(reduced.state.live_agent_message.is_none());
        assert_eq!(reduced.state.messages.len(), 1);
        assert_eq!(reduced.state.messages[0].text, "completed answer");
        assert_eq!(
            reduced.state.cached_conversation_lines,
            format_conversation_lines(&reduced.state.messages)
        );
    }

    #[test]
    fn turn_completed_carries_command_activity_into_last_turn_summary() {
        let mut state = sample_active_turn_conversation();
        state.turn_activity.current_turn_command_count = 1;
        state.turn_activity.current_turn_file_change_count = 2;
        state.turn_activity.current_turn_last_summary =
            Some("file change: update src/app.rs".to_string());
        state.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: Vec::new(),
            }),
        );

        assert_eq!(reduced.state.turn_activity.current_turn_command_count, 0);
        assert_eq!(
            reduced.state.turn_activity.current_turn_file_change_count,
            0
        );
        assert_eq!(
            reduced
                .state
                .turn_activity
                .last_completed_turn_command_count,
            1
        );
        assert_eq!(
            reduced
                .state
                .turn_activity
                .last_completed_file_change_count(),
            2
        );
        assert_eq!(
            reduced.state.turn_activity.activity_summary(false),
            "file change: update src/app.rs"
        );
    }

    #[test]
    fn stream_failure_marks_turn_finished_and_appends_status_message() {
        let state = sample_active_turn_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::Failed {
                message: "stream exploded".to_string(),
            }),
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::ReadyToContinue
        );
        assert!(reduced.state.active_turn_id.is_none());
        assert_eq!(reduced.state.status_text, "turn failed");
        assert_eq!(reduced.state.messages.len(), 1);
        assert_eq!(
            reduced.state.messages[0].kind,
            ConversationMessageKind::Status
        );
        assert_eq!(reduced.state.messages[0].text, "stream exploded");
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn stream_failure_flushes_buffered_tool_messages_before_status_message() {
        let mut state = sample_active_turn_conversation();
        state.buffer_tool_message("command: cargo test [failed]");

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::Failed {
                message: "stream exploded".to_string(),
            }),
        );

        assert_eq!(reduced.state.messages.len(), 2);
        assert_eq!(
            reduced.state.messages[0].kind,
            ConversationMessageKind::Tool
        );
        assert_eq!(
            reduced.state.messages[0].text,
            "command: cargo test [failed]"
        );
        assert_eq!(
            reduced.state.messages[1].kind,
            ConversationMessageKind::Status
        );
        assert_eq!(reduced.state.messages[1].text, "stream exploded");
    }

    #[test]
    fn post_turn_evaluation_queues_auto_prompt_from_reducer_state_transition() {
        let mut state = sample_conversation();
        state.input_buffer.clear();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::PostTurnEvaluated {
                evaluation: Box::new(ConversationPostTurnEvaluation {
                    planning_runtime_snapshot: PlanningRuntimeSnapshot::invalid(
                        "planning queue needs confirmation".to_string(),
                    ),
                    planning_repair_state: None,
                    runtime_notices: vec!["planning reconciliation completed".to_string()],
                    action: ConversationPostTurnAction::QueueAutoPrompt {
                        prompt: "continue".to_string(),
                        queued_from_turn_id: "turn-1".to_string(),
                        template_label: "builtin next-task".to_string(),
                        transcript_text: "다음 queued task 1개를 이어서 진행합니다."
                            .to_string(),
                        handoff_task: None,
                    },
                }),
            },
        );

        assert_eq!(
            reduced.effects,
            vec![ConversationRuntimeEffect::QueueAutoPrompt {
                prompt: "continue".to_string(),
                queued_from_turn_id: "turn-1".to_string(),
                template_label: "builtin next-task".to_string(),
                transcript_text: "다음 queued task 1개를 이어서 진행합니다."
                    .to_string(),
                handoff_task: None,
            }]
        );
        assert_eq!(
            reduced.state.status_text,
            "turn completed / queued auto follow-up with template builtin next-task"
        );
        assert!(
            reduced
                .state
                .runtime_notices
                .contains(&"planning reconciliation completed".to_string())
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("queued auto turn 1/3")
        );
    }

    #[test]
    fn post_turn_evaluation_rechecks_manual_input_before_queueing_auto_prompt() {
        let mut state = sample_conversation();
        state.input_buffer = "user is typing".to_string();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::PostTurnEvaluated {
                evaluation: Box::new(ConversationPostTurnEvaluation {
                    planning_runtime_snapshot: PlanningRuntimeSnapshot::invalid(
                        "planning queue needs confirmation".to_string(),
                    ),
                    planning_repair_state: None,
                    runtime_notices: vec!["planning reconciliation completed".to_string()],
                    action: ConversationPostTurnAction::QueueAutoPrompt {
                        prompt: "continue".to_string(),
                        queued_from_turn_id: "turn-1".to_string(),
                        template_label: "builtin next-task".to_string(),
                        transcript_text: "다음 queued task 1개를 이어서 진행합니다."
                            .to_string(),
                        handoff_task: None,
                    },
                }),
            },
        );

        assert!(reduced.effects.is_empty());
        assert_eq!(
            reduced.state.status_text,
            "turn completed / auto follow-up skipped: manual input buffered"
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("skipped: manual input buffered")
        );
    }

    #[test]
    fn post_turn_evaluation_pauses_planning_repair_without_queueing_effect() {
        let state = sample_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::PostTurnEvaluated {
                evaluation: Box::new(ConversationPostTurnEvaluation {
                    planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
                    planning_repair_state: Some(PlanningRepairState {
                        root_turn_id: "turn-root".to_string(),
                        attempts_used: 1,
                        max_attempts: 2,
                        latest_request: PlanningRepairRequest {
                            failure_summary: "failed to parse task-ledger.json".to_string(),
                            validation_errors: vec!["failed to parse task-ledger.json".to_string()],
                            directions_toml: "version = 1".to_string(),
                            task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
                            accepted_task_ledger_json: "{\"version\":1,\"tasks\":[]}".to_string(),
                            rejected_task_ledger_json: Some("{ invalid json".to_string()),
                            rejected_archive_path: None,
                        },
                    }),
                    runtime_notices: vec![
                        "planning repair retry 1/2 is waiting because manual input is buffered"
                            .to_string(),
                    ],
                    action: ConversationPostTurnAction::PausePlanningRepair {
                        attempt_number: 1,
                        max_attempts: 2,
                    },
                }),
            },
        );

        assert!(reduced.effects.is_empty());
        assert_eq!(
            reduced.state.status_text,
            "turn completed / planning repair paused: manual input buffered (1/2)"
        );
        assert_eq!(
            reduced
                .state
                .planning_repair_state
                .as_ref()
                .map(|state| state.attempts_used),
            Some(1)
        );
        assert!(
            reduced
                .state
                .runtime_notices
                .iter()
                .any(|notice| notice.contains("planning repair retry 1/2"))
        );
    }

    #[test]
    fn stream_execution_observed_accumulates_runtime_notice_without_effects() {
        let state = sample_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamExecutionObserved {
                notice: "turn stream returned an error after the terminal event: transport closed"
                    .to_string(),
            },
        );

        assert!(reduced.effects.is_empty());
        assert_eq!(
            reduced.state.runtime_notices,
            vec![
                "turn stream returned an error after the terminal event: transport closed"
                    .to_string()
            ]
        );
    }

    fn sample_conversation() -> ConversationViewModel {
        ConversationViewModel {
            thread_id: "thread-1".to_string(),
            title: "Existing session".to_string(),
            cwd: "/tmp/workspace".to_string(),
            draft_workspace_directory: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            cached_conversation_lines: format_conversation_lines(&[]),
            live_agent_message: None,
            buffered_tool_messages: Vec::new(),
            base_warnings: Vec::new(),
            template_warnings: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            input_buffer: "ship it".to_string(),
            startup_submit_armed: false,
            active_turn_id: None,
            active_turn_workspace_directory: None,
            planning_repair_state: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(FollowupTemplateCatalog {
                items: vec![FollowupTemplateDefinition {
                    id: "builtin-next-task".to_string(),
                    label: "builtin next-task".to_string(),
                    body: "follow up {auto_turn}/{max_auto_turns}\n{last_message}".to_string(),
                    source: FollowupTemplateSource::Builtin,
                }],
            }),
            planning_runtime_snapshot: PlanningRuntimeSnapshot::uninitialized(),
            turn_activity: TurnActivityState::default(),
            approval_review: None,
            last_auto_followup_activity: None,
            last_planning_task_handoff: None,
            status_text: "thread loaded".to_string(),
        }
    }

    fn sample_active_turn_conversation() -> ConversationViewModel {
        let mut state = sample_conversation();
        state.input_buffer.clear();
        state.input_state = ConversationInputState::StreamingTurn;
        state.active_turn_id = Some("turn-1".to_string());
        state.active_turn_workspace_directory = Some("/tmp/workspace".to_string());
        state
    }
}
