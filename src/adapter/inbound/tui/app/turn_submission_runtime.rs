/* Turn submission is the execution layer for ConversationRuntimeEffect. The
 * reducer decides what should happen; this module gates the prompt against shell
 * readiness, sends stream startup and post-turn planning evaluation through the
 * core runtime, and re-enters the reducer for auto-follow prompts.
 */
#[path = "turn_submission_runtime/post_turn_execution.rs"]
mod post_turn_execution;

use crate::core::app::{
    AppCommand, AppEvent, CoreInput, CorePromptOrigin, ManualPromptPreparationAdmission,
    ManualPromptPreparationIntent, TurnSubmissionAdmission, TurnSubmissionRequest,
};
use crate::domain::parallel_mode::ParallelModeAutomationTrigger;
use crate::domain::planning::{
    ManualPlanningBootstrapFailureKind, ManualPromptCorrelation, ManualPromptIntakeOutcome,
    ManualPromptOutcome as ManualPromptPreparationResult,
    ParallelTurnHandoff as ParallelTurnSlotLeaseHandoff, PlanningQueueMutationKind,
    PlanningQueueMutationReceipt, PlanningQueueMutationReceiptEntry, QUEUED_TASK_TRANSCRIPT_TEXT,
    TaskStatus,
};
use post_turn_execution::PostTurnEvaluationRequest;

use super::conversation_input::MAX_PROMPT_INPUT_BYTES;
use super::planning::planning_worker_status_label;
use super::planning_worker_debug_preview::build_debug_preview_lines;
use super::{
    AutoFollowSubmitContext, ConversationComposerEvent, ConversationInputEvent,
    ConversationRuntimeEffect, ConversationRuntimeEvent, ConversationState, ConversationViewModel,
    InlineShellCommandInput, ManualIntakeSubmitContext, ManualPromptDelivery, NativeTuiApp,
    PARALLEL_SUPERVISOR_OPERATOR_ACTOR, PendingManualPromptPreparation, PromptOrigin,
    ShellActionAvailability, ShellChromeEvent,
};

const AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES: usize = 32;

