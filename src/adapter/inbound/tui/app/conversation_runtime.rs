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
use super::conversation_model::{AutoFollowSkipReason, ConversationViewModel, PlanningRepairState};
use crate::adapter::inbound::tui::conversation_text::{
    approval_review_manual_client_action_notice, attachment_runtime_notice,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningTaskHandoff};
use crate::diagnostics::event_log;
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};
use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;
use serde_json::json;
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
    EvaluateAutoFollow {
        workspace_directory: String,
        completed_turn_id: String,
        changed_planning_file_paths: Vec<String>,
    },
    QueueAutoPrompt {
        prompt: String,
        completed_turn_id: String,
        mode_label: String,
        transcript_text: String,
        handoff_task: Option<PlanningTaskHandoff>,
    },
    DispatchOperatorAlert {
        alert: OperatorAlert,
    },
}
#[derive(Debug, Clone)]
pub(super) struct ConversationPostTurnEvaluation {
    // Fresh planning projection after the just-finished turn. It replaces the
    // embedded conversation snapshot before auto-follow copy is derived.
    pub runtime_snapshot: PlanningRuntimeSnapshot,
    // Repair state is presentation state, but it is decided by post-turn
    // execution where planning files and runtime diagnostics are inspected.
    pub planning_repair_state: Option<PlanningRepairState>,
    // Runtime notices are appended after the turn so footer/status panels can
    // surface planning repairs, skipped work, or provider execution details.
    pub runtime_notices: Vec<String>,
    // The post-turn policy either schedules the next internal prompt or records
    // the reason the loop stopped.
    pub action: ConversationPostTurnAction,
    // Parallel orchestration handoff is independent from auto-follow copy. Keeping
    // this separate prevents SkipAutoFollow reasons from becoming hidden control
    // signals for the supervisor.
    pub parallel_queue_signal: Option<ParallelModePostTurnQueueSignal>,
    // Operator alerts are explicit post-turn outputs, not inferred by the reducer
    // from auto-follow skip copy.
    pub operator_alerts: Vec<OperatorAlert>,
}
#[derive(Debug, Clone)]
pub(super) struct QueuedAutoPrompt {
    // Prompt sent to app-server. It can include planning context not meant to be
    // shown verbatim in transcript.
    pub prompt: String,
    // Turn id used to prevent repeatedly queuing auto-follow for the same queue
    // head after retries or delayed background messages.
    pub completed_turn_id: String,
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
    // optional handoff identity through the background-message channel.
    QueueAutoPrompt(Box<QueuedAutoPrompt>),
    SkipAutoFollow { reason: AutoFollowSkipReason },
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
     * "queued auto-follow" state even if the next background message arrives
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
                event_log::emit_lazy("prompt_submission_ignored", || {
                    json!({
                        "origin": prompt_origin_label(&origin),
                        "reason": if prompt.is_empty() {
                            "empty_prompt"
                        } else {
                            "runtime_prompt_not_acceptable"
                        },
                        "status_text": state.status_text,
                        "input_ready": state.can_accept_runtime_prompt(),
                        "manual_input_ready": state.can_accept_manual_prompt(),
                    })
                });
                return ConversationRuntimeReduction { state, effects };
            }
            if matches!(origin, PromptOrigin::Manual | PromptOrigin::ManualIntake(_))
                && !state.can_accept_manual_prompt()
            {
                // Manual prompts are stricter than internal auto-follow prompts:
                // startup gates and input state can block the operator even when
                // an internally queued follow-up is allowed to continue.
                event_log::emit_lazy("prompt_submission_ignored", || {
                    json!({
                        "origin": "Manual",
                        "reason": "manual_prompt_not_acceptable",
                        "status_text": state.status_text,
                        "input_ready": state.can_accept_runtime_prompt(),
                        "manual_input_ready": state.can_accept_manual_prompt(),
                    })
                });
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
                    state.clear_auto_follow_skip();
                    state.clear_last_planning_task_handoff();
                }
                PromptOrigin::ManualIntake(context) => {
                    state.planning_repair_state = None;
                    state.auto_follow_state.reset_for_manual_turn();
                    state.clear_auto_follow_skip();
                    state.record_manual_intake_handoff(context.handoff_task.as_ref());
                }
                PromptOrigin::AutoFollow(context) => {
                    // Record the completed turn that queued this prompt before the provider stream starts.
                    // If this queued prompt loops back without progress, the
                    // next post-turn evaluation can stop it deterministically.
                    state.record_auto_follow_submission(
                        &context.completed_turn_id,
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
                PromptOrigin::Manual | PromptOrigin::ManualIntake(_) => ConversationMessage::new(
                    ConversationMessageKind::User,
                    transcript_text,
                    None,
                    None,
                ),
            };
            state.record_submitted_prompt(
                transcript_message,
                workspace_directory.clone(),
                matches!(origin, PromptOrigin::Manual | PromptOrigin::ManualIntake(_)),
            );
            state.status_text = match origin {
                PromptOrigin::Manual | PromptOrigin::ManualIntake(_) => "starting turn".to_string(),
                PromptOrigin::AutoFollow(context) => format!(
                    "auto-follow submitted / turn {auto_follow_progress} / mode: {}",
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
                state.begin_auto_follow_evaluation();
                event_log::emit_lazy("post_turn_evaluation_queued", || {
                    json!({
                        "thread_id": state.thread_id.as_str(),
                        "completed_turn_id": turn_id,
                        "workspace_directory": workspace_directory,
                        "operation": "post_turn",
                        "phase": "queued",
                        "decision": "evaluate",
                        "changed_planning_file_count": changed_planning_file_paths.len(),
                    })
                });
                effects.push(ConversationRuntimeEffect::EvaluateAutoFollow {
                    workspace_directory,
                    completed_turn_id: turn_id,
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
            state.replace_planning_runtime_snapshot(evaluation.runtime_snapshot);
            state.planning_repair_state = evaluation.planning_repair_state;
            state.extend_runtime_notices(evaluation.runtime_notices);
            match evaluation.action {
                ConversationPostTurnAction::QueueAutoPrompt(queued_prompt) => {
                    // Queueing records the pending loop in visible history before
                    // emitting QueueAutoPrompt. The effect will re-enter this
                    // reducer as SubmitPrompt with PromptOrigin::AutoFollow.
                    let QueuedAutoPrompt {
                        prompt,
                        completed_turn_id,
                        mode_label,
                        transcript_text,
                        handoff_task,
                    } = *queued_prompt;
                    state.clear_auto_follow_skip();
                    state.record_auto_follow_queue(&completed_turn_id);
                    state.status_text =
                        format!("turn completed / queued auto-follow with mode {mode_label}");
                    state.append_status_message(state.status_text.clone());
                    effects.push(ConversationRuntimeEffect::QueueAutoPrompt {
                        prompt,
                        completed_turn_id,
                        mode_label,
                        transcript_text,
                        handoff_task,
                    });
                }
                ConversationPostTurnAction::SkipAutoFollow { reason } => {
                    // Skips are durable status messages because they explain why
                    // the automatic loop stopped and often require operator
                    // action before the next manual prompt.
                    state.record_auto_follow_skip(reason);
                    state.status_text = reason.runtime_status(&state.auto_follow_state);
                    state.append_status_message(state.status_text.clone());
                    for alert in evaluation.operator_alerts {
                        state.extend_runtime_notices([alert.runtime_notice()]);
                        state.append_status_message(alert.transcript_banner());
                        effects.push(ConversationRuntimeEffect::DispatchOperatorAlert { alert });
                    }
                }
            }
        }
    }

    ConversationRuntimeReduction { state, effects }
}

fn prompt_origin_label(origin: &PromptOrigin) -> &'static str {
    match origin {
        PromptOrigin::Manual => "manual",
        PromptOrigin::ManualIntake(_) => "manual_intake",
        PromptOrigin::AutoFollow(_) => "auto_follow",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::{AutoFollowSubmitContext, PromptOrigin};

    #[test]
    fn auto_follow_turn_completion_advances_done_progress() {
        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.thread_id = "thread-1".to_string();

        let reduction = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "continue queue".to_string(),
                transcript_text: "continue queue".to_string(),
                origin: PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
                    completed_turn_id: "turn-root".to_string(),
                    mode_label: "planning queue".to_string(),
                    transcript_text: "continue queue".to_string(),
                    debug_detail: None,
                    handoff_task: None,
                })),
            },
        );
        assert_eq!(reduction.state.auto_follow_state.progress_label(), "0/20");

        let reduction = reduce_conversation_runtime(
            reduction.state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnStarted {
                turn_id: "turn-auto-1".to_string(),
            }),
        );
        assert!(
            reduction.state.auto_follow_state.has_live_activity(),
            "auto turn should be live after provider start"
        );

        let reduction = reduce_conversation_runtime(
            reduction.state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-auto-1".to_string(),
                changed_planning_file_paths: Vec::new(),
            }),
        );

        assert_eq!(reduction.state.auto_follow_state.progress_label(), "1/20");
        assert!(
            !reduction.state.auto_follow_state.has_live_activity(),
            "completed auto turn should not leave a stale running phase"
        );
    }

    #[test]
    fn drained_planning_queue_skip_emits_operator_alert() {
        let state = ConversationViewModel::new_draft("/tmp/workspace".to_string());

        let reduction = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::PostTurnEvaluated {
                evaluation: Box::new(ConversationPostTurnEvaluation {
                    runtime_snapshot: PlanningRuntimeSnapshot::ready_with_details(
                        "Planning Context".to_string(),
                        "queue idle: no executable planning task".to_string(),
                        None,
                        None,
                    ),
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: ConversationPostTurnAction::SkipAutoFollow {
                        reason: AutoFollowSkipReason::PlanningQueueDrained,
                    },
                    parallel_queue_signal: None,
                    operator_alerts: vec![OperatorAlert::planning_queue_drained()],
                }),
            },
        );

        assert!(
            reduction
                .state
                .status_text
                .contains("all planning tasks complete")
        );
        assert!(
            reduction
                .state
                .messages
                .iter()
                .any(|message| message.text.contains("ALL PLANNING TASKS COMPLETE"))
        );
        assert!(
            reduction
                .state
                .runtime_notices
                .iter()
                .any(|notice| notice.contains("All planning tasks complete"))
        );
        assert!(reduction.effects.iter().any(|effect| matches!(
            effect,
            ConversationRuntimeEffect::DispatchOperatorAlert { alert }
                if alert.audible && alert.title == "All planning tasks complete"
        )));
    }
}
