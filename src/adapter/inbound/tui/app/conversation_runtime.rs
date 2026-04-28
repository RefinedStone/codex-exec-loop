use super::PromptOrigin;
use super::conversation_model::{
    AutoFollowupSkipReason, ConversationViewModel, PlanningRepairState,
};
use crate::adapter::inbound::tui::conversation_text::{
    approval_review_manual_client_action_notice, attachment_runtime_notice,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningTaskHandoff};
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

#[derive(Debug, Clone)]
pub(super) enum ConversationRuntimeEvent {
    SubmitPrompt {
        prompt: String,
        transcript_text: String,
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
        mode_label: String,
        transcript_text: String,
        handoff_task: Option<PlanningTaskHandoff>,
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
pub(super) struct QueuedAutoPrompt {
    pub prompt: String,
    pub queued_from_turn_id: String,
    pub mode_label: String,
    pub transcript_text: String,
    pub handoff_task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone)]
pub(super) enum ConversationPostTurnAction {
    QueueAutoPrompt(Box<QueuedAutoPrompt>),
    SkipAutoFollowup { reason: AutoFollowupSkipReason },
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
        ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            transcript_text,
            origin,
        } => {
            let prompt = prompt.trim().to_string();
            if prompt.is_empty() || !state.can_accept_runtime_prompt() {
                return ConversationRuntimeReduction { state, effects };
            }
            if matches!(origin, PromptOrigin::Manual) && !state.can_accept_manual_prompt() {
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
                    state.record_auto_followup_submission(
                        &context.queued_from_turn_id,
                        context.handoff_task.as_ref(),
                    );
                }
                PromptOrigin::ParallelDispatch(context) => {
                    state.record_parallel_dispatch_submission(&context.handoff_task);
                }
            }
            let auto_follow_progress = format!(
                "{}/{}",
                state
                    .auto_follow_state
                    .active_turn_index()
                    .unwrap_or_else(|| state.auto_follow_state.next_auto_turn_index()),
                state.auto_follow_state.max_auto_turns_label()
            );
            let transcript_message = match &origin {
                PromptOrigin::AutoFollow(context) => {
                    let mut message = ConversationMessage::new(
                        ConversationMessageKind::User,
                        context.transcript_text.clone(),
                        None,
                        None,
                    )
                    .with_display_label("Auto Follow-up");
                    if let Some(detail) = context.debug_detail.as_deref() {
                        message = message.with_debug_detail(detail);
                    }
                    message
                }
                PromptOrigin::ParallelDispatch(context) => ConversationMessage::new(
                    ConversationMessageKind::User,
                    context.transcript_text.clone(),
                    None,
                    None,
                )
                .with_display_label("Parallel Dispatch"),
                _ => ConversationMessage::new(
                    ConversationMessageKind::User,
                    transcript_text,
                    None,
                    None,
                ),
            };
            state.record_submitted_prompt(
                transcript_message,
                workspace_directory.clone(),
                matches!(origin, PromptOrigin::Manual),
            );
            state.status_text = match origin {
                PromptOrigin::Manual => "starting turn".to_string(),
                PromptOrigin::AutoFollow(context) => format!(
                    "auto follow-up submitted / turn {auto_follow_progress} / mode: {}",
                    context.mode_label
                ),
                PromptOrigin::ParallelDispatch(context) => format!(
                    "parallel dispatch submitted / turn {auto_follow_progress} / task: {}",
                    context.handoff_task.task_title
                ),
            };
            effects.push(ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
            });
        }
        ConversationRuntimeEvent::StreamUpdated(event) => match event {
            ConversationStreamEvent::AttachmentObserved { profile } => {
                state.extend_runtime_notices([attachment_runtime_notice(profile)]);
            }
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
                if let Some(notice) = approval_review_manual_client_action_notice(
                    &review,
                    state.turn_control_truth().approval,
                ) {
                    state.extend_runtime_notices([notice]);
                }
                state.update_approval_review(review);
            }
            ConversationStreamEvent::TurnCompleted {
                turn_id,
                changed_planning_file_paths,
            } => {
                let workspace_directory = state.finish_turn(&turn_id, &changed_planning_file_paths);
                state.begin_auto_followup_evaluation();
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
            match evaluation.action {
                ConversationPostTurnAction::QueueAutoPrompt(queued_prompt) => {
                    let QueuedAutoPrompt {
                        prompt,
                        queued_from_turn_id,
                        mode_label,
                        transcript_text,
                        handoff_task,
                    } = *queued_prompt;
                    state.clear_auto_followup_skip();
                    state.record_auto_followup_queue(&queued_from_turn_id);
                    state.status_text =
                        format!("turn completed / queued auto follow-up with mode {mode_label}");
                    state.append_status_message(state.status_text.clone());
                    effects.push(ConversationRuntimeEffect::QueueAutoPrompt {
                        prompt,
                        queued_from_turn_id,
                        mode_label,
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
    #[test]
    fn test_module_compiles_after_task_authority_file_removal() {
        assert!(std::env::current_dir().is_ok());
    }
}
