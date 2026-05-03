/*
 * conversation_runtime.rs is the reducer/effect boundary for the live TUI
 * conversation. App-server streaming, post-turn planning checks, and auto-follow
 * submission all meet here as events, but this module does not perform I/O.
 *
 * The split matters because a single turn can receive keyboard submissions,
 * provider stream notifications, planning repair notices, and auto-follow
 * decisions on different threads. The reducer serializes those facts into
 * ConversationViewModel and returns explicit effects for turn_submission_runtime
 * to execute.
 */
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
    /*
     * Runtime events are facts that already happened at the TUI boundary. Manual
     * and auto-follow submissions enter through SubmitPrompt, provider messages
     * enter through StreamUpdated, and planning post-turn policy returns through
     * PostTurnEvaluated.
     */
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
    /*
     * Effects are work requests the reducer must not run directly. This keeps
     * app-server stream startup, planning evaluation, and internal prompt
     * re-submission outside state mutation, while still making their ordering
     * visible to app_runtime.
     */
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
    // Fresh planning projection after the just-finished turn. It replaces the
    // embedded conversation snapshot before auto-follow copy is derived.
    pub planning_runtime_snapshot: PlanningRuntimeSnapshot,
    // Repair state is presentation state, but it is decided by post-turn
    // execution where planning files and runtime diagnostics are inspected.
    pub planning_repair_state: Option<PlanningRepairState>,
    // Runtime notices are appended after the turn so footer/status panels can
    // surface planning repairs, skipped work, or provider execution details.
    pub runtime_notices: Vec<String>,
    // The post-turn policy either schedules the next internal prompt or records
    // the reason the loop stopped.
    pub action: ConversationPostTurnAction,
}
#[derive(Debug, Clone)]
pub(super) struct QueuedAutoPrompt {
    // Prompt sent to app-server. It can include planning context not meant to be
    // shown verbatim in transcript.
    pub prompt: String,
    // Turn id used to prevent repeatedly queuing auto-follow for the same queue
    // head after retries or delayed background messages.
    pub queued_from_turn_id: String,
    // Human label for the auto-follow policy that produced this prompt.
    pub mode_label: String,
    // Transcript marker shown to the operator; deliberately distinct from the
    // executable prompt text above.
    pub transcript_text: String,
    // Optional planning task identity when this auto-follow is tied to a concrete
    // queue handoff.
    pub handoff_task: Option<PlanningTaskHandoff>,
}
#[derive(Debug, Clone)]
pub(super) enum ConversationPostTurnAction {
    // Box keeps the enum small because queued prompts carry several strings and
    // optional handoff metadata through the background-message channel.
    QueueAutoPrompt(Box<QueuedAutoPrompt>),
    SkipAutoFollowup { reason: AutoFollowupSkipReason },
}
#[derive(Debug, Clone)]
pub(super) struct ConversationRuntimeReduction {
    // Updated view model that shell rendering reads immediately after dispatch.
    pub state: ConversationViewModel,
    // Effects are executed after state replacement, so UI copy can already show
    // "streaming/evaluating/queued" while background work starts.
    pub effects: Vec<ConversationRuntimeEffect>,
}
pub(super) fn reduce_conversation_runtime(
    mut state: ConversationViewModel,
    event: ConversationRuntimeEvent,
) -> ConversationRuntimeReduction {
    /*
     * The reducer always mutates local state before returning effects. That order
     * ensures the shell can render a coherent "starting turn", "evaluating", or
     * "queued auto follow-up" state even if the next background message arrives
     * quickly.
     */
    let mut effects = Vec::new();
    match event {
        ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            transcript_text,
            origin,
        } => {
            /*
             * Manual and auto-follow prompts share stream startup, but their
             * bookkeeping differs. Manual input resets planning repair and loop
             * state; auto-follow input records its queue provenance before the
             * stream starts so the next post-turn policy can detect repetition.
             */
            let prompt = prompt.trim().to_string();
            if prompt.is_empty() || !state.can_accept_runtime_prompt() {
                // Empty prompts and prompts sent while the runtime is not ready
                // are ignored rather than turned into provider calls.
                return ConversationRuntimeReduction { state, effects };
            }
            if matches!(origin, PromptOrigin::Manual) && !state.can_accept_manual_prompt() {
                // Manual prompts are stricter than internal auto-follow prompts:
                // startup gates and input state can block the operator even when
                // an internally queued follow-up is allowed to continue.
                return ConversationRuntimeReduction { state, effects };
            }
            let thread_id = state.has_active_thread().then(|| state.thread_id.clone());
            let workspace_directory = state.planning_workspace_directory().to_string();
            match &origin {
                PromptOrigin::Manual => {
                    // A manual turn is a new operator decision, so previous
                    // repair prompts, skip reasons, and handoff identity should
                    // not leak into this turn.
                    state.planning_repair_state = None;
                    state.auto_follow_state.reset_for_manual_turn();
                    state.clear_auto_followup_skip();
                    state.clear_last_planning_task_handoff();
                }
                PromptOrigin::AutoFollow(context) => {
                    // Record the source turn before the provider stream starts.
                    // If this queued prompt loops back without progress, the
                    // next post-turn evaluation can stop it deterministically.
                    state.record_auto_followup_submission(
                        &context.queued_from_turn_id,
                        context.handoff_task.as_ref(),
                    );
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
                    // Auto-follow transcript copy is intentionally not the raw
                    // prompt. Debug detail can reveal the generated prompt when
                    // operator diagnostics are enabled.
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
            };
            effects.push(ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
            });
        }
        ConversationRuntimeEvent::StreamUpdated(event) => match event {
            ConversationStreamEvent::AttachmentObserved { profile } => {
                // Attachment information is a runtime notice, not a transcript
                // row, because it describes bridge recovery rather than model
                // conversation content.
                state.extend_runtime_notices([attachment_runtime_notice(profile)]);
            }
            ConversationStreamEvent::ThreadPrepared {
                thread_id,
                title,
                cwd,
            } => {
                // Thread preparation binds provider identity and cwd to the
                // conversation before turn events start appending transcript.
                state.record_thread_prepared(thread_id, title, cwd);
            }
            ConversationStreamEvent::TurnStarted { turn_id } => {
                // Turn id is later used by TurnCompleted and auto-follow
                // provenance, so it is recorded as soon as the provider reports
                // start.
                state.record_turn_started(turn_id);
            }
            ConversationStreamEvent::StatusUpdated { text } => {
                // Provider status copy owns the main status line while a turn is
                // active, but it does not become durable transcript history.
                state.status_text = text;
            }
            ConversationStreamEvent::AgentMessageDelta {
                item_id,
                phase,
                delta,
            } => {
                // Deltas stay in the live buffer until completion, preserving
                // streaming responsiveness without committing partial transcript
                // rows as final history.
                state.push_live_agent_delta(item_id, phase, delta);
            }
            ConversationStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
            } => {
                // Completion either flushes the live buffer or patches the final
                // transcript row for the provider item.
                state.complete_live_agent_message(item_id, phase, text);
            }
            ConversationStreamEvent::ToolActivity { activity } => {
                // Tool activity feeds both compact live counters and ordered
                // transcript notices so shell tail and transcript agree.
                state.turn_activity.register_tool_activity(&activity);
                state.buffer_tool_message(activity.text);
            }
            ConversationStreamEvent::ApprovalReviewUpdated { review } => {
                // Some provider statuses require approval outside the visible
                // shell. Add a runtime notice before updating the stored review
                // so footer/status panes can explain the handoff.
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
                // Turn completion closes the provider stream but does not decide
                // whether to auto-follow. That policy needs fresh planning state,
                // so it is emitted as an effect after the model enters evaluating
                // state.
                let workspace_directory = state.finish_turn(&turn_id, &changed_planning_file_paths);
                state.begin_auto_followup_evaluation();
                effects.push(ConversationRuntimeEffect::EvaluateAutoFollowup {
                    workspace_directory,
                    queued_from_turn_id: turn_id,
                    changed_planning_file_paths,
                });
            }
            ConversationStreamEvent::Failed { message } => {
                // Failure ends the active turn locally. No post-turn evaluation
                // is scheduled because planning side effects may be incomplete.
                state.fail_turn(message);
            }
        },
        ConversationRuntimeEvent::StreamExecutionObserved { notice } => {
            // Execution-layer notices come from effect runners, not provider
            // stream events. They are still runtime notices so the user can see
            // background execution failures in the same place.
            state.extend_runtime_notices([notice]);
        }
        ConversationRuntimeEvent::PostTurnEvaluated { evaluation } => {
            let evaluation = *evaluation;
            // Apply the new planning view before acting on the decision; queued
            // or skipped auto-follow copy should describe the latest queue state.
            state.replace_planning_runtime_snapshot(evaluation.planning_runtime_snapshot);
            state.planning_repair_state = evaluation.planning_repair_state;
            state.extend_runtime_notices(evaluation.runtime_notices);
            match evaluation.action {
                ConversationPostTurnAction::QueueAutoPrompt(queued_prompt) => {
                    // Queueing records the pending loop in visible history before
                    // emitting QueueAutoPrompt. The effect will re-enter this
                    // reducer as SubmitPrompt with PromptOrigin::AutoFollow.
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
                    // Skips are durable status messages because they explain why
                    // the automatic loop stopped and often require operator
                    // action before the next manual prompt.
                    state.record_auto_followup_skip(reason);
                    state.status_text = reason.runtime_status(&state.auto_follow_state);
                    state.append_status_message(state.status_text.clone());
                }
            }
        }
    }

    ConversationRuntimeReduction { state, effects }
}
