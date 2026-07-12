/*
 * conversation_runtime.rs is the reducer/effect boundary for the live TUI
 * conversation. App-server streaming, post-turn evaluation checks, and auto-follow
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
use crate::application::service::planning::{
    PlanningRuntimeProjection, PlanningTaskHandoff, PlanningTurnExecutionSnapshotCapture,
};
use crate::core::app::{TurnStreamProgressiveActivityUpdate, TurnStreamSnapshot, TurnStreamUpdate};
use crate::diagnostics::event_log;
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalResolution, ConversationApprovalReview,
    ConversationMessage, ConversationMessageKind,
};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeEnvelopeObservation, ConversationRuntimeObservedValue,
    ConversationRuntimeThreadStatus,
};
use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;
use serde_json::json;
#[derive(Debug, Clone)]
pub(super) enum ConversationRuntimeEvent {
    /*
     * Runtime events are facts that already happened at the TUI boundary. Manual
     * and auto-follow submissions enter through SubmitPrompt, core stream
     * snapshots enter through StreamSnapshotApplied, and post-turn evaluation
     * completions return through PostTurnEvaluationCompleted.
     */
    SubmitPrompt {
        prompt: String,
        transcript_text: String,
        origin: PromptOrigin,
    },
    StreamSnapshotApplied(Box<TurnStreamSnapshot>),
    ApprovalDecisionSubmitted {
        approval_id: String,
        decision: ConversationApprovalDecision,
    },
    ApprovalDecisionSubmissionFailed {
        approval_id: String,
        error: String,
    },
    RuntimeNoticeObserved {
        notice: String,
    },
    PostTurnEvaluationCompleted {
        evaluation: Box<PostTurnEvaluationOutcome>,
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
        prompt_origin: PromptOrigin,
    },
    EvaluatePostTurn {
        workspace_directory: String,
        completed_turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: Option<PlanningTurnExecutionSnapshotCapture>,
    },
    QueueAutoPrompt {
        prompt: String,
        completed_turn_id: String,
        mode_label: String,
        transcript_text: String,
        handoff_task: Option<PlanningTaskHandoff>,
    },
    PersistApprovalReview {
        workspace_directory: String,
        thread_id: String,
        review: ConversationApprovalReview,
    },
    ResolveApprovalRequest {
        approval_id: String,
        decision: ConversationApprovalDecision,
    },
    ShowApprovalOverlay,
    CloseApprovalOverlay,
    // A Ctrl-C can arrive while turn/start is still in flight. Once TurnStarted
    // arrives, resend the sticky request so the app-server stream observes a
    // generation newer than the one sampled before turn/start.
    ResendPendingInterrupt,
    DispatchOperatorAlert {
        alert: OperatorAlert,
    },
}
#[derive(Debug, Clone)]
pub(super) struct PostTurnEvaluationOutcome {
    // Provenance binds every post-turn decision to the completed turn and
    // optional handoff signals that downstream continuation routing may consume.
    pub provenance: PostTurnEvaluationProvenance,
    // Fresh planning projection after the just-finished turn. It replaces the
    // embedded conversation snapshot before auto-follow copy is derived.
    pub runtime_projection: PlanningRuntimeProjection,
    // Repair state is presentation state, but it is decided by post-turn
    // execution where planning files and runtime diagnostics are inspected.
    pub planning_repair_state: Option<PlanningRepairState>,
    // Runtime notices are appended after the turn so footer/status panels can
    // surface planning repairs, skipped work, or provider execution details.
    pub runtime_notices: Vec<String>,
    // The post-turn policy either schedules the next internal prompt or records
    // the reason the loop stopped.
    pub action: PostTurnContinuationAction,
    // Operator alerts are explicit post-turn outputs, not inferred by the reducer
    // from auto-follow skip copy.
    pub operator_alerts: Vec<OperatorAlert>,
}
#[derive(Debug, Clone)]
pub(super) struct PostTurnEvaluationProvenance {
    pub completed_turn_id: String,
    pub handoff_task: Option<PlanningTaskHandoff>,
    pub parallel_queue_signal: Option<ParallelModePostTurnQueueSignal>,
}
impl PostTurnEvaluationProvenance {
    pub(super) fn new(completed_turn_id: String) -> Self {
        Self {
            completed_turn_id,
            handoff_task: None,
            parallel_queue_signal: None,
        }
    }

    pub(super) fn with_handoff_task(mut self, handoff_task: Option<PlanningTaskHandoff>) -> Self {
        self.handoff_task = handoff_task;
        self
    }