impl NativeTuiApp {
    pub(super) fn start_turn_submission(&mut self) {
        // Enter first belongs to inline shell commands. Only non-command prompt
        // text becomes a conversation turn, and only when the current conversation
        // can accept a manual prompt.
        let inline_command = match &self.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => {
                InlineShellCommandInput::parse(&conversation.composer.input_buffer)
            }
            _ => None,
        };
        if let Some(command) = inline_command {
            self.execute_inline_shell_command_input(command);
            return;
        }
        if self.conversation.pending_turn_steer.is_some() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self
                    .shell
                    .tui_language
                    .turn_steer_pending_status()
                    .to_string(),
            });
            return;
        }
        let operator_prompt = match &self.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => conversation.composer.input_buffer.clone(),
            _ => return,
        };
        if operator_prompt.trim().is_empty() {
            return;
        }

        self.submit_manual_prompt_from_text(operator_prompt);
    }

    pub(super) fn execute_conversation_runtime_effect(
        &mut self,
        effect: ConversationRuntimeEffect,
    ) -> bool {
        // This switchboard is intentionally thin: stream work and post-turn planning
        // live in submodules, while auto-follow reuses the same submit path as a
        // manual prompt with a different origin.
        let mut turn_submission_admitted = false;
        match effect {
            ConversationRuntimeEffect::RequestTurnSubmission {
                workspace_directory,
                thread_id,
                prompt,
                transcript_text,
                prompt_origin,
            } => {
                let outcome = self.reduce_core_client_event(CoreInput::Command(
                    AppCommand::SubmitTurn(Box::new(self.build_turn_submission_request(
                        workspace_directory,
                        thread_id,
                        prompt,
                        &prompt_origin,
                    ))),
                ));
                turn_submission_admitted = outcome.events.iter().any(|event| {
                    matches!(
                        event,
                        AppEvent::TurnSubmissionAdmissionResolved(
                            TurnSubmissionAdmission::Accepted { .. }
                        )
                    )
                });
                let stale_auto_follow = !turn_submission_admitted
                    && matches!(&prompt_origin, PromptOrigin::AutoFollow(_));
                if turn_submission_admitted {
                    self.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::PromptSubmissionAdmitted {
                            transcript_text,
                            origin: prompt_origin,
                        },
                    );
                }
                // Admission is committed before stream events from an immediate
                // executor are applied, so even synchronous failure reduces from
                // the submitted state instead of being overwritten by it.
                self.apply_core_dispatch_outcome(outcome);
                if stale_auto_follow
                    && let ConversationState::Ready(conversation) =
                        &mut self.conversation.lifecycle.conversation_state
                {
                    conversation.record_stale_auto_follow_submission();
                }
            }
            ConversationRuntimeEffect::EvaluatePostTurn {
                workspace_directory,
                completed_turn_id,
                changed_planning_file_paths,
                execution_snapshot_capture,
            } => self.execute_post_turn_evaluation(PostTurnEvaluationRequest {
                workspace_directory,
                completed_turn_id,
                changed_planning_file_paths,
                execution_snapshot_capture,
            }),
            ConversationRuntimeEffect::QueueAutoPrompt {
                source,
                prompt,
                completed_turn_id,
                mode_label,
                transcript_text,
            } => {
                let debug_detail = self.build_auto_follow_transcript_debug_detail(&transcript_text);
                turn_submission_admitted = self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
                        source,
                        completed_turn_id,
                        mode_label,
                        transcript_text,
                        debug_detail,
                    })),
                );
            }
            ConversationRuntimeEffect::ShowApprovalOverlay => {
                self.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);
            }
            ConversationRuntimeEffect::CloseApprovalOverlay => {
                self.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayClosed);
            }
            ConversationRuntimeEffect::DispatchOperatorAlert { alert } => {
                if let Err(error) = self
                    .runtime
                    .tx
                    .try_send(super::BackgroundMessage::OperatorAlert(alert))
                {
                    // This effect runs on the UI thread, which also drains the
                    // bounded background queue. Never block that thread waiting
                    // on itself; the reducer has already persisted the alert as a
                    // transcript banner/runtime notice.
                    tracing::warn!(%error, "operator alert notification queue unavailable");
                }
            }
        }
        turn_submission_admitted
    }

    fn build_turn_submission_request(
        &self,
        workspace_directory: String,
        thread_id: Option<String>,
        prompt: String,
        prompt_origin: &PromptOrigin,
    ) -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            workspace_directory,
            thread_id,
            prompt,
            prompt_origin: core_prompt_origin(prompt_origin),
            auto_follow_source: match prompt_origin {
                PromptOrigin::AutoFollow(context) => Some(context.source.clone()),
                PromptOrigin::Manual | PromptOrigin::ManualIntake(_) => None,
            },
            planning_handoff: match prompt_origin {
                PromptOrigin::ManualIntake(context) => context.handoff_task.clone(),
                PromptOrigin::Manual | PromptOrigin::AutoFollow(_) => None,
            },
            turn_options: self.conversation.turn_options.clone(),
            slot_lease_handoff: self.build_parallel_mode_slot_lease_handoff(prompt_origin),
        }
    }

    fn build_parallel_mode_slot_lease_handoff(
        &self,
        prompt_origin: &PromptOrigin,
    ) -> Option<ParallelTurnSlotLeaseHandoff> {
        // A slot lease needs a concrete planning handoff so the parallel pool can
        // bind cleanup ownership. Application/domain code owns the lease request and
        // slug policy; the TUI only forwards task identity.
        if !prompt_origin_allows_parallel_slot_handoff(prompt_origin, self.parallel_mode_enabled())
        {
            return None;
        }
        let handoff_task = match prompt_origin {
            PromptOrigin::Manual => None,
            PromptOrigin::ManualIntake(context) => context.handoff_task.as_ref(),
            PromptOrigin::AutoFollow(_) => match &self.conversation.lifecycle.conversation_state {
                ConversationState::Ready(conversation) => conversation.last_planning_task_handoff(),
                ConversationState::Loading | ConversationState::Failed(_) => None,
            },
        };
        let handoff_task = handoff_task?;

        Some(ParallelTurnSlotLeaseHandoff::new(
            handoff_task.task_id.clone(),
            handoff_task.task_title.clone(),
        ))
    }

    pub(super) fn resolve_startup_submit_queue(&mut self) {
        let (startup_submit_armed, operator_prompt) =
            match &self.conversation.lifecycle.conversation_state {
                ConversationState::Ready(conversation) => (
                    conversation.composer.startup_submit_armed,
                    conversation.composer.input_buffer.clone(),
                ),
                ConversationState::Loading | ConversationState::Failed(_) => return,
            };
        if !startup_submit_armed {
            return;
        }

        // Prompts typed during startup checks are replayed only after the shell is
        // action-ready; blocked startup keeps the text in the buffer for the operator.
        match self.shell_action_availability() {
            ShellActionAvailability::Ready if operator_prompt.trim().is_empty() => {
                self.dispatch_conversation_input(
                    ConversationComposerEvent::StartupSubmitDisarmed { status_text: None },
                );
            }
            ShellActionAvailability::Ready => {
                self.submit_manual_prompt_from_text(operator_prompt);
            }
            ShellActionAvailability::Pending => {}
            ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(
                    ConversationComposerEvent::StartupSubmitDisarmed {
                        status_text: Some(format!(
                            "{}; queued prompt kept in buffer",
                            self.submission_blocked_status(PromptOrigin::Manual)
                        )),
                    },
                );
            }
        }
    }

    pub(super) fn submit_manual_prompt_from_text(&mut self, operator_prompt: String) {
        if operator_prompt.len() > MAX_PROMPT_INPUT_BYTES {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: format!(
                    "prompt exceeds the {MAX_PROMPT_INPUT_BYTES}-byte input limit; shorten it before submitting"
                ),
            });
            return;
        }
        let transcript_text = operator_prompt.trim().to_string();
        if transcript_text.is_empty() {
            return;
        }
        let (input_is_current, delivery, parent_thread_id, parent_turn_id) =
            match &self.conversation.lifecycle.conversation_state {
                ConversationState::Ready(conversation) => {
                    let delivery = manual_prompt_delivery(conversation);
                    (
                        delivery.is_some() && conversation.composer.input_buffer == operator_prompt,
                        delivery.unwrap_or(ManualPromptDelivery::StartTurn),
                        Some(conversation.thread_id.clone())
                            .filter(|thread_id| !thread_id.trim().is_empty()),
                        conversation.active_turn_id().map(str::to_string),
                    )
                }
                ConversationState::Loading | ConversationState::Failed(_) => {
                    (false, ManualPromptDelivery::StartTurn, None, None)
                }
            };
        if !input_is_current {
            return;
        }
        match self.shell_action_availability() {
            ShellActionAvailability::Pending => {
                self.dispatch_conversation_input(ConversationComposerEvent::StartupSubmitArmed {
                    status_text: "prompt queued until startup checks finish".to_string(),
                });
                return;
            }
            ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self.submission_blocked_status(PromptOrigin::Manual),
                });
                return;
            }
            ShellActionAvailability::Ready => {}
        }
        if self
            .conversation
            .pending_manual_prompt_preparation
            .is_some()
        {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text:
                    "turn preparation already in progress; wait for it to finish before submitting again"
                        .to_string(),
            });
            return;
        }

        let workspace_directory = self.planning_workspace_directory();
        let parallel_mode_enabled_at_submission = self.parallel_mode_enabled();
        let outcome = self.reduce_core_client_event(CoreInput::Command(
            AppCommand::PrepareManualPrompt(Box::new(ManualPromptPreparationIntent {
                workspace_directory,
                raw_prompt: transcript_text.clone(),
                parent_thread_id,
                parent_turn_id: parent_turn_id.clone(),
            })),
        ));
        let admission = outcome.events.iter().find_map(|event| match event {
            AppEvent::ManualPromptPreparationAdmissionResolved(admission) => {
                Some(admission.clone())
            }
            _ => None,
        });
        match admission {
            Some(ManualPromptPreparationAdmission::Accepted { correlation }) => {
                self.conversation.pending_manual_prompt_preparation =
                    Some(PendingManualPromptPreparation {
                        correlation,
                        source_input_buffer: operator_prompt,
                        transcript_text: transcript_text.clone(),
                        parallel_mode_enabled_at_submission,
                        delivery,
                        parent_turn_id,
                    });
                if parallel_mode_enabled_at_submission {
                    self.show_supersession_overlay();
                    self.record_parallel_supervisor_event(
                        PARALLEL_SUPERVISOR_OPERATOR_ACTOR,
                        format!(
                            "operator prompt submitted / chars: {}",
                            transcript_text.chars().count()
                        ),
                    );
                    self.record_parallel_supervisor_event(
                        "Task Intake",
                        "task generation started from the operator prompt.",
                    );
                    self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: "parallel task intake: preparing operator prompt".to_string(),
                    });
                }
            }
            Some(ManualPromptPreparationAdmission::RejectedActive { .. }) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text:
                        "previous turn preparation is still settling; retry after it finishes"
                            .to_string(),
                });
            }
            None => {}
        }
        // Install adapter-local prompt metadata before the background completion
        // can be polled from the Core mailbox.
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn apply_manual_prompt_preparation(
        &mut self,
        result: ManualPromptPreparationResult,
    ) {
        let correlation = result.correlation().clone();
        let result_transcript_text = result.transcript_text().to_string();
        let Some(pending) = self
            .conversation
            .pending_manual_prompt_preparation
            .as_ref()
            .filter(|pending| pending.correlation == correlation)
            .cloned()
        else {
            return;
        };
        let should_apply = result_transcript_text == pending.transcript_text
            && self.manual_prompt_preparation_is_current(&pending);
        let pending = self
            .take_exact_manual_prompt_preparation(&correlation)
            .expect("exact manual prompt preparation should remain pending");
        if !should_apply {
            return;
        }

        self.sync_ready_conversation_planning_runtime_projection(
            result.runtime_projection().clone(),
        );
        match result {
            ManualPromptPreparationResult::PromptReady {
                transcript_text,
                intake,
                ..
            } => {
                self.apply_manual_prompt_intake_outcome(
                    *intake,
                    transcript_text,
                    pending.parallel_mode_enabled_at_submission,
                    pending.delivery,
                    pending.parent_turn_id,
                );
            }
            ManualPromptPreparationResult::BootstrapReviewRequired {
                transcript_text: _,
                review,
                ..
            } => {
                let draft_name = review.draft_name.clone();
                self.planning
                    .planning_init_overlay_ui_state
                    .open_simple_review_summary(
                        crate::core::app::PlanningEditorSessionIdentity::new(
                            correlation.generation,
                            correlation.workspace_directory.clone(),
                            review.draft_name.clone(),
                        ),
                        review.draft_name,
                        review.staged_file_count,
                        review.validation_report,
                    );
                self.planning.planning_draft_editor_ui_state.reset();
                self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "planning bootstrap promote blocked / draft: {draft_name} / validation needs attention"
                    ),
                });
            }
            ManualPromptPreparationResult::BootstrapFailed {
                transcript_text: _,
                kind,
                reason,
                ..
            } => {
                let status_text = match kind {
                    ManualPlanningBootstrapFailureKind::Stage => {
                        format!("planning bootstrap failed: {reason}")
                    }
                    ManualPlanningBootstrapFailureKind::Promote => {
                        format!("planning bootstrap promote failed: {reason}")
                    }
                };
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            ManualPromptPreparationResult::Rejected {
                transcript_text,
                reason,
                ..
            } => {
                if pending.delivery == ManualPromptDelivery::QueueOnly {
                    self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: format!("queue preparation failed / {reason}; draft kept"),
                    });
                } else {
                    self.dispatch_conversation_input(
                        ConversationInputEvent::ManualPromptPreparationFailed {
                            transcript_text,
                            status_text: format!("turn preparation failed / {reason}"),
                        },
                    );
                }
            }
        }
    }

    fn apply_manual_prompt_intake_outcome(
        &mut self,
        outcome: ManualPromptIntakeOutcome,
        transcript_text: String,
        parallel_mode_enabled_at_submission: bool,
        delivery: ManualPromptDelivery,
        parent_turn_id: Option<String>,
    ) {
        if parallel_mode_enabled_at_submission {
            self.apply_parallel_manual_prompt_intake_outcome(outcome, transcript_text);
            return;
        }
        if delivery == ManualPromptDelivery::QueueOnly {
            self.apply_queue_only_manual_prompt_intake_outcome(
                outcome,
                transcript_text,
                parent_turn_id,
            );
            return;
        }

        match outcome {
            ManualPromptIntakeOutcome::TaskCommitted { handoff, .. }
            | ManualPromptIntakeOutcome::TaskUpdated { handoff, .. } => {
                if handoff.transcript_text != transcript_text {
                    return;
                }
                let _ = self.submit_prompt_with_transcript(
                    handoff.prompt,
                    handoff.transcript_text.clone(),
                    PromptOrigin::ManualIntake(Box::new(ManualIntakeSubmitContext {
                        transcript_text: handoff.transcript_text,
                        handoff_task: handoff.task,
                        parallel_mode_enabled_at_submission,
                    })),
                );
            }
            ManualPromptIntakeOutcome::Rejected { reason }
            | ManualPromptIntakeOutcome::Failed { reason } => {
                self.dispatch_conversation_input(
                    ConversationInputEvent::ManualPromptPreparationFailed {
                        transcript_text,
                        status_text: format!("turn preparation failed / {reason}"),
                    },
                );
            }
        }
    }

    fn apply_queue_only_manual_prompt_intake_outcome(
        &mut self,
        outcome: ManualPromptIntakeOutcome,
        transcript_text: String,
        parent_turn_id: Option<String>,
    ) {
        let (task_id, planning_revision, handoff, mutation_kind) = match outcome {
            ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id,
                committed_planning_revision,
                handoff,
            } => (
                committed_task_id,
                committed_planning_revision,
                handoff,
                PlanningQueueMutationKind::Created,
            ),
            ManualPromptIntakeOutcome::TaskUpdated {
                updated_task_id,
                committed_planning_revision,
                handoff,
            } => (
                updated_task_id,
                committed_planning_revision,
                handoff,
                PlanningQueueMutationKind::Updated,
            ),
            ManualPromptIntakeOutcome::Rejected { reason }
            | ManualPromptIntakeOutcome::Failed { reason } => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("queue preparation failed / {reason}; draft kept"),
                });
                return;
            }
        };
        if handoff.transcript_text != transcript_text {
            return;
        }

        self.dispatch_conversation_input(ConversationComposerEvent::InputCleared);
        let handoff_task = handoff.task;
        let undo_available =
            mutation_kind == PlanningQueueMutationKind::Created && handoff_task.is_some();
        // The preparation result already carried the committed runtime projection.
        // Invalidate any disposable Queue snapshot so the next open verifies its
        // destructive-action tokens off the input thread.
        self.planning.queue_overlay_ui_state.reset();
        let status_text = self.shell.tui_language.manual_prompt_queued_status(
            &task_id,
            planning_revision,
            undo_available,
        );
        if let ConversationState::Ready(conversation) =
            &mut self.conversation.lifecycle.conversation_state
        {
            conversation.latest_queue_mutation_receipt = None;
            if let Some(task) = handoff_task.filter(|_| undo_available) {
                conversation.latest_queue_mutation_receipt = Some(PlanningQueueMutationReceipt {
                    completed_turn_id: parent_turn_id
                        .unwrap_or_else(|| "manual-intake".to_string()),
                    planning_revision,
                    entries: vec![PlanningQueueMutationReceiptEntry {
                        task_id: task.task_id,
                        task_title: task.task_title,
                        mutation_kind,
                        before_status: None,
                        after_status: task_status_from_label(&task.status_label),
                        after_updated_at: task.updated_at,
                        unchanged_since_mutation: true,
                    }],
                });
            }
            conversation.status_text = status_text.clone();
            conversation.append_status_message(status_text.clone());
        }
        self.advance_planning_ui_intent_revision();
        self.planning
            .queue_overlay_ui_state
            .set_feedback(status_text);
    }

    pub(super) fn cancel_manual_prompt_preparation_for_identity_transition(&mut self) {
        self.conversation.pending_manual_prompt_preparation = None;
        self.conversation.turn_steer_confirmation = None;
        self.conversation.pending_turn_steer = None;
        self.dispatch_client_event(CoreInput::Command(
            AppCommand::CancelManualPromptPreparation,
        ));
    }

    fn take_exact_manual_prompt_preparation(
        &mut self,
        correlation: &ManualPromptCorrelation,
    ) -> Option<PendingManualPromptPreparation> {
        self.conversation
            .pending_manual_prompt_preparation
            .as_ref()
            .is_some_and(|pending| pending.correlation == *correlation)
            .then(|| self.conversation.pending_manual_prompt_preparation.take())
            .flatten()
    }

    fn manual_prompt_preparation_is_current(
        &self,
        pending: &PendingManualPromptPreparation,
    ) -> bool {
        if pending.correlation.workspace_directory != self.planning_workspace_directory() {
            return false;
        }
        match &self.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.composer.input_buffer == pending.source_input_buffer
            }
            ConversationState::Loading | ConversationState::Failed(_) => false,
        }
    }

    fn apply_parallel_manual_prompt_intake_outcome(
        &mut self,
        outcome: ManualPromptIntakeOutcome,
        transcript_text: String,
    ) {
        match outcome {
            ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id,
                committed_planning_revision,
                handoff,
            } => {
                self.apply_parallel_task_handoff(
                    "committed",
                    committed_task_id,
                    committed_planning_revision,
                    handoff,
                    &transcript_text,
                );
            }
            ManualPromptIntakeOutcome::TaskUpdated {
                updated_task_id,
                committed_planning_revision,
                handoff,
            } => {
                self.apply_parallel_task_handoff(
                    "updated",
                    updated_task_id,
                    committed_planning_revision,
                    handoff,
                    &transcript_text,
                );
            }
            ManualPromptIntakeOutcome::Rejected { reason }
            | ManualPromptIntakeOutcome::Failed { reason } => {
                self.dispatch_conversation_input(ConversationComposerEvent::InputCleared);
                self.record_parallel_supervisor_event(
                    "Task Intake",
                    format!(
                        "task generation failed / prompt chars: {} / {}",
                        transcript_text.chars().count(),
                        reason
                    ),
                );
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("parallel task intake failed / {reason}"),
                });
            }
        }
    }

    fn apply_parallel_task_handoff(
        &mut self,
        verb: &str,
        task_id: String,
        committed_planning_revision: i64,
        handoff: crate::domain::planning::ManualPromptMainSessionHandoff,
        expected_transcript_text: &str,
    ) {
        if handoff.transcript_text != expected_transcript_text {
            return;
        }
        self.dispatch_conversation_input(ConversationComposerEvent::InputCleared);

        let task_title = handoff
            .task
            .as_ref()
            .map(|task| task.task_title.as_str())
            .unwrap_or("untitled task");
        self.record_parallel_supervisor_event(
            "Task Intake",
            format!(
                "{} task {} / rev {} / {}",
                verb,
                task_id,
                committed_planning_revision,
                truncate_parallel_prompt_event_text(task_title, 72)
            ),
        );
        self.record_parallel_supervisor_event(
            "Orchestrator",
            "task intake dispatch requested for the parallel pool.",
        );
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: format!("parallel task intake: {verb} {task_id} / dispatch requested"),
        });

        let workspace_directory = self.planning_workspace_directory();
        self.open_parallel_mode_automation_epoch(workspace_directory.clone());
        self.request_parallel_mode_dispatch(
            workspace_directory,
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            None,
        );
    }

    pub(super) fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) -> bool {
        let transcript_text = match &prompt_origin {
            PromptOrigin::Manual => prompt.trim().to_string(),
            PromptOrigin::ManualIntake(context) => context.transcript_text.clone(),
            PromptOrigin::AutoFollow(context) => context.transcript_text.clone(),
        };
        self.submit_prompt_with_transcript(prompt, transcript_text, prompt_origin)
    }

    pub(super) fn submit_prompt_with_transcript(
        &mut self,
        prompt: String,
        transcript_text: String,
        prompt_origin: PromptOrigin,
    ) -> bool {
        if matches!(
            prompt_origin,
            PromptOrigin::Manual | PromptOrigin::ManualIntake(_)
        ) && matches!(
            self.shell_action_availability(),
            ShellActionAvailability::Pending
        ) {
            self.dispatch_conversation_input(ConversationComposerEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            });
            return false;
        }

        if !self.shell_action_availability().allows_actions() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.submission_blocked_status(prompt_origin),
            });
            return false;
        }

        crate::akra_event!(
            tracing::Level::DEBUG,
            "user_prompt_submit_inspected",
            origin = prompt_origin_label(&prompt_origin),
            transcript_text_len = transcript_text.len(),
            prompt_len = prompt.len(),
            parallel_mode_enabled = self.parallel_mode_enabled(),
        );

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            transcript_text,
            origin: prompt_origin,
        })
    }

    fn build_auto_follow_transcript_debug_detail(&self, transcript_text: &str) -> Option<String> {
        if !self.planning_worker_shows_debug_details()
            || transcript_text != QUEUED_TASK_TRANSCRIPT_TEXT
        {
            return None;
        }
        let planning_worker = self.planning.planning_worker_panel_state.current();
        let operation_label = planning_worker
            .last_operation_label
            .as_deref()
            .unwrap_or("unknown");
        let prompt = planning_worker.last_prompt.as_deref();
        let response = planning_worker.last_response.as_deref();
        let summary = planning_worker.last_summary.as_deref();
        if prompt.is_none() && response.is_none() && summary.is_none() {
            return None;
        }
        let mut lines = vec![format!(
            "planning worker temporary session: {operation_label} / {}",
            planning_worker_status_label(planning_worker.status)
        )];
        if let Some(summary) = summary.filter(|summary: &&str| !summary.trim().is_empty()) {
            lines.push(format!("planning worker summary: {summary}"));
        }
        append_debug_detail_preview_block(&mut lines, "planning worker prompt:", prompt);
        append_debug_detail_preview_block(&mut lines, "planning worker response:", response);

        Some(lines.join("\n"))
    }
}