    pub(super) fn with_parallel_queue_signal(
        mut self,
        parallel_queue_signal: Option<ParallelModePostTurnQueueSignal>,
    ) -> Self {
        self.parallel_queue_signal = parallel_queue_signal;
        self
    }
}
#[derive(Debug, Clone)]
pub(super) struct PostTurnQueuedPrompt {
    // Prompt sent to app-server. It can include planning context not meant to be
    // shown verbatim in transcript.
    pub prompt: String,
    // Human label for the auto-follow policy that produced this prompt.
    pub mode_label: String,
    // Transcript marker shown to the operator; deliberately distinct from the
    // executable prompt text above.
    pub transcript_text: String,
}
#[derive(Debug, Clone)]
pub(super) enum PostTurnContinuationAction {
    // Box keeps the enum small because queued prompts carry several strings and
    // optional handoff identity through the background-message channel.
    QueueAutoPrompt(Box<PostTurnQueuedPrompt>),
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
            let auto_follow_blocked = matches!(origin, PromptOrigin::AutoFollow(_))
                && !state.auto_follow_state.can_queue_next();
            if prompt.is_empty() || !state.can_accept_runtime_prompt() || auto_follow_blocked {
                // Empty prompts and prompts sent while the runtime is not ready
                // or while auto-follow is disarmed are ignored rather than turned
                // into provider calls. This is the final defense for delayed
                // QueueAutoPrompt effects after `:turns off` or `:stop`.
                event_log::emit_lazy("prompt_submission_ignored", || {
                    json!({
                        "origin": prompt_origin_label(&origin),
                        "reason": if prompt.is_empty() {
                            "empty_prompt"
                        } else if auto_follow_blocked {
                            "auto_follow_disarmed"
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
            state.status_text = match &origin {
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
                prompt_origin: origin,
            });
        }
        ConversationRuntimeEvent::StreamSnapshotApplied(snapshot) => {
            let applied = take_stream_snapshot_update(&mut state, snapshot);
            let progressive_activity = applied.progressive_activity;
            match applied.update {
                TurnStreamUpdate::AttachmentObserved { profile } => {
                    // Attachment information is a runtime notice, not a transcript
                    // row, because it describes bridge recovery rather than model
                    // conversation content.
                    state.extend_runtime_notices([attachment_runtime_notice(profile)]);
                }
                TurnStreamUpdate::ThreadPrepared {
                    thread_id,
                    title,
                    cwd,
                    status_text: _,
                } => {
                    // Thread preparation binds provider identity and cwd to the
                    // conversation before turn events start appending transcript.
                    state.record_thread_prepared(thread_id, title, cwd);
                }
                TurnStreamUpdate::TurnStarted {
                    turn_id,
                    status_text: _,
                } => {
                    // Turn id is later used by TurnCompleted and auto-follow
                    // provenance, so it is recorded as soon as the provider reports
                    // start.
                    let resend_pending_interrupt = state.interrupt_request_pending;
                    state.record_turn_started(turn_id);
                    if resend_pending_interrupt {
                        effects.push(ConversationRuntimeEffect::ResendPendingInterrupt);
                    }
                }
                TurnStreamUpdate::RuntimeEnvelopeObserved {
                    observation,
                    rejection,
                } => {
                    if let Some(rejection) = rejection {
                        state.extend_runtime_notices([format!(
                            "ignored runtime envelope observation: {}",
                            rejection.notice_label()
                        )]);
                    } else if let ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                        status,
                        ..
                    } = observation.as_ref()
                    {
                        state.status_text = format!(
                            "thread status: {}",
                            conversation_runtime_thread_status_label(status)
                        );
                    }
                }
                TurnStreamUpdate::ItemLifecycleObserved {
                    observation,
                    consistency,
                    rejection,
                } => {
                    if let Some(rejection) = rejection {
                        state.extend_runtime_notices([format!(
                            "ignored item lifecycle observation: {}",
                            rejection.notice_label()
                        )]);
                    } else {
                        let lifecycle_anomaly = matches!(
                            consistency,
                            Some(
                                crate::domain::conversation_item_lifecycle::ConversationItemLifecycleConsistency::DuplicateStart
                                    | crate::domain::conversation_item_lifecycle::ConversationItemLifecycleConsistency::DuplicateCompletion
                                    | crate::domain::conversation_item_lifecycle::ConversationItemLifecycleConsistency::StartAfterCompletion
                                    | crate::domain::conversation_item_lifecycle::ConversationItemLifecycleConsistency::KindMismatch
                                    | crate::domain::conversation_item_lifecycle::ConversationItemLifecycleConsistency::TimestampRegression
                            )
                        );
                        state
                            .progressive_activity
                            .observe_item_lifecycle(&observation, consistency);
                        if lifecycle_anomaly {
                            state.extend_runtime_notices([format!(
                                "app-server item lifecycle anomaly for {}",
                                observation.item_id
                            )]);
                        } else {
                            if let crate::domain::conversation_item_lifecycle::ConversationItemKind::Unknown(
                                wire_type,
                            ) = &observation.kind
                            {
                                state.extend_runtime_notices([format!(
                                    "app-server reported an unclassified item kind: {wire_type}"
                                )]);
                            }
                        }
                    }
                }
                TurnStreamUpdate::ProgressiveActivityObserved {
                    activity,
                    rejection,
                } => {
                    if let Some(rejection) = rejection {
                        state.extend_runtime_notices([format!(
                            "ignored progressive activity observation: {}",
                            rejection.notice_label()
                        )]);
                    } else {
                        state.progressive_activity.apply_projection_update(
                            progressive_activity.as_ref(),
                            activity.first_sequence,
                            activity.last_sequence,
                            activity.payload_truncation_count,
                            activity.dropped_observation_count,
                            activity.invalid_observation_count,
                            activity.unknown_observation_count,
                        );
                        if let Some((item_id, phase, text)) =
                            latest_agent_draft_for_update(progressive_activity.as_ref(), &activity)
                        {
                            state.sync_live_agent_draft(item_id, phase, text);
                        }
                        if progressive_activity_requires_runtime_notice(&activity) {
                            state.extend_runtime_notices([format!(
                                "progressive activity was bounded (superseded={}, payload_truncated={}, dropped={}, invalid={}, unknown={})",
                                activity.superseded_publication_count,
                                activity.payload_truncation_count,
                                activity.dropped_observation_count,
                                activity.invalid_observation_count,
                                activity.unknown_observation_count,
                            )]);
                        }
                    }
                }
                TurnStreamUpdate::StatusUpdated { text } => {
                    // Provider status copy owns the main status line while a turn is
                    // active, but it does not become durable transcript history.
                    state.status_text = text;
                }
                TurnStreamUpdate::AgentMessageCompleted {
                    item_id,
                    phase,
                    text,
                } => {
                    // Completion either flushes the live buffer or patches the final
                    // transcript row for the provider item.
                    state.complete_live_agent_message(item_id, phase, text);
                }
                TurnStreamUpdate::ToolActivity { activity } => {
                    // Tool activity feeds both compact live counters and ordered
                    // transcript notices so shell tail and transcript agree.
                    state.turn_activity.register_tool_activity(&activity);
                    state.buffer_tool_message(activity.text);
                }
                TurnStreamUpdate::ApprovalReviewUpdated { review } => {
                    // Some provider statuses require approval outside the visible
                    // shell. Add a runtime notice before updating the stored review
                    // so footer/status panes can explain the handoff.
                    if let Some(notice) = approval_review_manual_client_action_notice(
                        &review,
                        state.turn_control_truth().approval,
                    ) {
                        state.extend_runtime_notices([notice]);
                    }
                    if state.has_active_thread() {
                        effects.push(ConversationRuntimeEffect::PersistApprovalReview {
                            workspace_directory: state
                                .active_turn_workspace_directory
                                .clone()
                                .unwrap_or_else(|| {
                                    state.planning_workspace_directory().to_string()
                                }),
                            thread_id: state.thread_id.clone(),
                            review: review.clone(),
                        });
                    }
                    state.update_approval_review(review);
                }
                TurnStreamUpdate::ApprovalRequested { request } => {
                    state.status_text =
                        "approval required / Y to accept / N or Esc to decline".to_string();
                    state.set_pending_approval_request(request);
                    effects.push(ConversationRuntimeEffect::ShowApprovalOverlay);
                }
                TurnStreamUpdate::ApprovalResolved {
                    approval_id,
                    resolution,
                } => {
                    let resolves_current_request = state
                        .pending_approval_request
                        .as_ref()
                        .is_some_and(|request| request.approval_id == approval_id);
                    if resolves_current_request {
                        state.clear_pending_approval_request(&approval_id);
                        state.status_text = approval_resolution_status(resolution).to_string();
                        effects.push(ConversationRuntimeEffect::CloseApprovalOverlay);
                    }
                }
                TurnStreamUpdate::TurnInterruptRequestFailed { message } => {
                    state.clear_interrupt_request();
                    state.status_text = message;
                }
                TurnStreamUpdate::TurnRetrying {
                    error,
                    correlation_failure,
                    status_text,
                    ..
                } => {
                    if correlation_failure.is_none() {
                        state.clear_interrupt_request();
                        state.status_text = status_text;
                        state.extend_runtime_notices([format!(
                            "app-server retrying active turn: {}",
                            error.summary()
                        )]);
                    } else {
                        state.extend_runtime_notices([format!(
                            "ignored retry notification outside the active turn: {}",
                            error.summary()
                        )]);
                    }
                }
                TurnStreamUpdate::TurnCompleted {
                    turn_id,
                    changed_planning_file_paths,
                    execution_snapshot_capture,
                    status_text: _,
                } => {
                    // Turn completion closes the provider stream but does not decide
                    // whether to auto-follow. That policy needs fresh planning state,
                    // so it is emitted as an effect after the model enters evaluating
                    // state.
                    let approval_was_pending = state.pending_approval_request.is_some();
                    queue_post_turn_evaluation(
                        &mut state,
                        &mut effects,
                        turn_id,
                        changed_planning_file_paths,
                        execution_snapshot_capture,
                    );
                    if approval_was_pending {
                        effects.push(ConversationRuntimeEffect::CloseApprovalOverlay);
                    }
                }
                TurnStreamUpdate::TurnTerminal {
                    receipt,
                    status_text,
                    ..
                } => {
                    let approval_was_pending = state.pending_approval_request.is_some();
                    state.fail_turn(receipt.status_error_summary());
                    state.status_text = status_text;
                    if approval_was_pending {
                        effects.push(ConversationRuntimeEffect::CloseApprovalOverlay);
                    }
                }
                TurnStreamUpdate::TurnTerminalIgnored { receipt, reason } => {
                    state.extend_runtime_notices([format!(
                        "ignored terminal receipt for {}: {reason:?}",
                        receipt.turn_id
                    )]);
                }
                TurnStreamUpdate::Failed {
                    message,
                    status_text: _,
                } => {
                    // Failure ends the active turn locally. No post-turn evaluation
                    // is scheduled because planning side effects may be incomplete.
                    let approval_was_pending = state.pending_approval_request.is_some();
                    state.fail_turn(message);
                    if approval_was_pending {
                        effects.push(ConversationRuntimeEffect::CloseApprovalOverlay);
                    }
                }
                TurnStreamUpdate::RuntimeFailureIgnored { message } => {
                    state.extend_runtime_notices([format!(
                        "ignored runtime failure after terminal receipt: {message}"
                    )]);
                }
                TurnStreamUpdate::RuntimeNotice { notice } => {
                    // Execution-layer notices come from effect runners, not provider
                    // stream events. They are still runtime notices so the user can see
                    // background execution failures in the same place.
                    state.extend_runtime_notices([notice]);
                }
            }
        }
        ConversationRuntimeEvent::ApprovalDecisionSubmitted {
            approval_id,
            decision,
        } => {
            if state.mark_approval_decision_submitted(&approval_id, decision) {
                state.status_text = format!(
                    "approval decision submitted: {} / waiting for runtime resolution",
                    approval_decision_label(decision)
                );
                effects.push(ConversationRuntimeEffect::ResolveApprovalRequest {
                    approval_id,
                    decision,
                });
            }
        }
        ConversationRuntimeEvent::ApprovalDecisionSubmissionFailed { approval_id, error } => {
            if state.clear_pending_approval_resolution(&approval_id) {
                state.status_text =
                    format!("approval decision failed: {error} / retry accept or decline");
            }
        }
        ConversationRuntimeEvent::RuntimeNoticeObserved { notice } => {
            state.extend_runtime_notices([notice]);
        }
        ConversationRuntimeEvent::PostTurnEvaluationCompleted { evaluation } => {
            let PostTurnEvaluationOutcome {
                provenance,
                runtime_projection,
                planning_repair_state,
                runtime_notices,
                action,
                operator_alerts,
            } = *evaluation;
            // Apply the new planning view before acting on the decision; queued
            // or skipped auto-follow copy should describe the latest queue state.
            state.replace_reducer_event_projection_cache(runtime_projection);
            state.planning_repair_state = planning_repair_state;
            state.extend_runtime_notices(runtime_notices);
            match action {
                PostTurnContinuationAction::QueueAutoPrompt(queued_prompt) => {
                    let parallel_dispatch_queued = matches!(
                        provenance.parallel_queue_signal,
                        Some(ParallelModePostTurnQueueSignal::AutoFollowQueued)
                    );
                    let parallel_dispatch_allowed = parallel_dispatch_queued
                        && state
                            .auto_follow_state
                            .parallel_post_turn_continuation_allowed();
                    if !state.auto_follow_state.can_queue_next() && !parallel_dispatch_allowed {
                        let reason = if state.auto_follow_state.post_turn_continuation_paused() {
                            AutoFollowSkipReason::PostTurnContinuationPaused
                        } else {
                            AutoFollowSkipReason::LimitReached
                        };
                        apply_auto_follow_skip(&mut state, &mut effects, reason, operator_alerts);
                        return ConversationRuntimeReduction { state, effects };
                    }
                    // Queueing records the pending loop in visible history before
                    // emitting QueueAutoPrompt. The effect will re-enter this
                    // reducer as SubmitPrompt with PromptOrigin::AutoFollow.
                    let PostTurnQueuedPrompt {
                        prompt,
                        mode_label,
                        transcript_text,
                    } = *queued_prompt;
                    let completed_turn_id = provenance.completed_turn_id;
                    let handoff_task = provenance.handoff_task;
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
                PostTurnContinuationAction::SkipAutoFollow { mut reason } => {
                    // Skips are durable status messages because they explain why
                    // the automatic loop stopped and often require operator
                    // action before the next manual prompt.
                    if reason == AutoFollowSkipReason::PostTurnContinuationPaused
                        && !state.auto_follow_state.post_turn_continuation_paused()
                        && !state.auto_follow_state.is_enabled()
                    {
                        // Application execution receives `continuation_paused`
                        // for both secure-default off and sticky operator stop so
                        // it can suppress hidden workers. Preserve the distinct
                        // operator-facing reason at the TUI boundary.
                        reason = AutoFollowSkipReason::LimitReached;
                    }
                    apply_auto_follow_skip(&mut state, &mut effects, reason, operator_alerts);
                }
            }
        }
    }

    ConversationRuntimeReduction { state, effects }
}

struct AppliedStreamSnapshot {
    update: TurnStreamUpdate,
    progressive_activity: std::sync::Arc<
        crate::domain::conversation_progressive_activity::ConversationProgressiveActivityProjectionSnapshot,
    >,
}

fn take_stream_snapshot_update(
    state: &mut ConversationViewModel,
    snapshot: Box<TurnStreamSnapshot>,
) -> AppliedStreamSnapshot {
    let snapshot = *snapshot;
    state.runtime_envelope = snapshot.runtime_envelope.map(|envelope| *envelope);
    AppliedStreamSnapshot {
        update: snapshot.update,
        progressive_activity: snapshot.progressive_activity,
    }
}

fn latest_agent_draft_for_update(
    projection: &crate::domain::conversation_progressive_activity::ConversationProgressiveActivityProjectionSnapshot,
    update: &TurnStreamProgressiveActivityUpdate,
) -> Option<(String, Option<String>, String)> {
    let first_sequence = update.first_sequence?;
    let last_sequence = update.last_sequence?;
    projection
        .records
        .iter()
        .filter(|record| {
            record.last_sequence() >= first_sequence && record.last_sequence() <= last_sequence
        })
        .filter(|record| {
            matches!(
                &record.observation().payload,
                crate::domain::conversation_progressive_activity::ConversationProgressiveActivityPayload::AgentMessageDelta {
                    ..
                }
            )
        })
        .max_by_key(|record| record.last_sequence())
        .and_then(|record| {
            let crate::domain::conversation_progressive_activity::ConversationProgressiveActivityPayload::AgentMessageDelta {
                phase,
                text,
                ..
            } = &record.observation().payload
            else {
                return None;
            };
            Some((
                record.observation().item_id.clone().unwrap_or_default(),
                phase.clone(),
                text.clone(),
            ))
        })
}

const fn progressive_activity_requires_runtime_notice(
    activity: &TurnStreamProgressiveActivityUpdate,
) -> bool {
    activity.payload_truncation_count > 0
        || activity.dropped_observation_count > 0
        || activity.invalid_observation_count > 0
        || activity.unknown_observation_count > 0
}

fn conversation_runtime_thread_status_label(
    status: &ConversationRuntimeObservedValue<ConversationRuntimeThreadStatus>,
) -> &str {
    match status {
        ConversationRuntimeObservedValue::Observed(status)
        | ConversationRuntimeObservedValue::Defaulted(status) => match status {
            ConversationRuntimeThreadStatus::NotLoaded => "notLoaded",
            ConversationRuntimeThreadStatus::Idle => "idle",
            ConversationRuntimeThreadStatus::SystemError => "systemError",
            ConversationRuntimeThreadStatus::Active { .. } => "active",
            ConversationRuntimeThreadStatus::Unknown(label) => label,
        },
        ConversationRuntimeObservedValue::Null
        | ConversationRuntimeObservedValue::Missing
        | ConversationRuntimeObservedValue::Malformed(_)
        | ConversationRuntimeObservedValue::UnavailableOnStableResponse
        | ConversationRuntimeObservedValue::UnavailableAfterObservationGap => "unknown",
    }
}

fn apply_auto_follow_skip(
    state: &mut ConversationViewModel,
    effects: &mut Vec<ConversationRuntimeEffect>,
    reason: AutoFollowSkipReason,
    operator_alerts: Vec<OperatorAlert>,
) {
    state.record_auto_follow_skip(reason);
    state.status_text = reason.runtime_status(&state.auto_follow_state);
    state.append_status_message(state.status_text.clone());
    for alert in operator_alerts {
        state.extend_runtime_notices([alert.runtime_notice()]);
        state.append_status_message(alert.transcript_banner());
        effects.push(ConversationRuntimeEffect::DispatchOperatorAlert { alert });
    }
}

fn approval_resolution_status(resolution: ConversationApprovalResolution) -> &'static str {
    match resolution {
        ConversationApprovalResolution::Accepted => "approval accepted for this request",
        ConversationApprovalResolution::Declined => "approval declined",
        ConversationApprovalResolution::TimedOut => "approval timed out and was declined",
        ConversationApprovalResolution::Interrupted => {
            "approval declined because the turn was interrupted"
        }
        ConversationApprovalResolution::Disconnected => {
            "approval declined because the runtime disconnected"
        }
        ConversationApprovalResolution::InvalidRequest => {
            "approval request was invalid and was declined"
        }
    }
}

fn approval_decision_label(decision: ConversationApprovalDecision) -> &'static str {
    match decision {
        ConversationApprovalDecision::Accept => "accept",
        ConversationApprovalDecision::Decline => "decline",
    }
}

pub(super) fn conversation_runtime_auto_prompt_queued(
    effects: &[ConversationRuntimeEffect],
) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. }))
}

pub(super) fn suppress_conversation_runtime_auto_prompt(
    effects: &mut Vec<ConversationRuntimeEffect>,
) {
    effects.retain(|effect| !matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. }));
}

fn prompt_origin_label(origin: &PromptOrigin) -> &'static str {
    match origin {
        PromptOrigin::Manual => "manual",
        PromptOrigin::ManualIntake(_) => "manual_intake",
        PromptOrigin::AutoFollow(_) => "auto_follow",
    }
}

fn queue_post_turn_evaluation(
    state: &mut ConversationViewModel,
    effects: &mut Vec<ConversationRuntimeEffect>,
    turn_id: String,
    changed_planning_file_paths: Vec<String>,
    execution_snapshot_capture: Option<PlanningTurnExecutionSnapshotCapture>,
) {
    let changed_planning_file_count = changed_planning_file_paths.len();
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
            "changed_planning_file_count": changed_planning_file_count,
        })
    });
    effects.push(ConversationRuntimeEffect::EvaluatePostTurn {
        workspace_directory,
        completed_turn_id: turn_id,
        changed_planning_file_paths,
        execution_snapshot_capture,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::app_runtime::core_turn_stream_event_from_application;
    use crate::adapter::inbound::tui::app::{
        AutoFollowSubmitContext, ManualIntakeSubmitContext, PromptOrigin,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::planning::{
        PlanningExecutionSnapshot, PlanningTurnExecutionSnapshotCapture,
    };
    use crate::core::app::TurnStreamState;
    use crate::diagnostics::trace_event_log::AKRA_EVENT_TARGET;
    use crate::domain::conversation::{
        ConversationApprovalDecision, ConversationApprovalRequest, ConversationApprovalRequestKind,
        ConversationApprovalResolution, ConversationApprovalReview,
        ConversationApprovalReviewStatus, ConversationMessage, ConversationMessageKind,
        ConversationToolActivity, ConversationToolActivityKind,
    };
    use crate::domain::conversation_item_lifecycle::{
        ConversationItemKind, ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
        ConversationItemLifecycleSource, ConversationItemOutcome,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
        ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    };
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeConfigurationObservation, ConversationRuntimeConfigurationRequest,
        ConversationRuntimeEnvelope, ConversationRuntimeLaunchEnvironment,
        ConversationRuntimeObservedValue, ConversationRuntimeRequestedValue,
        ConversationRuntimeThreadStatus,
    };
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    fn stream_snapshot_event(event: ConversationStreamEvent) -> ConversationRuntimeEvent {
        let mut stream_state = TurnStreamState::new();
        match &event {
            ConversationStreamEvent::TurnTerminal { receipt } => {
                stream_state.seed_loaded_thread_identity(
                    receipt.thread_id.clone(),
                    "Test thread",
                    "/tmp/workspace",
                );
                stream_state.apply_stream_event(crate::core::app::TurnStreamEvent::TurnStarted {
                    turn_id: receipt.turn_id.clone(),
                    runtime_request: Box::default(),
                });
            }
            ConversationStreamEvent::ProgressiveActivityObserved { batch } => {
                let observation = &batch
                    .records()
                    .first()
                    .expect("progressive test event should retain an observation")
                    .observation();
                stream_state.seed_loaded_thread_identity(
                    observation.thread_id.clone(),
                    "Test thread",
                    "/tmp/workspace",
                );
                if let Some(turn_id) = observation.turn_id.clone() {
                    stream_state.apply_stream_event(
                        crate::core::app::TurnStreamEvent::TurnStarted {
                            turn_id,
                            runtime_request: Box::default(),
                        },
                    );
                }
            }
            ConversationStreamEvent::ItemLifecycleObserved { observation } => {
                stream_state.seed_loaded_thread_identity(
                    observation.thread_id.clone(),
                    "Test thread",
                    "/tmp/workspace",
                );
                stream_state.apply_stream_event(crate::core::app::TurnStreamEvent::TurnStarted {
                    turn_id: observation.turn_id.clone(),
                    runtime_request: Box::default(),
                });
            }
            _ => {}
        }
        ConversationRuntimeEvent::StreamSnapshotApplied(Box::new(
            stream_state.apply_stream_event(core_turn_stream_event_from_application(event)),
        ))
    }

    fn progressive_agent_event(text: &str) -> ConversationStreamEvent {
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(progressive_agent_batch(0, text)),
        }
    }

    fn progressive_agent_batch(sequence: u64, text: &str) -> ConversationProgressiveActivityBatch {
        ConversationProgressiveActivityBatch::single(ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: Some("agent-1".to_string()),
            kind: ConversationProgressiveActivityKind::AgentMessageDelta,
            payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                phase: Some("analysis".to_string()),
                text: text.to_string(),
                source_bytes: text.len() as u64,
                truncated_bytes: 0,
            },
        })
        .expect("progressive agent test event should be valid")
    }

    fn progressive_command_event(tail: &str) -> ConversationStreamEvent {
        let newline_count = tail
            .as_bytes()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count() as u64;
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence: 0,
                thread_id: "thread-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("command-1".to_string()),
                kind: ConversationProgressiveActivityKind::CommandOutput,
                payload: ConversationProgressiveActivityPayload::CommandOutput {
                    tail: tail.to_string(),
                    chunk_count: 1,
                    source_bytes: tail.len() as u64,
                    newline_count,
                    ends_with_newline: tail.ends_with('\n'),
                    truncated_bytes: 0,
                },
            },
        )
        .expect("progressive command test event should be valid");
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        }
    }

    fn command_started_event() -> ConversationStreamEvent {
        ConversationStreamEvent::ItemLifecycleObserved {
            observation: Box::new(ConversationItemLifecycleObservation {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "command-1".to_string(),
                kind: ConversationItemKind::CommandExecution,
                phase: ConversationItemLifecyclePhase::Started,
                source: ConversationItemLifecycleSource::Live,
                observed_at_ms: Some(1),
                outcome: ConversationItemOutcome::InProgress,
                summary: "command running".to_string(),
            }),
        }
    }

    fn coalesced_progressive_agent_event() -> ConversationStreamEvent {
        let mut batch = progressive_agent_batch(0, "hel");
        batch
            .try_merge_from(progressive_agent_batch(1, "lo"))
            .unwrap();
        batch.record_superseded_publication().unwrap();
        ConversationStreamEvent::ProgressiveActivityObserved {
            batch: Box::new(batch),
        }
    }

    fn completed_stream_event(thread_id: &str, turn_id: &str) -> ConversationStreamEvent {
        let receipt = crate::domain::turn_terminal::ConversationTurnTerminalReceipt::completed(
            thread_id,
            turn_id,
            Vec::new(),
        )
        .with_application_delivery(
            crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
        );
        ConversationStreamEvent::TurnTerminal { receipt }
    }

    fn runtime_envelope_with_models(
        requested_model: &str,
        applied_model: &str,
    ) -> ConversationRuntimeEnvelope {
        ConversationRuntimeEnvelope::prepared(
            ConversationRuntimeConfigurationRequest {
                model: ConversationRuntimeRequestedValue::Value(requested_model.to_string()),
                ..ConversationRuntimeConfigurationRequest::default()
            },
            ConversationRuntimeConfigurationObservation {
                model: ConversationRuntimeObservedValue::Observed(applied_model.to_string()),
                cwd: ConversationRuntimeObservedValue::Observed("/tmp/workspace".to_string()),
                ..ConversationRuntimeConfigurationObservation::default()
            },
            ConversationRuntimeLaunchEnvironment::unknown(),
            ConversationRuntimeObservedValue::Observed(ConversationRuntimeThreadStatus::Idle),
        )
    }

    fn terminal_stream_event(
        outcome: crate::domain::turn_terminal::ConversationTurnTerminalOutcome,
        delivery: crate::domain::turn_terminal::ConversationTurnApplicationDelivery,
    ) -> ConversationStreamEvent {
        let receipt = crate::domain::turn_terminal::ConversationTurnTerminalReceipt::new(
            "thread-1", "turn-1", outcome,
        )
        .with_application_delivery(delivery);
        ConversationStreamEvent::TurnTerminal { receipt }
    }

    fn with_akra_event_trace<T>(body: impl FnOnce() -> T) -> T {
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new(format!("{AKRA_EVENT_TARGET}=debug")))
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink));
        tracing::subscriber::with_default(subscriber, body)
    }

    fn auto_follow_origin() -> PromptOrigin {
        PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
            completed_turn_id: "turn-root".to_string(),
            mode_label: "planning queue".to_string(),
            transcript_text: "continue queue".to_string(),
            debug_detail: None,
            handoff_task: None,
        }))
    }

    fn manual_intake_origin() -> PromptOrigin {
        PromptOrigin::ManualIntake(Box::new(ManualIntakeSubmitContext {
            transcript_text: "manual intake".to_string(),
            handoff_task: None,
            parallel_mode_enabled_at_submission: true,
        }))
    }

    #[test]
    fn ignored_prompt_edges_and_origin_labels_are_stable() {
        assert_eq!(prompt_origin_label(&PromptOrigin::Manual), "manual");
        assert_eq!(
            prompt_origin_label(&manual_intake_origin()),
            "manual_intake"
        );
        assert_eq!(prompt_origin_label(&auto_follow_origin()), "auto_follow");

        let empty_prompt = with_akra_event_trace(|| {
            reduce_conversation_runtime(
                ConversationViewModel::new_draft("/tmp/workspace".to_string()),
                ConversationRuntimeEvent::SubmitPrompt {
                    prompt: "   ".to_string(),
                    transcript_text: "ignored".to_string(),
                    origin: PromptOrigin::Manual,
                },
            )
        });
        assert!(empty_prompt.effects.is_empty());
        assert!(empty_prompt.state.messages.is_empty());

        let mut submitting_state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        submitting_state.mark_turn_submitting("/tmp/workspace".to_string());
        let blocked_runtime_prompt = with_akra_event_trace(|| {
            reduce_conversation_runtime(
                submitting_state,
                ConversationRuntimeEvent::SubmitPrompt {
                    prompt: "continue".to_string(),
                    transcript_text: "continue".to_string(),
                    origin: auto_follow_origin(),
                },
            )
        });
        assert!(blocked_runtime_prompt.effects.is_empty());
        assert!(blocked_runtime_prompt.state.messages.is_empty());

        let mut manual_blocked_state =
            ConversationViewModel::new_draft("/tmp/workspace".to_string());
        manual_blocked_state.record_auto_follow_queue("turn-root");
        let blocked_manual_prompt = with_akra_event_trace(|| {
            reduce_conversation_runtime(
                manual_blocked_state,
                ConversationRuntimeEvent::SubmitPrompt {
                    prompt: "manual".to_string(),
                    transcript_text: "manual".to_string(),
                    origin: PromptOrigin::Manual,
                },
            )
        });
        assert!(blocked_manual_prompt.effects.is_empty());
        assert!(blocked_manual_prompt.state.messages.is_empty());
    }

    #[test]
    fn submitting_phase_interrupt_is_resent_when_the_turn_id_arrives() {
        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.mark_turn_submitting("/tmp/workspace".to_string());
        assert!(state.mark_interrupt_requested_once());

        let reduction = reduce_conversation_runtime(
            state,
            stream_snapshot_event(ConversationStreamEvent::TurnStarted {
                turn_id: "turn-after-stop".to_string(),
                runtime_request: Box::default(),
            }),
        );

        assert!(reduction.state.interrupt_request_pending);
        assert!(
            reduction
                .effects
                .iter()
                .any(|effect| matches!(effect, ConversationRuntimeEffect::ResendPendingInterrupt))
        );
    }

    #[test]
    fn approval_request_decision_and_resolution_drive_modal_effects() {
        let request = ConversationApprovalRequest {
            approval_id: "approval-7".to_string(),
            server_request_id: "server-7".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "Command execution requested.".to_string(),
            details: vec!["Command: cargo test".to_string()],
        };
        let requested = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/workspace".to_string()),
            stream_snapshot_event(ConversationStreamEvent::ApprovalRequested {
                request: request.clone(),
            }),
        );
        assert_eq!(requested.state.pending_approval_request, Some(request));
        assert_eq!(
            requested.state.status_text,
            "approval required / Y to accept / N or Esc to decline"
        );
        assert!(
            requested
                .effects
                .contains(&ConversationRuntimeEffect::ShowApprovalOverlay)
        );

        let stale_resolution = reduce_conversation_runtime(
            requested.state,
            stream_snapshot_event(ConversationStreamEvent::ApprovalResolved {
                approval_id: "approval-stale".to_string(),
                resolution: ConversationApprovalResolution::Declined,
            }),
        );
        assert!(stale_resolution.state.pending_approval_request.is_some());
        assert!(
            !stale_resolution
                .effects
                .contains(&ConversationRuntimeEffect::CloseApprovalOverlay)
        );

        let submitted = reduce_conversation_runtime(
            stale_resolution.state,
            ConversationRuntimeEvent::ApprovalDecisionSubmitted {
                approval_id: "approval-7".to_string(),
                decision: ConversationApprovalDecision::Accept,
            },
        );
        assert_eq!(
            submitted.effects,
            vec![ConversationRuntimeEffect::ResolveApprovalRequest {
                approval_id: "approval-7".to_string(),
                decision: ConversationApprovalDecision::Accept,
            }]
        );
        assert!(submitted.state.pending_approval_request.is_some());
        assert_eq!(
            submitted.state.pending_approval_decision(),
            Some(ConversationApprovalDecision::Accept)
        );
        assert_eq!(
            submitted.state.status_text,
            "approval decision submitted: accept / waiting for runtime resolution"
        );

        let rapid_decline = reduce_conversation_runtime(
            submitted.state,
            ConversationRuntimeEvent::ApprovalDecisionSubmitted {
                approval_id: "approval-7".to_string(),
                decision: ConversationApprovalDecision::Decline,
            },
        );
        assert!(rapid_decline.effects.is_empty());
        assert_eq!(
            rapid_decline.state.pending_approval_decision(),
            Some(ConversationApprovalDecision::Accept)
        );
        assert!(rapid_decline.state.pending_approval_request.is_some());

        let resolved = reduce_conversation_runtime(
            rapid_decline.state,
            stream_snapshot_event(ConversationStreamEvent::ApprovalResolved {
                approval_id: "approval-7".to_string(),
                resolution: ConversationApprovalResolution::Accepted,
            }),
        );
        assert!(resolved.state.pending_approval_request.is_none());
        assert_eq!(resolved.state.pending_approval_decision(), None);
        assert!(
            resolved
                .effects
                .contains(&ConversationRuntimeEffect::CloseApprovalOverlay)
        );
        assert_eq!(
            resolved.state.status_text,
            "approval accepted for this request"
        );
    }

    #[test]
    fn failed_approval_submission_reopens_the_current_modal_for_retry() {
        let request = ConversationApprovalRequest {
            approval_id: "approval-retry".to_string(),
            server_request_id: "server-retry".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "Command execution requested.".to_string(),
            details: vec!["Command: cargo test".to_string()],
        };
        let requested = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/workspace".to_string()),
            stream_snapshot_event(ConversationStreamEvent::ApprovalRequested { request }),
        );
        let submitted = reduce_conversation_runtime(
            requested.state,
            ConversationRuntimeEvent::ApprovalDecisionSubmitted {
                approval_id: "approval-retry".to_string(),
                decision: ConversationApprovalDecision::Accept,
            },
        );

        let failed = reduce_conversation_runtime(
            submitted.state,
            ConversationRuntimeEvent::ApprovalDecisionSubmissionFailed {
                approval_id: "approval-retry".to_string(),
                error: "runtime unavailable".to_string(),
            },
        );
        assert!(failed.state.pending_approval_request.is_some());
        assert_eq!(failed.state.pending_approval_decision(), None);
        assert_eq!(
            failed.state.status_text,
            "approval decision failed: runtime unavailable / retry accept or decline"
        );

        let retried = reduce_conversation_runtime(
            failed.state,
            ConversationRuntimeEvent::ApprovalDecisionSubmitted {
                approval_id: "approval-retry".to_string(),
                decision: ConversationApprovalDecision::Decline,
            },
        );
        assert_eq!(
            retried.effects,
            vec![ConversationRuntimeEffect::ResolveApprovalRequest {
                approval_id: "approval-retry".to_string(),
                decision: ConversationApprovalDecision::Decline,
            }]
        );
    }

    #[test]
    fn terminal_interrupt_failure_reopens_stop_request_gate() {
        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.mark_turn_submitting("/tmp/workspace".to_string());
        assert!(state.mark_interrupt_requested_once());

        let mut reduction = reduce_conversation_runtime(
            state,
            stream_snapshot_event(ConversationStreamEvent::TurnInterruptRequestFailed {
                message: "interrupt retries exhausted".to_string(),
            }),
        );

        assert!(!reduction.state.interrupt_request_pending);
        assert_eq!(reduction.state.status_text, "interrupt retries exhausted");
        assert!(reduction.state.mark_interrupt_requested_once());
    }

    #[test]
    fn stream_snapshot_updates_cover_runtime_notices_messages_and_failures() {
        let mut reduction = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/workspace".to_string()),
            stream_snapshot_event(ConversationStreamEvent::codex_app_server_launch_attachment()),
        );
        assert!(
            reduction
                .state
                .runtime_notices
                .iter()
                .any(|notice| notice.contains("provider-launched"))
        );

        reduction = reduce_conversation_runtime(
            reduction.state,
            stream_snapshot_event(ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Runtime thread".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            }),
        );
        assert_eq!(reduction.state.thread_id, "thread-1");
        assert_eq!(reduction.state.title, "Runtime thread");
        assert_eq!(reduction.state.cwd, "/tmp/workspace");

        reduction = reduce_conversation_runtime(
            reduction.state,
            stream_snapshot_event(progressive_agent_event("hel")),
        );
        assert_eq!(
            reduction
                .state
                .live_agent_message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("hel")
        );

        reduction = reduce_conversation_runtime(
            reduction.state,
            stream_snapshot_event(ConversationStreamEvent::AgentMessageCompleted {
                item_id: "agent-1".to_string(),
                phase: Some("final".to_string()),
                text: "hello final".to_string(),
            }),
        );
        assert!(reduction.state.live_agent_message.is_none());
        assert!(reduction.state.messages.iter().any(|message| {
            message.kind == ConversationMessageKind::Agent && message.text == "hello final"
        }));

        reduction = reduce_conversation_runtime(
            reduction.state,
            stream_snapshot_event(ConversationStreamEvent::ToolActivity {
                activity: ConversationToolActivity {
                    kind: ConversationToolActivityKind::CommandExecution,
                    text: "cargo test".to_string(),
                    file_change_count: 0,
                },
            }),
        );
        assert!(
            reduction
                .state
                .buffered_tool_messages
                .iter()
                .any(|message| {
                    message.kind == ConversationMessageKind::Tool && message.text == "cargo test"
                })
        );

        reduction = reduce_conversation_runtime(
            reduction.state,
            stream_snapshot_event(ConversationStreamEvent::ApprovalReviewUpdated {
                review: ConversationApprovalReview {
                    target_item_id: "tool-1".to_string(),
                    status: ConversationApprovalReviewStatus::Unknown(
                        "human_review_requested".to_string(),
                    ),
                    risk_level: Some("medium".to_string()),
                    rationale: Some("needs review".to_string()),
                },
            }),
        );
        assert!(
            reduction
                .state
                .runtime_notices
                .iter()
                .any(|notice| notice.contains("approval requires manual review"))
        );
        assert_eq!(
            reduction
                .state
                .approval_review
                .as_ref()
                .map(|review| review.target_item_id.as_str()),
            Some("tool-1")
        );
        assert!(reduction.effects.iter().any(|effect| matches!(
            effect,
            ConversationRuntimeEffect::PersistApprovalReview {
                workspace_directory,
                thread_id,
                review,
            } if workspace_directory == "/tmp/workspace"
                && thread_id == "thread-1"
                && review.target_item_id == "tool-1"
        )));

        reduction = reduce_conversation_runtime(
            reduction.state,
            stream_snapshot_event(ConversationStreamEvent::Failed {
                message: "provider failed".to_string(),
            }),
        );
        assert_eq!(reduction.state.status_text, "turn failed");
        assert!(reduction.state.messages.iter().any(|message| {
            message.kind == ConversationMessageKind::Tool && message.text == "cargo test"
        }));
        assert!(
            reduction
                .state
                .messages
                .iter()
                .any(|message| message.text == "provider failed")
        );
    }

    #[test]
    fn normal_progressive_coalescing_updates_live_draft_without_a_loss_notice() {
        let reduction = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/workspace".to_string()),
            stream_snapshot_event(coalesced_progressive_agent_event()),
        );

        assert_eq!(
            reduction
                .state
                .live_agent_message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("hello")
        );
        assert!(
            reduction
                .state
                .runtime_notices
                .iter()
                .all(|notice| !notice.contains("progressive activity was bounded"))
        );
        assert!(!reduction.state.progressive_activity.bounded_history());
    }

    #[test]
    fn progressive_command_projects_only_counts_and_clears_on_failure() {
        let secret = "AKRA_RAIL_RAW_COMMAND_SECRET";
        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.record_thread_prepared(
            "thread-1".to_string(),
            "Runtime thread".to_string(),
            "/tmp/workspace".to_string(),
        );
        state.mark_turn_submitting("/tmp/workspace".to_string());
        state.record_turn_started("turn-1".to_string());

        let started =
            reduce_conversation_runtime(state, stream_snapshot_event(command_started_event()));
        let projected = reduce_conversation_runtime(
            started.state,
            stream_snapshot_event(progressive_command_event(&format!("first line\n{secret}"))),
        );

        assert_eq!(projected.state.progressive_activity.command_line_count(), 2);
        assert!(!format!("{:?}", projected.state.progressive_activity).contains(secret));

        let failed = reduce_conversation_runtime(
            projected.state,
            stream_snapshot_event(ConversationStreamEvent::Failed {
                message: "provider failed".to_string(),
            }),
        );
        assert_eq!(failed.state.progressive_activity.command_line_count(), 0);
        assert_eq!(failed.state.progressive_activity.active_item_kind(), None);
    }

    #[test]
    fn same_thread_reattach_replaces_envelope_without_duplicate_open_status() {
        let mut reduction = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/workspace".to_string()),
            stream_snapshot_event(ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Runtime thread".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::new(runtime_envelope_with_models(
                    "requested-a",
                    "applied-a",
                )),
            }),
        );
        let opened_status_count = reduction
            .state
            .messages
            .iter()
            .filter(|message| {
                message.kind == ConversationMessageKind::Status
                    && message.text == "thread opened / Runtime thread"
            })
            .count();
        assert_eq!(opened_status_count, 1);

        reduction = reduce_conversation_runtime(
            reduction.state,
            stream_snapshot_event(ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Runtime thread renamed".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::new(runtime_envelope_with_models(
                    "requested-b",
                    "applied-b",
                )),
            }),
        );

        assert_eq!(reduction.state.title, "Runtime thread renamed");
        assert_eq!(
            reduction
                .state
                .runtime_envelope
                .as_ref()
                .map(|envelope| &envelope.applied.model),
            Some(&ConversationRuntimeObservedValue::Observed(
                "applied-b".to_string()
            ))
        );
        assert_eq!(
            reduction
                .state
                .messages
                .iter()
                .filter(|message| {
                    message.kind == ConversationMessageKind::Status
                        && message.text.starts_with("thread opened / ")
                })
                .count(),
            1
        );
    }

    #[test]
    fn accepted_typed_thread_status_keeps_existing_tui_status_copy() {
        let mut stream_state = TurnStreamState::new();
        stream_state.apply_stream_event(crate::core::app::TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-1".to_string(),
            title: "Runtime thread".to_string(),
            cwd: "/tmp/workspace".to_string(),
            runtime_envelope: Box::new(runtime_envelope_with_models(
                "requested-model",
                "applied-model",
            )),
        });
        stream_state.apply_stream_event(crate::core::app::TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
            runtime_request: Box::default(),
        });
        let snapshot = stream_state.apply_stream_event(
            crate::core::app::TurnStreamEvent::RuntimeEnvelopeObserved {
                observation: Box::new(
                    ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                        thread_id: "thread-1".to_string(),
                        status: ConversationRuntimeObservedValue::Observed(
                            ConversationRuntimeThreadStatus::Active {
                                waiting_on_approval: true,
                                waiting_on_user_input: false,
                                unknown_flags: Vec::new(),
                                unknown_flags_truncated: false,
                            },
                        ),
                    },
                ),
            },
        );

        let reduction = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/workspace".to_string()),
            ConversationRuntimeEvent::StreamSnapshotApplied(Box::new(snapshot)),
        );

        assert_eq!(reduction.state.status_text, "thread status: active");
    }

    #[test]
    fn auto_follow_turn_completion_advances_done_progress() {
        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.thread_id = "thread-1".to_string();
        state.auto_follow_state.set_max_auto_turns(20);

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
            stream_snapshot_event(ConversationStreamEvent::TurnStarted {
                turn_id: "turn-auto-1".to_string(),
                runtime_request: Box::default(),
            }),
        );
        assert!(
            reduction.state.auto_follow_state.has_live_activity(),
            "auto turn should be live after provider start"
        );

        let reduction = reduce_conversation_runtime(
            reduction.state,
            stream_snapshot_event(completed_stream_event("thread-1", "turn-auto-1")),
        );

        assert_eq!(reduction.state.auto_follow_state.progress_label(), "1/20");
        assert!(
            reduction.state.auto_follow_state.has_live_activity(),
            "completed auto turn must hold manual intake until post-turn evaluation settles"
        );
        assert!(!reduction.state.can_accept_manual_prompt());
    }

    #[test]
    fn only_confirmed_completed_terminal_queues_post_turn_evaluation() {
        use crate::domain::turn_terminal::{
            ConversationTurnApplicationDelivery, ConversationTurnApplicationDeliveryFailure,
            ConversationTurnError, ConversationTurnTerminalOutcome,
            ConversationTurnTerminalUncertainty,
        };

        let terminal_events = [
            terminal_stream_event(
                ConversationTurnTerminalOutcome::Interrupted,
                ConversationTurnApplicationDelivery::Confirmed,
            ),
            terminal_stream_event(
                ConversationTurnTerminalOutcome::Failed {
                    error: ConversationTurnError::new("provider failed", None::<&str>, None),
                },
                ConversationTurnApplicationDelivery::Confirmed,
            ),
            terminal_stream_event(
                ConversationTurnTerminalOutcome::Unknown {
                    reason: ConversationTurnTerminalUncertainty::NonRetryErrorGraceExpired,
                    observed_error: None,
                },
                ConversationTurnApplicationDelivery::Confirmed,
            ),
            terminal_stream_event(
                ConversationTurnTerminalOutcome::Completed,
                ConversationTurnApplicationDelivery::Unconfirmed(
                    ConversationTurnApplicationDeliveryFailure::Disconnected,
                ),
            ),
        ];

        for event in terminal_events {
            let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
            state.thread_id = "thread-1".to_string();
            let reduction = reduce_conversation_runtime(state, stream_snapshot_event(event));

            assert!(reduction.effects.iter().all(|effect| !matches!(
                effect,
                ConversationRuntimeEffect::EvaluatePostTurn { .. }
            )));
            assert!(reduction.state.can_accept_manual_prompt());
        }

        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.thread_id = "thread-1".to_string();
        let recovery_pending = reduce_conversation_runtime(
            state,
            stream_snapshot_event(terminal_stream_event(
                ConversationTurnTerminalOutcome::Completed,
                ConversationTurnApplicationDelivery::Unconfirmed(
                    ConversationTurnApplicationDeliveryFailure::Disconnected,
                ),
            )),
        );

        assert_eq!(recovery_pending.state.status_text, "turn recovery pending");
        assert!(
            recovery_pending
                .state
                .messages
                .iter()
                .any(|message| message.kind == ConversationMessageKind::Status
                    && message.text.contains("recovery pending")
                    && message.text.contains("application delivery unconfirmed"))
        );
    }

    #[test]
    fn completed_turn_with_auto_follow_off_blocks_manual_input_until_evaluation_settles() {
        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.thread_id = "thread-1".to_string();
        state.auto_follow_state.set_max_auto_turns(0);
        state.messages.extend([
            ConversationMessage::new(ConversationMessageKind::User, "operator task", None, None),
            ConversationMessage::new(
                ConversationMessageKind::Agent,
                "parallel worker result",
                None,
                None,
            ),
        ]);

        let reduction = reduce_conversation_runtime(
            state,
            stream_snapshot_event(completed_stream_event("thread-1", "turn-1")),
        );

        assert!(reduction.effects.iter().any(|effect| matches!(
            effect,
            ConversationRuntimeEffect::EvaluatePostTurn {
                completed_turn_id,
                ..
            } if completed_turn_id == "turn-1"
        )));
        assert!(reduction.state.auto_follow_state.has_live_activity());
        assert!(!reduction.state.can_accept_manual_prompt());

        let reduction = reduce_conversation_runtime(
            reduction.state,
            ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                evaluation: Box::new(PostTurnEvaluationOutcome {
                    provenance: PostTurnEvaluationProvenance::new("turn-1".to_string()),
                    runtime_projection: PlanningRuntimeProjection::uninitialized(),
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: PostTurnContinuationAction::SkipAutoFollow {
                        reason: AutoFollowSkipReason::LimitReached,
                    },
                    operator_alerts: Vec::new(),
                }),
            },
        );

        assert!(!reduction.state.auto_follow_state.has_live_activity());
        assert!(reduction.state.can_accept_manual_prompt());
    }

    #[test]
    fn stream_turn_completion_carries_execution_snapshot_to_post_turn_effect() {
        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.thread_id = "thread-1".to_string();
        state.replace_active_turn_workspace_directory("/tmp/workspace".to_string());
        let expected_lease = crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot::new(
            "slot-1",
            "task-1",
            "Task One",
            "agent-1",
            "akra-agent/slot-1/task-1",
            "/tmp/workspace",
            crate::domain::parallel_mode::ParallelModeSlotLeaseState::Running,
            "2026-07-12T00:00:00Z",
            Some("2026-07-12T00:00:01Z".to_string()),
        )
        .with_lease_generation("a".repeat(64));
        let snapshot_capture = PlanningTurnExecutionSnapshotCapture::ready(
            "/tmp/workspace",
            PlanningExecutionSnapshot::default(),
        )
        .with_parallel_slot_lease(Some(expected_lease.clone()));

        let reduction = with_akra_event_trace(|| {
            reduce_conversation_runtime(
                state,
                ConversationRuntimeEvent::StreamSnapshotApplied(Box::new(
                    TurnStreamState::new().apply_turn_completed(
                        "turn-1".to_string(),
                        vec!["new/docs/plan.md".to_string()],
                        snapshot_capture.clone(),
                    ),
                )),
            )
        });

        let post_turn_effect = reduction
            .effects
            .into_iter()
            .find_map(|effect| match effect {
                ConversationRuntimeEffect::EvaluatePostTurn {
                    execution_snapshot_capture,
                    ..
                } => execution_snapshot_capture,
                _ => None,
            })
            .expect("turn completion should queue post-turn evaluation with the snapshot capture");
        assert_eq!(post_turn_effect, snapshot_capture);
        assert_eq!(
            post_turn_effect.parallel_slot_lease.as_deref(),
            Some(&expected_lease)
        );
    }

    #[test]
    fn drained_planning_queue_skip_emits_operator_alert() {
        let state = ConversationViewModel::new_draft("/tmp/workspace".to_string());

        let reduction = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                evaluation: Box::new(PostTurnEvaluationOutcome {
                    provenance: PostTurnEvaluationProvenance::new("turn-root".to_string()),
                    runtime_projection: PlanningRuntimeProjection::ready_with_details(
                        "Planning Context".to_string(),
                        "queue idle: no executable planning task".to_string(),
                        None,
                        None,
                    ),
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: PostTurnContinuationAction::SkipAutoFollow {
                        reason: AutoFollowSkipReason::PlanningQueueDrained,
                    },
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

    #[test]
    fn queued_auto_prompt_uses_post_turn_provenance_for_handoff() {
        let mut state = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        state.auto_follow_state.set_max_auto_turns(3);
        let handoff_task = PlanningTaskHandoff {
            task_id: "task-1".to_string(),
            task_title: "Implement provenance".to_string(),
            direction_id: "general-workstream".to_string(),
            combined_priority: 10,
            updated_at: "2026-05-08T00:00:00Z".to_string(),
            status_label: "Ready".to_string(),
        };

        let reduction = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                evaluation: Box::new(PostTurnEvaluationOutcome {
                    provenance: PostTurnEvaluationProvenance::new(
                        "turn-from-provenance".to_string(),
                    )
                    .with_handoff_task(Some(handoff_task.clone())),
                    runtime_projection: PlanningRuntimeProjection::ready_with_details(
                        "Planning Context".to_string(),
                        "queue has a ready task".to_string(),
                        None,
                        None,
                    ),
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: PostTurnContinuationAction::QueueAutoPrompt(Box::new(
                        PostTurnQueuedPrompt {
                            prompt: "run task".to_string(),
                            mode_label: "planning queue".to_string(),
                            transcript_text: "next-task".to_string(),
                        },
                    )),
                    operator_alerts: Vec::new(),
                }),
            },
        );

        let queued_effect = reduction
            .effects
            .into_iter()
            .find_map(|effect| match effect {
                ConversationRuntimeEffect::QueueAutoPrompt {
                    completed_turn_id,
                    handoff_task,
                    ..
                } => Some((completed_turn_id, handoff_task)),
                _ => None,
            })
            .expect("post-turn queue action should emit an auto prompt effect");
        assert_eq!(queued_effect.0, "turn-from-provenance");
        assert_eq!(queued_effect.1, Some(handoff_task));
    }

    #[test]
    fn default_off_rejects_direct_auto_follow_submission() {
        let reduction = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/workspace".to_string()),
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "continue queue".to_string(),
                transcript_text: "continue queue".to_string(),
                origin: auto_follow_origin(),
            },
        );

        assert!(reduction.effects.is_empty());
        assert!(reduction.state.messages.is_empty());
        assert!(!reduction.state.auto_follow_state.can_queue_next());
    }

    #[test]
    fn manual_prompt_does_not_rearm_auto_follow_after_stop_or_off() {
        let mut stopped = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        stopped.auto_follow_state.set_max_auto_turns(5);
        stopped.auto_follow_state.pause_post_turn_continuation();
        let stopped = reduce_conversation_runtime(
            stopped,
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "manual work".to_string(),
                transcript_text: "manual work".to_string(),
                origin: PromptOrigin::Manual,
            },
        );
        assert!(
            stopped
                .state
                .auto_follow_state
                .post_turn_continuation_paused()
        );
        assert!(!stopped.state.auto_follow_state.can_queue_next());

        let disabled = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/workspace".to_string()),
            ConversationRuntimeEvent::SubmitPrompt {
                prompt: "manual work".to_string(),
                transcript_text: "manual work".to_string(),
                origin: PromptOrigin::Manual,
            },
        );
        assert!(!disabled.state.auto_follow_state.is_enabled());
        assert!(!disabled.state.auto_follow_state.can_queue_next());
    }

    #[test]
    fn stale_post_turn_queue_result_cannot_bypass_stop_or_off() {
        for mut state in [
            ConversationViewModel::new_draft("/tmp/stopped".to_string()),
            ConversationViewModel::new_draft("/tmp/off".to_string()),
        ] {
            if state.cwd.ends_with("stopped") {
                state.auto_follow_state.set_max_auto_turns(5);
                state.auto_follow_state.pause_post_turn_continuation();
            }
            let reduction = reduce_conversation_runtime(
                state,
                ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                    evaluation: Box::new(PostTurnEvaluationOutcome {
                        provenance: PostTurnEvaluationProvenance::new("turn-stale".to_string()),
                        runtime_projection: PlanningRuntimeProjection::ready_with_details(
                            "Planning Context".to_string(),
                            "queue has a ready task".to_string(),
                            None,
                            None,
                        ),
                        planning_repair_state: None,
                        runtime_notices: Vec::new(),
                        action: PostTurnContinuationAction::QueueAutoPrompt(Box::new(
                            PostTurnQueuedPrompt {
                                prompt: "must not run".to_string(),
                                mode_label: "planning queue".to_string(),
                                transcript_text: "must not run".to_string(),
                            },
                        )),
                        operator_alerts: Vec::new(),
                    }),
                },
            );

            assert!(
                !reduction.effects.iter().any(|effect| matches!(
                    effect,
                    ConversationRuntimeEffect::QueueAutoPrompt { .. }
                )),
                "stale result queued an automatic prompt for {}",
                reduction.state.cwd
            );
            assert!(
                reduction.state.status_text.contains("disabled")
                    || reduction.state.status_text.contains("disarmed")
            );
        }
    }

    #[test]
    fn explicit_parallel_queue_signal_obeys_stop_and_parallel_only_rearm() {
        let evaluation = |completed_turn_id: &str| PostTurnEvaluationOutcome {
            provenance: PostTurnEvaluationProvenance::new(completed_turn_id.to_string())
                .with_parallel_queue_signal(Some(
                    ParallelModePostTurnQueueSignal::AutoFollowQueued,
                )),
            runtime_projection: PlanningRuntimeProjection::ready_with_details(
                "Planning Context".to_string(),
                "queue has a ready task".to_string(),
                None,
                None,
            ),
            planning_repair_state: None,
            runtime_notices: Vec::new(),
            action: PostTurnContinuationAction::QueueAutoPrompt(Box::new(PostTurnQueuedPrompt {
                prompt: "dispatch ready task".to_string(),
                mode_label: "planning queue".to_string(),
                transcript_text: "dispatch ready task".to_string(),
            })),
            operator_alerts: Vec::new(),
        };

        let default_off = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/parallel".to_string()),
            ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                evaluation: Box::new(evaluation("turn-parallel")),
            },
        );
        assert!(
            default_off
                .effects
                .iter()
                .any(|effect| matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. }))
        );

        let mut stopped = ConversationViewModel::new_draft("/tmp/stopped".to_string());
        stopped.auto_follow_state.pause_post_turn_continuation();
        let stopped = reduce_conversation_runtime(
            stopped,
            ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                evaluation: Box::new(evaluation("turn-stopped")),
            },
        );
        assert!(
            !stopped
                .effects
                .iter()
                .any(|effect| matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. }))
        );
        assert!(stopped.state.status_text.contains("stopped and disarmed"));

        let mut parallel_rearmed =
            ConversationViewModel::new_draft("/tmp/parallel-rearmed".to_string());
        parallel_rearmed
            .auto_follow_state
            .pause_post_turn_continuation();
        parallel_rearmed
            .auto_follow_state
            .rearm_parallel_post_turn_continuation();
        assert!(
            parallel_rearmed
                .auto_follow_state
                .post_turn_continuation_paused(),
            "parallel rearm must not clear the single-session stop"
        );
        let parallel_rearmed = reduce_conversation_runtime(
            parallel_rearmed,
            ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                evaluation: Box::new(evaluation("turn-parallel-rearmed")),
            },
        );
        assert!(
            parallel_rearmed
                .effects
                .iter()
                .any(|effect| matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. })),
            "an explicit :parallel rearm must admit the parallel-only queue signal"
        );
        assert!(
            parallel_rearmed
                .state
                .auto_follow_state
                .post_turn_continuation_paused(),
            "dispatching parallel work must leave single-session auto-follow stopped"
        );
    }

    #[test]
    fn application_pause_reason_is_rendered_as_disabled_for_secure_default_off() {
        let reduction = reduce_conversation_runtime(
            ConversationViewModel::new_draft("/tmp/off".to_string()),
            ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                evaluation: Box::new(PostTurnEvaluationOutcome {
                    provenance: PostTurnEvaluationProvenance::new("turn-off".to_string()),
                    runtime_projection: PlanningRuntimeProjection::uninitialized(),
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: PostTurnContinuationAction::SkipAutoFollow {
                        reason: AutoFollowSkipReason::PostTurnContinuationPaused,
                    },
                    operator_alerts: Vec::new(),
                }),
            },
        );

        assert!(reduction.state.status_text.contains("auto-follow disabled"));
        assert!(!reduction.state.status_text.contains("disarmed"));
    }

    #[test]
    fn stale_off_evaluation_does_not_misreport_a_later_explicit_rearm() {
        let mut state = ConversationViewModel::new_draft("/tmp/rearmed".to_string());
        state.auto_follow_state.set_max_auto_turns(2);
        let reduction = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                evaluation: Box::new(PostTurnEvaluationOutcome {
                    provenance: PostTurnEvaluationProvenance::new("turn-before-rearm".to_string()),
                    runtime_projection: PlanningRuntimeProjection::uninitialized(),
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: PostTurnContinuationAction::SkipAutoFollow {
                        reason: AutoFollowSkipReason::PostTurnContinuationPaused,
                    },
                    operator_alerts: Vec::new(),
                }),
            },
        );

        assert!(reduction.state.status_text.contains("auto-follow re-armed"));
        assert!(reduction.state.auto_follow_state.can_queue_next());
    }
}