fn manual_prompt_delivery(conversation: &ConversationViewModel) -> Option<ManualPromptDelivery> {
    if conversation.can_accept_manual_prompt() {
        Some(ManualPromptDelivery::StartTurn)
    } else if conversation.has_running_turn() {
        Some(ManualPromptDelivery::QueueOnly)
    } else {
        None
    }
}

fn task_status_from_label(label: &str) -> TaskStatus {
    match label {
        "blocked" => TaskStatus::Blocked,
        "in_progress" => TaskStatus::InProgress,
        "done" => TaskStatus::Done,
        "cancelled" => TaskStatus::Cancelled,
        "awaiting_user" => TaskStatus::AwaitingUser,
        "proposed" => TaskStatus::Proposed,
        _ => TaskStatus::Ready,
    }
}

#[cfg(test)]
fn user_prompt_submit_detail(
    prompt: &str,
    transcript_text: &str,
    prompt_origin: &PromptOrigin,
    parallel_mode_enabled: bool,
) -> serde_json::Value {
    serde_json::json!({
        "origin": prompt_origin_label(prompt_origin),
        "transcript_text_len": transcript_text.len(),
        "prompt_len": prompt.len(),
        "parallel_mode_enabled": parallel_mode_enabled,
    })
}

fn prompt_origin_label(prompt_origin: &PromptOrigin) -> &'static str {
    match prompt_origin {
        PromptOrigin::Manual => "Manual",
        PromptOrigin::ManualIntake(_) => "ManualIntake",
        PromptOrigin::AutoFollow(_) => "AutoFollow",
    }
}

fn truncate_parallel_prompt_event_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let keep = max_chars.saturating_sub(3);
    let mut truncated = trimmed.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn prompt_origin_allows_parallel_slot_handoff(
    prompt_origin: &PromptOrigin,
    current_parallel_mode_enabled: bool,
) -> bool {
    match prompt_origin {
        PromptOrigin::Manual | PromptOrigin::AutoFollow(_) => current_parallel_mode_enabled,
        PromptOrigin::ManualIntake(context) => context.parallel_mode_enabled_at_submission,
    }
}

fn core_prompt_origin(prompt_origin: &PromptOrigin) -> CorePromptOrigin {
    match prompt_origin {
        PromptOrigin::Manual => CorePromptOrigin::Manual,
        PromptOrigin::ManualIntake(_) => CorePromptOrigin::ManualIntake,
        PromptOrigin::AutoFollow(_) => CorePromptOrigin::AutoFollow,
    }
}

fn append_debug_detail_preview_block(lines: &mut Vec<String>, label: &str, block: Option<&str>) {
    let Some(block) = block.filter(|block| !block.trim().is_empty()) else {
        return;
    };

    lines.push(label.to_string());
    for line in build_debug_preview_lines(block, AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES) {
        lines.push(format!("  {line}"));
    }
}

#[cfg(test)]
mod prompt_submit_diagnostics_tests {
    use super::{PromptOrigin, core_prompt_origin, user_prompt_submit_detail};
    use crate::core::app::CorePromptOrigin;

    #[test]
    fn user_prompt_submit_detail_keeps_lengths_without_raw_prompt_text() {
        let detail = user_prompt_submit_detail(
            "final wrapper\noperator text",
            "operator text",
            &PromptOrigin::Manual,
            true,
        );

        assert_eq!(detail["origin"], "Manual");
        assert_eq!(detail["transcript_text_len"], 13);
        assert_eq!(detail["prompt_len"], 27);
        assert_eq!(detail["parallel_mode_enabled"], true);
        assert!(detail.get("transcript_text").is_none());
        assert!(detail.get("prompt").is_none());
    }

    #[test]
    fn prompt_origin_maps_to_core_origin_without_tui_context() {
        assert_eq!(
            core_prompt_origin(&PromptOrigin::Manual),
            CorePromptOrigin::Manual
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers;
    use crate::adapter::inbound::tui::app::{
        AutoFollowSubmitContext, BackgroundMessage, ConversationInputState, ConversationState,
        ConversationViewMode, NativeTuiApp, NativeTuiParallelModeBinding, PlanningInitOverlayStep,
        PlanningWorkerVisibility, ShellOverlay, StartupState, TuiLanguage,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    use crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort;
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::{
        AppServerStartupContext, StartupProbePort,
    };
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::manual_prompt_preparation::{
        ManualPlanningBootstrapReview, ManualPromptPreparationResult,
    };
    use crate::application::service::parallel_mode::turn::ParallelTurnSlotLeaseHandoff;
    use crate::application::service::planning::{
        ManualPromptIntakeOutcome, ManualPromptMainSessionHandoff, PlanningRuntimeProjection,
        PlanningTaskHandoff,
    };
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::core::app::{CoreInput, CorePromptOrigin, StartupReadySnapshot, TurnStreamEvent};
    use crate::domain::conversation::{
        ConversationReasoningEffort, ConversationRuntimeControlTruth, ConversationSnapshot,
        ConversationTurnOptions,
    };
    use crate::domain::operator_alert::OperatorAlert;
    use crate::domain::planning::PlanningValidationReport;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use anyhow::Result;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct FakeAppServerPort;

    impl StartupProbePort for FakeAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }
    }

    impl SessionCatalogPort for FakeAppServerPort {
        fn load_session_catalog(&self, _request: SessionCatalogRequest) -> Result<SessionCatalog> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }
    }

    impl InteractiveTurnRuntimePort for FakeAppServerPort {
        fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
            ConversationRuntimeControlTruth::codex_app_server()
        }

        fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            })
        }

        fn request_stop_all_sessions(&self) -> Result<()> {
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            cwd: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            event_sender: crate::application::port::conversation_stream::ConversationStreamSender,
        ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
            crate::application::port::conversation_stream::emit_confirmed_test_terminal_receipt(
                &event_sender,
                "test-thread",
                cwd,
            )
        }

        fn run_turn_stream(
            &self,
            thread_id: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            event_sender: crate::application::port::conversation_stream::ConversationStreamSender,
        ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
            crate::application::port::conversation_stream::emit_confirmed_test_terminal_receipt(
                &event_sender,
                thread_id,
                "/tmp/test-workspace",
            )
        }
    }

    struct TempWorkspace {
        path: PathBuf,
        path_text: String,
    }

    impl TempWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&path).expect("temp workspace should be created");
            let path_text = path.display().to_string();
            Self { path, path_text }
        }

        fn path_str(&self) -> &str {
            &self.path_text
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn make_test_app(workspace: &TempWorkspace) -> NativeTuiApp {
        let codex_port = Arc::new(FakeAppServerPort);
        let planning = test_helpers::test_planning_services(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        let parallel_mode_control_plane_composition =
            test_helpers::test_parallel_mode_control_plane_composition_with_worker(
                test_helpers::test_parallel_mode_service(),
                planning,
                Arc::new(NoopParallelAgentWorkerPort),
            );
        let parallel_mode_binding =
            NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
        let mut app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            parallel_mode_binding,
        );
        app.shell.chrome.startup_state =
            StartupState::Ready(startup_ready_snapshot(workspace.path_str(), true));
        app.sync_draft_shell_workspace(workspace.path_str());
        app
    }

    fn startup_ready_snapshot(
        workspace_path: &str,
        can_continue: bool,
    ) -> Box<StartupReadySnapshot> {
        Box::new(StartupReadySnapshot::from_diagnostics(StartupDiagnostics {
            cwd: workspace_path.to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "ok".to_string(),
            workspace_ok: true,
            workspace_path: workspace_path.to_string(),
            workspace_detail: "ok".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_ok: true,
            initialize_detail: "ok".to_string(),
            account_ok: can_continue,
            account_detail: if can_continue {
                "ok"
            } else {
                "missing account"
            }
            .to_string(),
            warnings: Vec::new(),
            schema_snapshot: "schema".to_string(),
        }))
    }

    fn ready_conversation(app: &NativeTuiApp) -> &super::super::ConversationViewModel {
        match &app.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => conversation,
            other => panic!("conversation should be ready, got {other:?}"),
        }
    }

    fn ready_conversation_mut(app: &mut NativeTuiApp) -> &mut super::super::ConversationViewModel {
        match &mut app.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => conversation,
            other => panic!("conversation should be ready, got {other:?}"),
        }
    }

    fn post_turn_source(
        completed_turn_id: &str,
        workspace_directory: &str,
    ) -> crate::core::app::PostTurnEvaluationCorrelation {
        crate::core::app::PostTurnEvaluationCorrelation::new(
            1,
            "thread-1",
            completed_turn_id,
            workspace_directory,
            workspace_directory,
        )
    }

    fn install_running_turn(
        conversation: &mut super::super::ConversationViewModel,
        turn_id: &str,
        workspace_directory: &str,
    ) {
        let mut runtime = conversation.runtime_snapshot().clone();
        runtime.active_turn = Some(crate::core::app::ActiveTurnSnapshot {
            correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
            phase: crate::core::app::ActiveTurnPhase::Running,
            workspace_directory: workspace_directory.to_string(),
            turn_id: Some(turn_id.to_string()),
            prompt_origin: crate::core::app::CorePromptOrigin::Manual,
            started_at: Instant::now(),
        });
        conversation.apply_runtime_snapshot(runtime);
        conversation.record_turn_started(turn_id.to_string());
    }

    fn install_running_core_turn(app: &mut NativeTuiApp, turn_id: &str, workspace_directory: &str) {
        let correlation = app.runtime.client_runtime.begin_test_turn_submission();
        app.dispatch_client_event(CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-running".to_string(),
                title: "Running".to_string(),
                cwd: workspace_directory.to_string(),
                runtime_envelope: Box::default(),
            },
        });
        app.dispatch_client_event(CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::TurnStarted {
                turn_id: turn_id.to_string(),
                runtime_request: Box::default(),
            },
        });
    }

    fn set_input(app: &mut NativeTuiApp, input: &str) {
        ready_conversation_mut(app).composer.input_buffer = input.to_string();
    }

    fn poll_manual_prompt_preparation_completion(app: &mut NativeTuiApp) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.conversation.pending_manual_prompt_preparation.is_some()
            && Instant::now() < deadline
        {
            app.poll_client_runtime_events(16);
            std::thread::yield_now();
        }
        assert!(
            app.conversation.pending_manual_prompt_preparation.is_none(),
            "manual prompt preparation should complete through the Core mailbox"
        );
    }

    fn arm_manual_prompt_preparation(
        app: &mut NativeTuiApp,
        transcript_text: &str,
    ) -> ManualPromptCorrelation {
        static NEXT_FIXTURE_CORRELATION: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT_FIXTURE_CORRELATION.fetch_add(1, Ordering::Relaxed);
        let correlation = ManualPromptCorrelation {
            request_id: generation,
            generation,
            workspace_directory: app.planning_workspace_directory(),
        };
        app.conversation.pending_manual_prompt_preparation = Some(PendingManualPromptPreparation {
            correlation: correlation.clone(),
            source_input_buffer: transcript_text.to_string(),
            transcript_text: transcript_text.to_string(),
            parallel_mode_enabled_at_submission: app.parallel_mode_enabled(),
            delivery: ManualPromptDelivery::StartTurn,
            parent_turn_id: None,
        });
        correlation
    }

    fn different_manual_prompt_correlation(
        correlation: &ManualPromptCorrelation,
    ) -> ManualPromptCorrelation {
        ManualPromptCorrelation {
            request_id: correlation.request_id.wrapping_add(1),
            generation: correlation.generation,
            workspace_directory: correlation.workspace_directory.clone(),
        }
    }

    fn rejected_manual_prompt_result(
        correlation: ManualPromptCorrelation,
        transcript_text: &str,
        reason: &str,
    ) -> ManualPromptPreparationResult {
        ManualPromptPreparationResult::Rejected {
            correlation,
            transcript_text: transcript_text.to_string(),
            runtime_projection: runtime_projection(),
            reason: reason.to_string(),
        }
    }

    fn runtime_projection() -> Box<PlanningRuntimeProjection> {
        Box::new(PlanningRuntimeProjection::ready_with_details(
            "Planning Context".to_string(),
            "queue idle".to_string(),
            None,
            None,
        ))
    }

    fn sample_handoff_task() -> PlanningTaskHandoff {
        PlanningTaskHandoff {
            task_id: "task-1".to_string(),
            task_title: "Implement turn submission coverage".to_string(),
            direction_id: "general-workstream".to_string(),
            combined_priority: 10,
            updated_at: "2026-05-12T00:00:00Z".to_string(),
            status_label: "Ready".to_string(),
        }
    }

    fn handoff(
        prompt: &str,
        transcript_text: &str,
        task: Option<PlanningTaskHandoff>,
    ) -> ManualPromptMainSessionHandoff {
        ManualPromptMainSessionHandoff {
            prompt: prompt.to_string(),
            transcript_text: transcript_text.to_string(),
            task,
        }
    }

    fn auto_follow_origin() -> PromptOrigin {
        PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
            source: post_turn_source("turn-1", "/tmp/workspace"),
            completed_turn_id: "turn-1".to_string(),
            mode_label: "planning queue".to_string(),
            transcript_text: QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            debug_detail: None,
        }))
    }

    #[test]
    fn start_turn_submission_handles_inline_blank_and_unready_states() {
        let workspace = TempWorkspace::new("turn-submit-start-entry");
        let mut inline_app = make_test_app(&workspace);
        set_input(&mut inline_app, ":view");

        inline_app.start_turn_submission();

        assert_eq!(
            inline_app.shell.chrome.shell_overlay,
            ShellOverlay::ViewSelection
        );
        assert!(
            inline_app
                .conversation
                .pending_manual_prompt_preparation
                .is_none()
        );

        let mut blank_app = make_test_app(&workspace);
        set_input(&mut blank_app, "   ");

        blank_app.start_turn_submission();

        assert!(
            blank_app
                .conversation
                .pending_manual_prompt_preparation
                .is_none()
        );
        assert!(ready_conversation(&blank_app).messages.is_empty());

        let mut loading_app = make_test_app(&workspace);
        loading_app.conversation.lifecycle.conversation_state = ConversationState::Loading;

        loading_app.start_turn_submission();

        assert!(matches!(
            loading_app.conversation.lifecycle.conversation_state,
            ConversationState::Loading
        ));
    }

    #[test]
    fn execute_runtime_effects_cover_stream_and_post_turn_dispatch_arms() {
        let workspace = TempWorkspace::new("turn-submit-effect-arms");
        let mut app = make_test_app(&workspace);

        app.execute_conversation_runtime_effect(ConversationRuntimeEffect::RequestTurnSubmission {
            workspace_directory: workspace.path_str().to_string(),
            thread_id: None,
            prompt: "ship it".to_string(),
            transcript_text: "ship it".to_string(),
            prompt_origin: PromptOrigin::Manual,
        });
        app.execute_conversation_runtime_effect(ConversationRuntimeEffect::EvaluatePostTurn {
            workspace_directory: workspace.path_str().to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
        });

        assert!(matches!(
            app.conversation.lifecycle.conversation_state,
            ConversationState::Ready(_)
        ));
    }

    #[test]
    fn submit_prompt_respects_startup_readiness_gates() {
        let workspace = TempWorkspace::new("turn-submit-startup-gates");
        let mut pending_app = make_test_app(&workspace);
        pending_app.shell.chrome.startup_state = StartupState::Loading;
        set_input(&mut pending_app, "ship it");

        assert!(!pending_app.submit_prompt_with_transcript(
            "ship it".to_string(),
            "ship it".to_string(),
            PromptOrigin::Manual,
        ));

        let pending_conversation = ready_conversation(&pending_app);
        assert!(pending_conversation.composer.startup_submit_armed);
        assert_eq!(
            pending_conversation.status_text,
            "prompt queued until startup checks finish"
        );

        let mut blocked_app = make_test_app(&workspace);
        blocked_app.shell.chrome.startup_state =
            StartupState::Ready(startup_ready_snapshot(workspace.path_str(), false));

        assert!(!blocked_app.submit_prompt_with_transcript(
            "continue queue".to_string(),
            QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            auto_follow_origin(),
        ));

        assert_eq!(
            ready_conversation(&blocked_app).status_text,
            "auto-follow paused because startup diagnostics need attention"
        );
    }

    #[test]
    fn manual_preparation_waits_for_startup_before_planning_side_effects() {
        let workspace = TempWorkspace::new("turn-submit-manual-startup-gate");
        let mut pending_app = make_test_app(&workspace);
        pending_app.shell.chrome.startup_state = StartupState::Loading;
        set_input(&mut pending_app, "ship it");

        pending_app.submit_manual_prompt_from_text("ship it".to_string());

        assert!(
            pending_app
                .conversation
                .pending_manual_prompt_preparation
                .is_none()
        );
        assert!(
            ready_conversation(&pending_app)
                .composer
                .startup_submit_armed
        );
        assert_eq!(
            ready_conversation(&pending_app).composer.input_buffer,
            "ship it"
        );
        assert!(ready_conversation(&pending_app).messages.is_empty());

        pending_app.shell.chrome.startup_state =
            StartupState::Ready(startup_ready_snapshot(workspace.path_str(), true));
        pending_app.resolve_startup_submit_queue();
        poll_manual_prompt_preparation_completion(&mut pending_app);

        assert!(
            !ready_conversation(&pending_app)
                .composer
                .startup_submit_armed
        );
        assert_eq!(ready_conversation(&pending_app).composer.input_buffer, "");
        assert_eq!(ready_conversation(&pending_app).messages.len(), 1);
    }

    #[test]
    fn manual_preparation_tracks_raw_draft_and_submits_trimmed_transcript() {
        let workspace = TempWorkspace::new("turn-submit-manual-whitespace");
        let mut app = make_test_app(&workspace);
        set_input(&mut app, "  ship it  ");

        app.submit_manual_prompt_from_text("  ship it  ".to_string());
        poll_manual_prompt_preparation_completion(&mut app);

        let conversation = ready_conversation(&app);
        assert_eq!(conversation.composer.input_buffer, "");
        assert_eq!(conversation.messages.len(), 1);
        assert_eq!(conversation.messages[0].text, "ship it");
    }

    #[test]
    fn blocked_startup_never_enters_manual_planning_preparation() {
        let workspace = TempWorkspace::new("turn-submit-manual-startup-blocked");
        let mut app = make_test_app(&workspace);
        app.shell.chrome.startup_state =
            StartupState::Ready(startup_ready_snapshot(workspace.path_str(), false));
        set_input(&mut app, "ship it");

        app.submit_manual_prompt_from_text("ship it".to_string());

        assert!(app.conversation.pending_manual_prompt_preparation.is_none());
        assert!(!ready_conversation(&app).composer.startup_submit_armed);
        assert_eq!(ready_conversation(&app).composer.input_buffer, "ship it");
        assert!(ready_conversation(&app).messages.is_empty());
        assert!(
            ready_conversation(&app)
                .status_text
                .contains("startup diagnostics need attention")
        );
    }

    #[test]
    fn resolve_startup_submit_queue_keeps_or_disarms_buffered_prompt() {
        let workspace = TempWorkspace::new("turn-submit-startup-queue");
        let mut pending_app = make_test_app(&workspace);
        pending_app.shell.chrome.startup_state = StartupState::Loading;
        set_input(&mut pending_app, "queued prompt");
        pending_app.dispatch_conversation_input(ConversationComposerEvent::StartupSubmitArmed {
            status_text: "queued".to_string(),
        });

        pending_app.resolve_startup_submit_queue();

        assert!(
            ready_conversation(&pending_app)
                .composer
                .startup_submit_armed
        );
        assert_eq!(
            ready_conversation(&pending_app).composer.input_buffer,
            "queued prompt"
        );

        let mut blocked_app = make_test_app(&workspace);
        blocked_app.shell.chrome.startup_state =
            StartupState::Ready(startup_ready_snapshot(workspace.path_str(), false));
        set_input(&mut blocked_app, "queued prompt");
        blocked_app.dispatch_conversation_input(ConversationComposerEvent::StartupSubmitArmed {
            status_text: "queued".to_string(),
        });

        blocked_app.resolve_startup_submit_queue();

        let blocked_conversation = ready_conversation(&blocked_app);
        assert!(!blocked_conversation.composer.startup_submit_armed);
        assert_eq!(blocked_conversation.composer.input_buffer, "queued prompt");
        assert_eq!(
            blocked_conversation.status_text,
            "startup diagnostics need attention; open diagnostics with Ctrl+d; queued prompt kept in buffer"
        );

        let mut ready_empty_app = make_test_app(&workspace);
        set_input(&mut ready_empty_app, "   ");
        ready_empty_app.dispatch_conversation_input(
            ConversationComposerEvent::StartupSubmitArmed {
                status_text: "queued".to_string(),
            },
        );

        ready_empty_app.resolve_startup_submit_queue();

        assert!(
            !ready_conversation(&ready_empty_app)
                .composer
                .startup_submit_armed
        );
    }

    #[test]
    fn resolve_startup_submit_queue_replays_ready_buffered_prompt() {
        let workspace = TempWorkspace::new("turn-submit-startup-replay");
        let mut app = make_test_app(&workspace);
        set_input(&mut app, "  queued prompt  ");
        app.dispatch_conversation_input(ConversationComposerEvent::StartupSubmitArmed {
            status_text: "queued".to_string(),
        });

        app.resolve_startup_submit_queue();
        poll_manual_prompt_preparation_completion(&mut app);

        let conversation = ready_conversation(&app);
        assert!(app.conversation.pending_manual_prompt_preparation.is_none());
        assert_eq!(conversation.composer.input_buffer, "");
        assert_eq!(
            conversation
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some("queued prompt")
        );
        assert_eq!(conversation.status_text, "starting turn");
    }

    #[test]
    fn apply_manual_prompt_preparation_routes_bootstrap_review_and_failures() {
        let workspace = TempWorkspace::new("turn-submit-bootstrap-review");
        let mut review_app = make_test_app(&workspace);
        set_input(&mut review_app, "create the planning workspace");
        let review_correlation =
            arm_manual_prompt_preparation(&mut review_app, "create the planning workspace");

        review_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::BootstrapReviewRequired {
                correlation: review_correlation,
                transcript_text: "create the planning workspace".to_string(),
                runtime_projection: runtime_projection(),
                review: ManualPlanningBootstrapReview {
                    draft_name: "simple-draft".to_string(),
                    staged_file_count: 2,
                    validation_report: PlanningValidationReport::default(),
                },
            },
        );

        assert_eq!(
            review_app.shell.chrome.shell_overlay,
            ShellOverlay::PlanningInit
        );
        assert_eq!(
            review_app.planning.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        let review = review_app
            .planning
            .planning_init_overlay_ui_state
            .simple_review()
            .expect("bootstrap review should be retained for the overlay");
        assert_eq!(review.draft_name(), "simple-draft");
        assert_eq!(review.staged_file_count(), 2);
        assert_eq!(
            ready_conversation(&review_app).status_text,
            "planning bootstrap promote blocked / draft: simple-draft / validation needs attention"
        );

        for (kind, expected_status) in [
            (
                ManualPlanningBootstrapFailureKind::Stage,
                "planning bootstrap failed: disk full",
            ),
            (
                ManualPlanningBootstrapFailureKind::Promote,
                "planning bootstrap promote failed: disk full",
            ),
        ] {
            let mut failure_app = make_test_app(&workspace);
            set_input(&mut failure_app, "create the planning workspace");
            let failure_correlation =
                arm_manual_prompt_preparation(&mut failure_app, "create the planning workspace");

            failure_app.apply_manual_prompt_preparation(
                ManualPromptPreparationResult::BootstrapFailed {
                    correlation: failure_correlation,
                    transcript_text: "create the planning workspace".to_string(),
                    runtime_projection: runtime_projection(),
                    kind,
                    reason: "disk full".to_string(),
                },
            );

            assert_eq!(
                ready_conversation(&failure_app).status_text,
                expected_status
            );
        }
    }

    #[test]
    fn apply_manual_prompt_preparation_rejects_and_ignores_stale_results() {
        let workspace = TempWorkspace::new("turn-submit-prep-rejected");
        let mut app = make_test_app(&workspace);
        set_input(&mut app, "ship it");
        let correlation = arm_manual_prompt_preparation(&mut app, "ship it");

        app.apply_manual_prompt_preparation(ManualPromptPreparationResult::Rejected {
            correlation,
            transcript_text: "ship it".to_string(),
            runtime_projection: runtime_projection(),
            reason: "not actionable".to_string(),
        });

        let conversation = ready_conversation(&app);
        assert_eq!(
            conversation.status_text,
            "turn preparation failed / not actionable"
        );
        assert_eq!(conversation.composer.input_buffer, "");
        assert_eq!(conversation.messages.last().unwrap().text, "ship it");

        let mut stale_app = make_test_app(&workspace);
        set_input(&mut stale_app, "newer text");
        let current_correlation = arm_manual_prompt_preparation(&mut stale_app, "newer text");
        let stale_correlation = different_manual_prompt_correlation(&current_correlation);
        let previous_status = ready_conversation(&stale_app).status_text.clone();

        stale_app.apply_manual_prompt_preparation(ManualPromptPreparationResult::Rejected {
            correlation: stale_correlation,
            transcript_text: "older text".to_string(),
            runtime_projection: runtime_projection(),
            reason: "should not surface".to_string(),
        });

        assert_eq!(ready_conversation(&stale_app).status_text, previous_status);
        assert!(ready_conversation(&stale_app).messages.is_empty());
        assert_eq!(
            stale_app
                .conversation
                .pending_manual_prompt_preparation
                .as_ref()
                .map(|pending| &pending.correlation),
            Some(&current_correlation)
        );
    }

    #[test]
    fn apply_manual_prompt_preparation_ignores_stale_success_review_and_failure_results() {
        let workspace = TempWorkspace::new("turn-submit-prep-stale-extra");
        let task = sample_handoff_task();

        let mut stale_success_app = make_test_app(&workspace);
        set_input(&mut stale_success_app, "newer text");
        let current_correlation =
            arm_manual_prompt_preparation(&mut stale_success_app, "newer text");
        let stale_correlation = different_manual_prompt_correlation(&current_correlation);
        let previous_status = ready_conversation(&stale_success_app).status_text.clone();
        stale_success_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::PromptReady {
                correlation: stale_correlation,
                transcript_text: "older text".to_string(),
                runtime_projection: runtime_projection(),
                intake: Box::new(ManualPromptIntakeOutcome::TaskUpdated {
                    updated_task_id: task.task_id.clone(),
                    committed_planning_revision: 8,
                    handoff: handoff("wrapped", "older text", Some(task.clone())),
                }),
            },
        );
        assert_eq!(
            ready_conversation(&stale_success_app).status_text,
            previous_status
        );
        assert!(ready_conversation(&stale_success_app).messages.is_empty());

        let mut stale_review_app = make_test_app(&workspace);
        set_input(&mut stale_review_app, "newer text");
        let current_correlation =
            arm_manual_prompt_preparation(&mut stale_review_app, "newer text");
        let stale_correlation = different_manual_prompt_correlation(&current_correlation);
        stale_review_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::BootstrapReviewRequired {
                correlation: stale_correlation,
                transcript_text: "older text".to_string(),
                runtime_projection: runtime_projection(),
                review: ManualPlanningBootstrapReview {
                    draft_name: "simple-draft".to_string(),
                    staged_file_count: 2,
                    validation_report: PlanningValidationReport::default(),
                },
            },
        );
        assert_eq!(
            stale_review_app.shell.chrome.shell_overlay,
            ShellOverlay::Hidden
        );

        let mut stale_failure_app = make_test_app(&workspace);
        set_input(&mut stale_failure_app, "newer text");
        let current_correlation =
            arm_manual_prompt_preparation(&mut stale_failure_app, "newer text");
        let stale_correlation = different_manual_prompt_correlation(&current_correlation);
        let previous_status = ready_conversation(&stale_failure_app).status_text.clone();
        stale_failure_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::BootstrapFailed {
                correlation: stale_correlation,
                transcript_text: "older text".to_string(),
                runtime_projection: runtime_projection(),
                kind: ManualPlanningBootstrapFailureKind::Promote,
                reason: "should not surface".to_string(),
            },
        );
        assert_eq!(
            ready_conversation(&stale_failure_app).status_text,
            previous_status
        );
    }

    #[test]
    fn direct_manual_submission_rejects_a_prompt_above_the_input_limit() {
        let workspace = TempWorkspace::new("turn-submit-prompt-limit");
        let mut app = make_test_app(&workspace);

        app.submit_manual_prompt_from_text("x".repeat(MAX_PROMPT_INPUT_BYTES + 1));

        assert!(app.conversation.pending_manual_prompt_preparation.is_none());
        assert!(
            ready_conversation(&app)
                .status_text
                .contains("1048576-byte input limit")
        );
    }

    #[test]
    fn rapid_double_enter_after_serial_preparation_does_not_duplicate_submission() {
        let workspace = TempWorkspace::new("turn-submit-double-enter");
        let mut app = make_test_app(&workspace);
        set_input(&mut app, "ship it");

        app.submit_manual_prompt_from_text("ship it".to_string());
        poll_manual_prompt_preparation_completion(&mut app);

        app.start_turn_submission();

        assert!(app.conversation.pending_manual_prompt_preparation.is_none());
        assert_eq!(ready_conversation(&app).status_text, "starting turn");
    }

    #[test]
    fn turn_submission_admission_rejection_preserves_draft_then_retry_commits_once() {
        let workspace = TempWorkspace::new("turn-submit-core-rejection");
        let mut app = make_test_app(&workspace);
        set_input(&mut app, "second prompt");
        let previous_messages = ready_conversation(&app).messages.clone();
        let previous_status = ready_conversation(&app).status_text.clone();
        let active_correlation = app.runtime.client_runtime.begin_test_turn_submission();

        let admitted = app.submit_prompt_with_transcript(
            "second prompt".to_string(),
            "second prompt".to_string(),
            PromptOrigin::Manual,
        );

        let conversation = ready_conversation(&app);
        assert!(!admitted);
        assert_eq!(conversation.composer.input_buffer, "second prompt");
        assert_eq!(conversation.messages, previous_messages);
        assert_eq!(conversation.status_text, previous_status);
        assert_eq!(
            conversation.input_state(),
            ConversationInputState::SubmittingTurn
        );

        let _ = app.reduce_core_client_event(CoreInput::ConversationStreamUpdated {
            correlation: active_correlation,
            event: TurnStreamEvent::Failed {
                message: "first submission released".to_string(),
            },
        });
        let message_count_before_retry = ready_conversation(&app).messages.len();

        assert!(app.submit_prompt_with_transcript(
            "second prompt".to_string(),
            "second prompt".to_string(),
            PromptOrigin::Manual,
        ));

        let conversation = ready_conversation(&app);
        assert!(conversation.composer.input_buffer.is_empty());
        assert_eq!(conversation.messages.len(), message_count_before_retry + 1);
        assert_eq!(
            conversation
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some("second prompt")
        );
        assert_eq!(
            conversation.input_state(),
            ConversationInputState::SubmittingTurn
        );
    }

    #[test]
    fn manual_preparation_lock_prevents_a_b_a_input_replacement() {
        let workspace = TempWorkspace::new("turn-submit-input-aba");
        let mut app = make_test_app(&workspace);
        set_input(&mut app, "A");
        let correlation = arm_manual_prompt_preparation(&mut app, "A");

        app.dispatch_conversation_input(ConversationComposerEvent::TextInserted {
            text: "B".to_string(),
        });
        app.dispatch_conversation_input(ConversationComposerEvent::BackspacePressed);
        assert_eq!(ready_conversation(&app).composer.input_buffer, "A");
        assert_eq!(
            ready_conversation(&app).status_text,
            "turn preparation in progress; prompt editing is locked until it finishes"
        );

        app.apply_manual_prompt_preparation(rejected_manual_prompt_result(
            correlation,
            "A",
            "stale result",
        ));

        assert!(app.conversation.pending_manual_prompt_preparation.is_none());
        assert_eq!(
            ready_conversation(&app).status_text,
            "turn preparation failed / stale result"
        );
        assert_eq!(ready_conversation(&app).messages.len(), 1);
        assert!(ready_conversation(&app).composer.input_buffer.is_empty());
    }

    #[test]
    fn manual_preparation_result_is_dropped_after_workspace_switch() {
        let first_workspace = TempWorkspace::new("turn-submit-workspace-first");
        let second_workspace = TempWorkspace::new("turn-submit-workspace-second");
        let mut app = make_test_app(&first_workspace);
        set_input(&mut app, "ship it");
        let correlation = arm_manual_prompt_preparation(&mut app, "ship it");

        app.sync_draft_shell_workspace(second_workspace.path_str());
        let switched_status = ready_conversation(&app).status_text.clone();
        assert_eq!(
            app.planning_workspace_directory(),
            second_workspace.path_str()
        );
        assert!(app.conversation.pending_manual_prompt_preparation.is_none());

        app.dispatch_conversation_input(ConversationComposerEvent::TextInserted {
            text: " now".to_string(),
        });
        assert_eq!(
            ready_conversation(&app).composer.input_buffer,
            "ship it now"
        );
        assert!(
            !ready_conversation(&app)
                .status_text
                .contains("preparation in progress")
        );
        let replacement = arm_manual_prompt_preparation(&mut app, "ship it now");
        assert_ne!(replacement, correlation);
        assert_eq!(replacement.workspace_directory, second_workspace.path_str());

        app.apply_manual_prompt_preparation(rejected_manual_prompt_result(
            correlation,
            "ship it",
            "stale workspace",
        ));

        assert_eq!(
            app.conversation
                .pending_manual_prompt_preparation
                .as_ref()
                .map(|pending| &pending.correlation),
            Some(&replacement)
        );
        assert_eq!(ready_conversation(&app).status_text, switched_status);
        assert!(ready_conversation(&app).messages.is_empty());
        assert_eq!(
            ready_conversation(&app).composer.input_buffer,
            "ship it now"
        );
    }

    #[test]
    fn manual_preparation_completion_is_consumed_only_once() {
        let workspace = TempWorkspace::new("turn-submit-completion-once");
        let mut app = make_test_app(&workspace);
        set_input(&mut app, "ship it");
        let correlation = arm_manual_prompt_preparation(&mut app, "ship it");
        let result = rejected_manual_prompt_result(correlation, "ship it", "blocked");

        app.apply_manual_prompt_preparation(result.clone());
        let message_count = ready_conversation(&app).messages.len();
        let status_text = ready_conversation(&app).status_text.clone();
        app.apply_manual_prompt_preparation(result);

        assert_eq!(ready_conversation(&app).messages.len(), message_count);
        assert_eq!(ready_conversation(&app).status_text, status_text);
        assert!(app.conversation.pending_manual_prompt_preparation.is_none());
    }

    #[test]
    fn remaining_edges_cover_unready_empty_stale_handoffs_and_fake_port_methods() {
        let workspace = TempWorkspace::new("turn-submit-remaining-edges");
        let task = sample_handoff_task();

        let mut loading_queue_app = make_test_app(&workspace);
        loading_queue_app.conversation.lifecycle.conversation_state = ConversationState::Loading;
        loading_queue_app.resolve_startup_submit_queue();
        assert!(matches!(
            loading_queue_app.conversation.lifecycle.conversation_state,
            ConversationState::Loading
        ));

        let mut empty_prompt_app = make_test_app(&workspace);
        empty_prompt_app.submit_manual_prompt_from_text("   ".to_string());
        assert!(
            empty_prompt_app
                .conversation
                .pending_manual_prompt_preparation
                .is_none()
        );

        let mut loading_submit_app = make_test_app(&workspace);
        loading_submit_app.conversation.lifecycle.conversation_state = ConversationState::Loading;
        loading_submit_app.submit_manual_prompt_from_text("ship it".to_string());
        assert!(
            loading_submit_app
                .conversation
                .pending_manual_prompt_preparation
                .is_none()
        );

        let mut clear_pending_app = make_test_app(&workspace);
        set_input(&mut clear_pending_app, "reject me");
        let clear_pending_correlation =
            arm_manual_prompt_preparation(&mut clear_pending_app, "reject me");
        clear_pending_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::Rejected {
                correlation: clear_pending_correlation,
                transcript_text: "reject me".to_string(),
                runtime_projection: runtime_projection(),
                reason: "no task".to_string(),
            },
        );
        assert!(
            clear_pending_app
                .conversation
                .pending_manual_prompt_preparation
                .is_none()
        );

        let mut stale_handoff_app = make_test_app(&workspace);
        set_input(&mut stale_handoff_app, "outer");
        let stale_handoff_correlation =
            arm_manual_prompt_preparation(&mut stale_handoff_app, "outer");
        let previous_status = ready_conversation(&stale_handoff_app).status_text.clone();
        stale_handoff_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::PromptReady {
                correlation: stale_handoff_correlation,
                transcript_text: "outer".to_string(),
                runtime_projection: runtime_projection(),
                intake: Box::new(ManualPromptIntakeOutcome::TaskCommitted {
                    committed_task_id: task.task_id.clone(),
                    committed_planning_revision: 9,
                    handoff: handoff("wrapped", "inner", Some(task.clone())),
                }),
            },
        );
        assert_eq!(
            ready_conversation(&stale_handoff_app).status_text,
            previous_status
        );
        assert!(ready_conversation(&stale_handoff_app).messages.is_empty());

        let mut parallel_stale_handoff_app = make_test_app(&workspace);
        parallel_stale_handoff_app.set_parallel_mode_enabled_for_test(true);
        set_input(&mut parallel_stale_handoff_app, "outer parallel");
        let parallel_stale_handoff_correlation =
            arm_manual_prompt_preparation(&mut parallel_stale_handoff_app, "outer parallel");
        parallel_stale_handoff_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::PromptReady {
                correlation: parallel_stale_handoff_correlation,
                transcript_text: "outer parallel".to_string(),
                runtime_projection: runtime_projection(),
                intake: Box::new(ManualPromptIntakeOutcome::TaskUpdated {
                    updated_task_id: "task-stale".to_string(),
                    committed_planning_revision: 10,
                    handoff: handoff("wrapped", "inner parallel", Some(task)),
                }),
            },
        );
        let event_lines = parallel_stale_handoff_app
            .parallel_supervisor_event_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!event_lines.contains("updated task task-stale"));

        let mut loading_match_app = make_test_app(&workspace);
        set_input(&mut loading_match_app, "ship it");
        arm_manual_prompt_preparation(&mut loading_match_app, "ship it");
        let pending = loading_match_app
            .conversation
            .pending_manual_prompt_preparation
            .clone()
            .expect("preparation should be pending");
        loading_match_app.conversation.lifecycle.conversation_state = ConversationState::Loading;
        assert!(!loading_match_app.manual_prompt_preparation_is_current(&pending));

        let mut manual_submit_app = make_test_app(&workspace);
        set_input(&mut manual_submit_app, "manual prompt");
        assert!(
            manual_submit_app.submit_prompt("  manual prompt  ".to_string(), PromptOrigin::Manual,)
        );

        let mut intake_submit_app = make_test_app(&workspace);
        assert!(intake_submit_app.submit_prompt(
            "wrapped intake".to_string(),
            PromptOrigin::ManualIntake(Box::new(super::super::ManualIntakeSubmitContext {
                transcript_text: "operator transcript".to_string(),
                handoff_task: None,
                parallel_mode_enabled_at_submission: false,
            })),
        ));

        let port = FakeAppServerPort;
        assert!(port.load_startup_context().is_ok());
        assert!(
            port.load_session_catalog(SessionCatalogRequest::for_workspace(
                1,
                workspace.path_str().to_string()
            ))
            .is_ok()
        );
        assert_eq!(
            port.load_conversation_snapshot("thread-1")
                .expect("fake snapshot should load")
                .thread_id,
            "thread-1"
        );
        assert!(port.request_stop_all_sessions().is_ok());
        let (tx, _rx) =
            crate::application::port::conversation_stream::conversation_stream_channel();
        assert!(
            port.run_turn_stream("thread-1", "prompt", Default::default(), tx,)
                .is_ok()
        );
    }

    #[test]
    fn apply_manual_prompt_preparation_routes_manual_intake_outcomes() {
        let workspace = TempWorkspace::new("turn-submit-manual-intake");
        let task = sample_handoff_task();
        let mut committed_app = make_test_app(&workspace);
        set_input(&mut committed_app, "turn this into a task");
        let committed_correlation =
            arm_manual_prompt_preparation(&mut committed_app, "turn this into a task");

        committed_app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
            correlation: committed_correlation,
            transcript_text: "turn this into a task".to_string(),
            runtime_projection: runtime_projection(),
            intake: Box::new(ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id: task.task_id.clone(),
                committed_planning_revision: 7,
                handoff: handoff(
                    "wrapped task prompt",
                    "turn this into a task",
                    Some(task.clone()),
                ),
            }),
        });

        assert_eq!(
            ready_conversation(&committed_app).last_planning_task_handoff(),
            Some(&task)
        );
        assert_eq!(
            ready_conversation(&committed_app).status_text,
            "starting turn"
        );

        for outcome in [
            ManualPromptIntakeOutcome::Rejected {
                reason: "too small".to_string(),
            },
            ManualPromptIntakeOutcome::Failed {
                reason: "intake crashed".to_string(),
            },
        ] {
            let mut failure_app = make_test_app(&workspace);
            set_input(&mut failure_app, "make task");
            let failure_correlation = arm_manual_prompt_preparation(&mut failure_app, "make task");
            let expected_reason = match &outcome {
                ManualPromptIntakeOutcome::Rejected { reason }
                | ManualPromptIntakeOutcome::Failed { reason } => reason.clone(),
                _ => unreachable!("test only uses failure outcomes"),
            };

            failure_app.apply_manual_prompt_preparation(
                ManualPromptPreparationResult::PromptReady {
                    correlation: failure_correlation,
                    transcript_text: "make task".to_string(),
                    runtime_projection: runtime_projection(),
                    intake: Box::new(outcome),
                },
            );

            assert_eq!(
                ready_conversation(&failure_app).status_text,
                format!("turn preparation failed / {expected_reason}")
            );
            assert_eq!(
                ready_conversation(&failure_app)
                    .messages
                    .last()
                    .unwrap()
                    .text,
                "make task"
            );
        }
    }

    #[test]
    fn running_turn_manual_intake_queues_with_reversible_receipt_without_starting_a_turn() {
        let workspace = TempWorkspace::new("turn-submit-running-queue-only");
        let mut app = make_test_app(&workspace);
        let task = sample_handoff_task();
        install_running_core_turn(&mut app, "turn-running", workspace.path_str());
        ready_conversation_mut(&mut app).composer.input_buffer = "queue this follow-up".to_string();
        assert_eq!(
            manual_prompt_delivery(ready_conversation(&app)),
            Some(ManualPromptDelivery::QueueOnly)
        );
        let correlation = arm_manual_prompt_preparation(&mut app, "queue this follow-up");
        let pending = app
            .conversation
            .pending_manual_prompt_preparation
            .as_mut()
            .expect("manual intake should be armed");
        pending.delivery = ManualPromptDelivery::QueueOnly;
        pending.parent_turn_id = Some("turn-running".to_string());

        app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
            correlation,
            transcript_text: "queue this follow-up".to_string(),
            runtime_projection: runtime_projection(),
            intake: Box::new(ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id: task.task_id.clone(),
                committed_planning_revision: 12,
                handoff: handoff(
                    "wrapped prompt must not be submitted",
                    "queue this follow-up",
                    Some(task.clone()),
                ),
            }),
        });

        let conversation = ready_conversation(&app);
        assert_eq!(conversation.active_turn_id(), Some("turn-running"));
        assert!(conversation.composer.input_buffer.is_empty());
        assert_eq!(conversation.last_planning_task_handoff(), None);
        let receipt = conversation
            .latest_queue_mutation_receipt
            .as_ref()
            .expect("queue-only intake should expose an undo receipt");
        assert_eq!(receipt.completed_turn_id, "turn-running");
        assert_eq!(receipt.planning_revision, 12);
        assert_eq!(receipt.entries.len(), 1);
        assert_eq!(receipt.entries[0].task_id, task.task_id);
        assert!(receipt.created_batch_is_cancellable());

        set_input(&mut app, "update the queued task");
        let updated_correlation = arm_manual_prompt_preparation(&mut app, "update the queued task");
        let pending = app
            .conversation
            .pending_manual_prompt_preparation
            .as_mut()
            .expect("updated manual intake should be armed");
        pending.delivery = ManualPromptDelivery::QueueOnly;
        pending.parent_turn_id = Some("turn-running".to_string());
        app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
            correlation: updated_correlation,
            transcript_text: "update the queued task".to_string(),
            runtime_projection: runtime_projection(),
            intake: Box::new(ManualPromptIntakeOutcome::TaskUpdated {
                updated_task_id: task.task_id.clone(),
                committed_planning_revision: 13,
                handoff: handoff(
                    "updated prompt must not be submitted",
                    "update the queued task",
                    Some(task),
                ),
            }),
        });

        let conversation = ready_conversation(&app);
        assert!(conversation.latest_queue_mutation_receipt.is_none());
        assert!(!conversation.status_text.contains("undo available"));
    }

    #[test]
    fn running_turn_queue_failures_keep_the_draft_out_of_the_transcript() {
        let workspace = TempWorkspace::new("turn-submit-running-queue-failure");
        let make_running_app = || {
            let mut app = make_test_app(&workspace);
            install_running_core_turn(&mut app, "turn-running", workspace.path_str());
            ready_conversation_mut(&mut app).composer.input_buffer =
                "retry this exact draft".to_string();
            app
        };

        for outcome in [
            ManualPromptIntakeOutcome::Rejected {
                reason: "queue changed".to_string(),
            },
            ManualPromptIntakeOutcome::Failed {
                reason: "database unavailable".to_string(),
            },
        ] {
            let mut app = make_running_app();
            let original_message_count = ready_conversation(&app).messages.len();
            let correlation = arm_manual_prompt_preparation(&mut app, "retry this exact draft");
            app.conversation
                .pending_manual_prompt_preparation
                .as_mut()
                .expect("queue intake should be pending")
                .delivery = ManualPromptDelivery::QueueOnly;
            let reason = match &outcome {
                ManualPromptIntakeOutcome::Rejected { reason }
                | ManualPromptIntakeOutcome::Failed { reason } => reason.clone(),
                _ => unreachable!("fixture only contains failures"),
            };

            app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
                correlation,
                transcript_text: "retry this exact draft".to_string(),
                runtime_projection: runtime_projection(),
                intake: Box::new(outcome),
            });

            let conversation = ready_conversation(&app);
            assert_eq!(conversation.composer.input_buffer, "retry this exact draft");
            assert_eq!(conversation.messages.len(), original_message_count);
            assert_eq!(
                conversation.status_text,
                format!("queue preparation failed / {reason}; draft kept")
            );
            assert_eq!(conversation.active_turn_id(), Some("turn-running"));
        }

        let mut app = make_running_app();
        let original_message_count = ready_conversation(&app).messages.len();
        let correlation = arm_manual_prompt_preparation(&mut app, "retry this exact draft");
        app.conversation
            .pending_manual_prompt_preparation
            .as_mut()
            .expect("queue intake should be pending")
            .delivery = ManualPromptDelivery::QueueOnly;
        app.apply_manual_prompt_preparation(ManualPromptPreparationResult::Rejected {
            correlation,
            transcript_text: "retry this exact draft".to_string(),
            runtime_projection: runtime_projection(),
            reason: "planning rejected".to_string(),
        });

        let conversation = ready_conversation(&app);
        assert_eq!(conversation.composer.input_buffer, "retry this exact draft");
        assert_eq!(conversation.messages.len(), original_message_count);
        assert_eq!(
            conversation.status_text,
            "queue preparation failed / planning rejected; draft kept"
        );
        assert_eq!(conversation.active_turn_id(), Some("turn-running"));
    }

    #[test]
    fn manual_intake_covers_task_updates_parallel_failures_and_fallback_title() {
        let workspace = TempWorkspace::new("turn-submit-intake-edges");
        let task = sample_handoff_task();
        let mut updated_app = make_test_app(&workspace);
        set_input(&mut updated_app, "refresh task");
        let updated_correlation = arm_manual_prompt_preparation(&mut updated_app, "refresh task");

        updated_app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
            correlation: updated_correlation,
            transcript_text: "refresh task".to_string(),
            runtime_projection: runtime_projection(),
            intake: Box::new(ManualPromptIntakeOutcome::TaskUpdated {
                updated_task_id: task.task_id.clone(),
                committed_planning_revision: 8,
                handoff: handoff("wrapped update", "refresh task", Some(task.clone())),
            }),
        });

        assert_eq!(
            ready_conversation(&updated_app).last_planning_task_handoff(),
            Some(&task)
        );
        assert_eq!(
            ready_conversation(&updated_app).status_text,
            "starting turn"
        );

        let mut parallel_updated_app = make_test_app(&workspace);
        parallel_updated_app.set_parallel_mode_enabled_for_test(true);
        set_input(&mut parallel_updated_app, "parallel update");
        let parallel_updated_correlation =
            arm_manual_prompt_preparation(&mut parallel_updated_app, "parallel update");

        parallel_updated_app.apply_manual_prompt_preparation(
            ManualPromptPreparationResult::PromptReady {
                correlation: parallel_updated_correlation,
                transcript_text: "parallel update".to_string(),
                runtime_projection: runtime_projection(),
                intake: Box::new(ManualPromptIntakeOutcome::TaskUpdated {
                    updated_task_id: "task-2".to_string(),
                    committed_planning_revision: 11,
                    handoff: handoff("wrapped update", "parallel update", None),
                }),
            },
        );

        let event_lines = parallel_updated_app
            .parallel_supervisor_event_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(event_lines.contains("Task Intake: updated task task-2 / rev 11 / untitled task"));

        for outcome in [
            ManualPromptIntakeOutcome::Rejected {
                reason: "too broad".to_string(),
            },
            ManualPromptIntakeOutcome::Failed {
                reason: "worker crashed".to_string(),
            },
        ] {
            let mut failure_app = make_test_app(&workspace);
            failure_app.set_parallel_mode_enabled_for_test(true);
            let prompt = "parallel intake failure prompt with enough words to truncate in events";
            set_input(&mut failure_app, prompt);
            let failure_correlation = arm_manual_prompt_preparation(&mut failure_app, prompt);
            let expected_reason = match &outcome {
                ManualPromptIntakeOutcome::Rejected { reason }
                | ManualPromptIntakeOutcome::Failed { reason } => reason.clone(),
                _ => unreachable!("test only uses failure outcomes"),
            };

            failure_app.apply_manual_prompt_preparation(
                ManualPromptPreparationResult::PromptReady {
                    correlation: failure_correlation,
                    transcript_text: prompt.to_string(),
                    runtime_projection: runtime_projection(),
                    intake: Box::new(outcome),
                },
            );

            let conversation = ready_conversation(&failure_app);
            assert_eq!(conversation.composer.input_buffer, "");
            assert_eq!(
                conversation.status_text,
                format!("parallel task intake failed / {expected_reason}")
            );
            let event_lines = failure_app
                .parallel_supervisor_event_lines()
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(event_lines.contains("Task Intake: task generation failed"));
        }
    }

    #[test]
    fn parallel_manual_intake_stays_on_supervisor_layer_without_main_turn() {
        let workspace = TempWorkspace::new("turn-submit-parallel-manual-intake");
        let task = sample_handoff_task();
        let mut committed_app = make_test_app(&workspace);
        committed_app.set_parallel_mode_enabled_for_test(true);
        set_input(&mut committed_app, "안녕하세요 ?");
        let committed_correlation =
            arm_manual_prompt_preparation(&mut committed_app, "안녕하세요 ?");
        committed_app.set_parallel_mode_enabled_for_test(false);

        committed_app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
            correlation: committed_correlation,
            transcript_text: "안녕하세요 ?".to_string(),
            runtime_projection: runtime_projection(),
            intake: Box::new(ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id: task.task_id.clone(),
                committed_planning_revision: 7,
                handoff: handoff("wrapped task prompt", "안녕하세요 ?", Some(task.clone())),
            }),
        });

        let committed_conversation = ready_conversation(&committed_app);
        assert!(committed_conversation.messages.is_empty());
        assert_eq!(committed_conversation.composer.input_buffer, "");
        assert_eq!(committed_conversation.last_planning_task_handoff(), None);
        assert_ne!(committed_conversation.status_text, "starting turn");
        let event_lines = committed_app
            .parallel_supervisor_event_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(event_lines.contains("Task Intake: committed task task-1"));
        assert!(event_lines.contains("Orchestrator: task intake dispatch requested"));
    }

    #[test]
    fn normal_manual_intake_stays_on_main_turn_after_parallel_mode_toggle() {
        let workspace = TempWorkspace::new("turn-submit-normal-manual-intake");
        let task = sample_handoff_task();
        let mut committed_app = make_test_app(&workspace);
        set_input(&mut committed_app, "turn this into a task");
        let committed_correlation =
            arm_manual_prompt_preparation(&mut committed_app, "turn this into a task");
        committed_app.set_parallel_mode_enabled_for_test(true);

        committed_app.apply_manual_prompt_preparation(ManualPromptPreparationResult::PromptReady {
            correlation: committed_correlation,
            transcript_text: "turn this into a task".to_string(),
            runtime_projection: runtime_projection(),
            intake: Box::new(ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id: task.task_id.clone(),
                committed_planning_revision: 7,
                handoff: handoff(
                    "wrapped task prompt",
                    "turn this into a task",
                    Some(task.clone()),
                ),
            }),
        });

        let committed_conversation = ready_conversation(&committed_app);
        assert_eq!(
            committed_conversation
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some("turn this into a task")
        );
        assert_eq!(
            committed_conversation.last_planning_task_handoff(),
            Some(&task)
        );
        assert_eq!(committed_conversation.status_text, "starting turn");
        let event_lines = committed_app
            .parallel_supervisor_event_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!event_lines.contains("Task Intake: committed task task-1"));
        assert!(!event_lines.contains("Orchestrator: task intake dispatch requested"));
    }

    #[test]
    fn stale_queue_auto_prompt_is_rejected_without_leaving_manual_input_locked() {
        let workspace = TempWorkspace::new("turn-submit-auto-debug");
        let mut app = make_test_app(&workspace);
        let admitted =
            app.execute_conversation_runtime_effect(ConversationRuntimeEffect::QueueAutoPrompt {
                source: post_turn_source("turn-completed", workspace.path_str()),
                prompt: "continue task".to_string(),
                completed_turn_id: "turn-completed".to_string(),
                mode_label: "planning queue".to_string(),
                transcript_text: QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            });

        assert!(!admitted);
        let conversation = ready_conversation(&app);
        assert_eq!(conversation.status_text, "auto-follow cancelled");
        assert!(conversation.can_accept_manual_prompt());
        assert_eq!(
            conversation
                .last_auto_follow_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("auto-follow cancelled")
        );
        assert!(
            conversation
                .messages
                .iter()
                .all(|message| message.display_label.as_deref() != Some("Auto Follow-up"))
        );
    }

    #[test]
    fn terminal_before_steer_completion_rejects_premature_auto_follow_submission() {
        let workspace = TempWorkspace::new("turn-submit-steer-auto-race");
        let mut app = make_test_app(&workspace);
        let turn_submission = app.runtime.client_runtime.begin_test_turn_submission();
        app.dispatch_client_event(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Core runtime".to_string(),
                cwd: workspace.path_str().to_string(),
                runtime_envelope: Box::default(),
            },
        });
        app.dispatch_client_event(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        });
        set_input(&mut app, "steer this before completion");
        app.dispatch_client_event(CoreInput::Command(AppCommand::SetAutoFollowMaxTurns {
            value: 1,
        }));
        assert!(app.show_turn_steer_confirmation());
        assert!(
            app.handle_turn_steer_confirmation_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ))
        );
        assert!(app.conversation.pending_turn_steer.is_some());

        app.dispatch_client_event(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::TurnTerminal {
                receipt: crate::domain::turn_terminal::ConversationTurnTerminalReceipt::completed(
                    "thread-1",
                    "turn-1",
                    Vec::new(),
                )
                .with_application_delivery(
                    crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
                ),
                execution_snapshot_capture: None,
            },
        });
        let source = match app
            .runtime
            .client_runtime
            .snapshot()
            .conversation_runtime
            .post_turn
        {
            crate::core::app::PostTurnAuthoritySnapshot::Evaluating { correlation, .. } => {
                correlation
            }
            state => panic!("post-turn source should still be evaluating, got {state:?}"),
        };
        let admitted =
            app.execute_conversation_runtime_effect(ConversationRuntimeEffect::QueueAutoPrompt {
                source,
                prompt: "continue task".to_string(),
                completed_turn_id: "turn-1".to_string(),
                mode_label: "planning queue".to_string(),
                transcript_text: QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            });

        assert!(!admitted);
        assert!(app.conversation.pending_turn_steer.is_some());
        let conversation = ready_conversation(&app);
        assert_eq!(conversation.status_text, "auto-follow cancelled");
        assert!(
            conversation
                .messages
                .iter()
                .all(|message| message.display_label.as_deref() != Some("Auto Follow-up"))
        );
    }

    #[test]
    fn helper_functions_cover_slot_handoff_origins_truncation_and_debug_absence() {
        let workspace = TempWorkspace::new("turn-submit-helper-edges");
        let mut app = make_test_app(&workspace);
        let task = sample_handoff_task();
        app.set_parallel_mode_enabled_for_test(true);
        ready_conversation_mut(&mut app).replace_planning_handoff_for_test(Some(task.clone()));
        let source = post_turn_source("turn-1", workspace.path_str());
        let auto_origin = PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
            source: source.clone(),
            completed_turn_id: "turn-1".to_string(),
            mode_label: "planning queue".to_string(),
            transcript_text: QUEUED_TASK_TRANSCRIPT_TEXT.to_string(),
            debug_detail: None,
        }));

        let auto_request = app.build_turn_submission_request(
            workspace.path_str().to_string(),
            Some("thread-1".to_string()),
            "continue queued task".to_string(),
            &auto_origin,
        );
        assert_eq!(
            auto_request.slot_lease_handoff,
            Some(ParallelTurnSlotLeaseHandoff::new(
                task.task_id.clone(),
                task.task_title.clone(),
            ))
        );
        assert_eq!(auto_request.auto_follow_source, Some(source));

        let mut no_task_app = make_test_app(&workspace);
        no_task_app.set_parallel_mode_enabled_for_test(true);
        let no_task_request = no_task_app.build_turn_submission_request(
            workspace.path_str().to_string(),
            None,
            "manual prompt".to_string(),
            &PromptOrigin::Manual,
        );
        assert_eq!(no_task_request.slot_lease_handoff, None);

        no_task_app.conversation.lifecycle.conversation_state = ConversationState::Loading;
        let loading_request = no_task_app.build_turn_submission_request(
            workspace.path_str().to_string(),
            None,
            "manual prompt".to_string(),
            &PromptOrigin::Manual,
        );
        assert_eq!(loading_request.slot_lease_handoff, None);

        let manual_intake_origin =
            PromptOrigin::ManualIntake(Box::new(super::super::ManualIntakeSubmitContext {
                transcript_text: "operator text".to_string(),
                handoff_task: None,
                parallel_mode_enabled_at_submission: false,
            }));
        assert_eq!(prompt_origin_label(&manual_intake_origin), "ManualIntake");
        assert_eq!(prompt_origin_label(&auto_follow_origin()), "AutoFollow");
        assert_eq!(
            core_prompt_origin(&auto_follow_origin()),
            CorePromptOrigin::AutoFollow
        );
        assert!(prompt_origin_allows_parallel_slot_handoff(
            &auto_follow_origin(),
            true
        ));
        assert!(!prompt_origin_allows_parallel_slot_handoff(
            &auto_follow_origin(),
            false
        ));
        assert_eq!(truncate_parallel_prompt_event_text("  abc  ", 8), "abc");
        assert_eq!(
            truncate_parallel_prompt_event_text("abcdefghijklmnopqrstuvwxyz", 5),
            "ab..."
        );
        assert_eq!(
            truncate_parallel_prompt_event_text("abcdefghijklmnopqrstuvwxyz", 2),
            "..."
        );

        let mut lines = vec!["header".to_string()];
        append_debug_detail_preview_block(&mut lines, "body:", Some("   "));
        assert_eq!(lines, vec!["header".to_string()]);

        assert_eq!(
            app.build_auto_follow_transcript_debug_detail("ordinary prompt"),
            None
        );
        app.planning.planning_worker_visibility = PlanningWorkerVisibility::Debug;
        assert_eq!(
            app.build_auto_follow_transcript_debug_detail(QUEUED_TASK_TRANSCRIPT_TEXT),
            None
        );
        let mut planning_worker_panel_state =
            app.planning.planning_worker_panel_state.current().clone();
        planning_worker_panel_state.last_summary = Some("   ".to_string());
        app.planning
            .planning_worker_panel_state
            .replace_for_test(planning_worker_panel_state);
        let debug_detail = app
            .build_auto_follow_transcript_debug_detail(QUEUED_TASK_TRANSCRIPT_TEXT)
            .expect("blank summary still records the worker status line");
        assert!(debug_detail.contains("planning worker temporary session"));
        assert!(!debug_detail.contains("planning worker summary"));
    }

    #[test]
    fn build_turn_submission_request_maps_origin_and_parallel_slot_handoff() {
        let workspace = TempWorkspace::new("turn-submit-request");
        let mut app = make_test_app(&workspace);
        let task = sample_handoff_task();
        app.set_parallel_mode_enabled_for_test(true);
        app.conversation.turn_options.model = Some("gpt-5.4".to_string());
        app.conversation.turn_options.reasoning_effort = Some(ConversationReasoningEffort::High);

        let request = app.build_turn_submission_request(
            workspace.path_str().to_string(),
            Some("thread-1".to_string()),
            "wrapped task prompt".to_string(),
            &PromptOrigin::ManualIntake(Box::new(super::super::ManualIntakeSubmitContext {
                transcript_text: "operator text".to_string(),
                handoff_task: Some(task.clone()),
                parallel_mode_enabled_at_submission: true,
            })),
        );

        assert_eq!(request.workspace_directory, workspace.path_str());
        assert_eq!(request.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(request.prompt, "wrapped task prompt");
        assert_eq!(request.prompt_origin, CorePromptOrigin::ManualIntake);
        assert_eq!(request.planning_handoff, Some(task.clone()));
        assert_eq!(request.turn_options, app.conversation.turn_options);
        assert_eq!(
            request.slot_lease_handoff,
            Some(ParallelTurnSlotLeaseHandoff::new(
                task.task_id.clone(),
                task.task_title.clone(),
            ))
        );

        let normal_intake_request = app.build_turn_submission_request(
            workspace.path_str().to_string(),
            Some("thread-1".to_string()),
            "wrapped task prompt".to_string(),
            &PromptOrigin::ManualIntake(Box::new(super::super::ManualIntakeSubmitContext {
                transcript_text: "operator text".to_string(),
                handoff_task: Some(task.clone()),
                parallel_mode_enabled_at_submission: false,
            })),
        );

        assert_eq!(normal_intake_request.slot_lease_handoff, None);
        assert_eq!(normal_intake_request.planning_handoff, Some(task.clone()));

        app.set_parallel_mode_enabled_for_test(false);
        let manual_request = app.build_turn_submission_request(
            workspace.path_str().to_string(),
            None,
            "manual prompt".to_string(),
            &PromptOrigin::Manual,
        );

        assert_eq!(manual_request.prompt_origin, CorePromptOrigin::Manual);
        assert_eq!(manual_request.planning_handoff, None);
        assert_eq!(manual_request.slot_lease_handoff, None);
    }

    #[test]
    fn inline_model_picker_and_think_command_update_turn_options() {
        let workspace = TempWorkspace::new("turn-options-command");
        let mut app = make_test_app(&workspace);

        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":model gpt-5.4").expect("model command should parse"),
        );
        assert_eq!(
            app.conversation.turn_options.model.as_deref(),
            Some(ConversationTurnOptions::DEFAULT_MODEL)
        );
        assert_eq!(app.shell.chrome.shell_overlay, ShellOverlay::ModelSelection);
        assert!(
            ready_conversation(&app)
                .status_text
                .contains("ignored the typed argument")
        );
        app.handle_model_selection_overlay_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('5'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_model_selection_overlay_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('3'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":think high").expect("think command should parse"),
        );

        assert_eq!(
            app.conversation.turn_options.model.as_deref(),
            Some("gpt-5.4")
        );
        assert_eq!(
            app.conversation.turn_options.reasoning_effort,
            Some(ConversationReasoningEffort::High)
        );

        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":model default")
                .expect("model clear command should parse"),
        );
        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":think default")
                .expect("think clear command should parse"),
        );

        assert_eq!(app.conversation.turn_options.model, None);
        assert_eq!(app.conversation.turn_options.reasoning_effort, None);
    }

    #[test]
    fn inline_view_picker_and_argument_update_conversation_view_mode() {
        let workspace = TempWorkspace::new("view-mode-command");
        let mut app = make_test_app(&workspace);

        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":view").expect("view command should parse"),
        );
        assert_eq!(app.shell.chrome.shell_overlay, ShellOverlay::ViewSelection);
        app.handle_view_selection_overlay_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('3'),
            crossterm::event::KeyModifiers::NONE,
        ));

        assert_eq!(
            app.conversation.conversation_view_mode,
            ConversationViewMode::Detail
        );
        assert_eq!(app.shell.chrome.shell_overlay, ShellOverlay::Hidden);
        assert!(
            ready_conversation(&app)
                .status_text
                .contains("conversation view set to detail")
        );

        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":view midium").expect("view command should parse"),
        );

        assert_eq!(
            app.conversation.conversation_view_mode,
            ConversationViewMode::Medium
        );
    }

    #[test]
    fn inline_language_picker_and_argument_update_tui_language() {
        let workspace = TempWorkspace::new("language-command");
        let mut app = make_test_app(&workspace);

        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":language").expect("language command should parse"),
        );
        assert_eq!(
            app.shell.chrome.shell_overlay,
            ShellOverlay::LanguageSelection
        );
        app.handle_language_selection_overlay_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('1'),
            crossterm::event::KeyModifiers::NONE,
        ));

        assert_eq!(app.shell.tui_language, TuiLanguage::English);
        assert_eq!(app.shell.chrome.shell_overlay, ShellOverlay::Hidden);
        assert!(
            ready_conversation(&app)
                .status_text
                .contains("language set to English")
        );

        app.execute_inline_shell_command_input(
            InlineShellCommandInput::parse(":language 한국어")
                .expect("language command should parse"),
        );

        assert_eq!(app.shell.tui_language, TuiLanguage::Korean);
        assert!(
            ready_conversation(&app)
                .status_text
                .contains("언어가 한국어로 설정되었습니다.")
        );
    }

    #[test]
    fn active_turn_workspace_directory_is_read_from_core_projection() {
        let workspace = TempWorkspace::new("turn-submit-active-workspace");
        let mut app = make_test_app(&workspace);
        install_running_turn(
            ready_conversation_mut(&mut app),
            "turn-1",
            "/tmp/active-turn",
        );

        assert_eq!(
            ready_conversation(&app).active_turn_workspace_directory(),
            Some("/tmp/active-turn")
        );

        app.conversation.lifecycle.conversation_state = ConversationState::Loading;
        assert!(matches!(
            app.conversation.lifecycle.conversation_state,
            ConversationState::Loading
        ));
    }

    #[test]
    fn dispatch_operator_alert_effect_sends_background_message() {
        let workspace = TempWorkspace::new("turn-submit-operator-alert");
        let mut app = make_test_app(&workspace);
        let alert = OperatorAlert::planning_queue_drained();

        app.execute_conversation_runtime_effect(ConversationRuntimeEffect::DispatchOperatorAlert {
            alert: alert.clone(),
        });

        let message = app
            .runtime
            .rx
            .try_recv()
            .expect("operator alert should be queued for the runtime");
        assert!(matches!(
            message,
            BackgroundMessage::OperatorAlert(queued_alert) if queued_alert == alert
        ));
    }
}
