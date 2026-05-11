/* Turn submission is the execution layer for ConversationRuntimeEffect. The
 * reducer decides what should happen; this module gates the prompt against shell
 * readiness, sends stream startup and post-turn planning evaluation through the
 * core runtime, and re-enters the reducer for auto-follow prompts.
 */
#[path = "turn_submission_runtime/post_turn_execution.rs"]
mod post_turn_execution;

use crate::application::service::parallel_mode::turn::ParallelTurnSlotLeaseHandoff;
use crate::application::service::planning::{
    ManualPromptIntakeOutcome, ManualPromptIntakeRequest, QUEUED_TASK_TRANSCRIPT_TEXT,
};
use crate::core::app::{AppCommand, CorePromptOrigin, TurnSubmissionRequest};
use post_turn_execution::PostTurnEvaluationRequest;

use super::planning_worker_debug_preview::build_debug_preview_lines;
use super::{
    AutoFollowSubmitContext, ConversationInputEvent, ConversationRuntimeEffect,
    ConversationRuntimeEvent, ConversationState, InlineShellCommandInput,
    ManualIntakeSubmitContext, NativeTuiApp, PromptOrigin, ShellActionAvailability,
    ShellChromeEvent,
};

const AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES: usize = 32;

impl NativeTuiApp {
    pub(super) fn start_turn_submission(&mut self) {
        // Enter first belongs to inline shell commands. Only non-command prompt
        // text becomes a conversation turn, and only when the current conversation
        // can accept a manual prompt.
        let inline_command = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                InlineShellCommandInput::parse(&conversation.input_buffer)
            }
            _ => None,
        };
        if let Some(command) = inline_command {
            if self.should_queue_task_intake_command(&command) {
                self.queue_task_intake_command_until_idle(command);
                return;
            }
            self.execute_inline_shell_command_input(command);
            return;
        }
        let operator_prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.can_accept_manual_prompt() => {
                conversation.input_buffer.clone()
            }
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
    ) {
        // This switchboard is intentionally thin: stream work and post-turn planning
        // live in submodules, while auto-follow reuses the same submit path as a
        // manual prompt with a different origin.
        match effect {
            ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
                prompt_origin,
            } => self.dispatch_core_command(AppCommand::SubmitTurn(
                self.build_turn_submission_request(
                    workspace_directory,
                    thread_id,
                    prompt,
                    &prompt_origin,
                ),
            )),
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
                prompt,
                completed_turn_id,
                mode_label,
                transcript_text,
                handoff_task,
            } => {
                let debug_detail = self.build_auto_follow_transcript_debug_detail(&transcript_text);
                let _ = self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
                        completed_turn_id,
                        mode_label,
                        transcript_text,
                        debug_detail,
                        handoff_task,
                    })),
                );
            }
            ConversationRuntimeEffect::DispatchOperatorAlert { alert } => {
                let _ = self.tx.send(super::BackgroundMessage::OperatorAlert(alert));
            }
        }
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
            slot_lease_handoff: self.build_parallel_mode_slot_lease_handoff(),
        }
    }

    fn build_parallel_mode_slot_lease_handoff(&self) -> Option<ParallelTurnSlotLeaseHandoff> {
        // A slot lease needs a concrete planning handoff so the parallel pool can
        // bind cleanup ownership. Application/domain code owns the lease request and
        // slug policy; the TUI only forwards task identity.
        if !self.parallel_mode_enabled() {
            return None;
        }
        let ConversationState::Ready(conversation) = &self.conversation_state else {
            return None;
        };
        let handoff_task = conversation.last_planning_task_handoff()?;

        Some(ParallelTurnSlotLeaseHandoff::new(
            handoff_task.task_id.clone(),
            handoff_task.task_title.clone(),
        ))
    }

    pub(super) fn sync_active_turn_workspace_directory(&mut self, workspace_directory: &str) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };

        conversation.replace_active_turn_workspace_directory(workspace_directory.to_string());
        self.conversation_state = ConversationState::ready(conversation);
    }

    pub(super) fn resolve_startup_submit_queue(&mut self) {
        let (startup_submit_armed, operator_prompt) = match &self.conversation_state {
            ConversationState::Ready(conversation) => (
                conversation.startup_submit_armed,
                conversation.input_buffer.clone(),
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
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: None,
                });
            }
            ShellActionAvailability::Ready => {
                self.submit_manual_prompt_from_text(operator_prompt);
            }
            ShellActionAvailability::Pending => {}
            ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: Some(format!(
                        "{}; queued prompt kept in buffer",
                        self.submission_blocked_status(PromptOrigin::Manual)
                    )),
                });
            }
        }
    }

    pub(super) fn submit_manual_prompt_from_text(&mut self, operator_prompt: String) {
        let transcript_text = operator_prompt.trim().to_string();
        if transcript_text.is_empty() {
            return;
        }
        if !self.ensure_manual_planning_workspace(&transcript_text) {
            return;
        }

        let workspace_directory = self.planning_workspace_directory();
        let (parent_thread_id, parent_turn_id) = match &self.conversation_state {
            ConversationState::Ready(conversation) => (
                Some(conversation.thread_id.clone())
                    .filter(|thread_id| !thread_id.trim().is_empty()),
                conversation.active_turn_id.clone(),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => (None, None),
        };
        match self
            .application
            .planning()
            .runtime()
            .prepare_manual_prompt_intake(ManualPromptIntakeRequest {
                workspace_directory,
                raw_prompt: transcript_text.clone(),
                legacy_source_turn_id: None,
                parent_thread_id,
                parent_turn_id,
            }) {
            ManualPromptIntakeOutcome::NoTaskNeeded(handoff) => {
                let _ = self.submit_prompt_with_transcript(
                    handoff.prompt,
                    handoff.transcript_text,
                    PromptOrigin::Manual,
                );
            }
            ManualPromptIntakeOutcome::TaskCommitted { handoff, .. }
            | ManualPromptIntakeOutcome::TaskUpdated { handoff, .. } => {
                let _ = self.submit_prompt_with_transcript(
                    handoff.prompt,
                    handoff.transcript_text.clone(),
                    PromptOrigin::ManualIntake(Box::new(ManualIntakeSubmitContext {
                        transcript_text: handoff.transcript_text,
                        handoff_task: handoff.task,
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
            self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
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
            transcript_text = transcript_text,
            transcript_text_len = transcript_text.len(),
            prompt = prompt,
            prompt_len = prompt.len(),
            parallel_mode_enabled = self.parallel_mode_enabled(),
        );

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            transcript_text,
            origin: prompt_origin,
        })
    }

    fn ensure_manual_planning_workspace(&mut self, manual_prompt: &str) -> bool {
        let workspace_directory = self.planning_workspace_directory();
        let runtime_projection = self.load_planning_runtime_projection(&workspace_directory);
        if runtime_projection.workspace_present() {
            self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
                &workspace_directory,
            );
            return true;
        }
        if manual_prompt.trim().is_empty() {
            return false;
        }

        // First-use simple mode creates and immediately promotes a default planning
        // scaffold. Validation failures open the review overlay instead of silently
        // submitting a prompt against missing planning files.
        match self
            .application
            .planning()
            .workspace()
            .stage_simple_mode_draft(&workspace_directory)
        {
            Ok(stage_result) => {
                let draft_name = stage_result.draft_name.clone();
                match self
                    .application
                    .planning()
                    .workspace()
                    .promote_staged_draft(&workspace_directory, &draft_name)
                {
                    Ok(result) => {
                        self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
                            &workspace_directory,
                        );
                        if result.promoted_file_count > 0 {
                            true
                        } else {
                            self.planning_init_overlay_ui_state
                                .open_simple_review(stage_result);
                            self.planning_draft_editor_ui_state.reset();
                            self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
                            self.dispatch_conversation_input(
                                ConversationInputEvent::StatusMessageShown {
                                    status_text: format!(
                                        "planning bootstrap promote blocked / draft: {} / validation needs attention",
                                        result.draft_name
                                    ),
                                },
                            );
                            false
                        }
                    }
                    Err(error) => {
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text: format!("planning bootstrap promote failed: {error}"),
                            },
                        );
                        false
                    }
                }
            }
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("planning bootstrap failed: {error}"),
                });
                false
            }
        }
    }

    fn build_auto_follow_transcript_debug_detail(&self, transcript_text: &str) -> Option<String> {
        if !self.planning_worker_shows_debug_details()
            || transcript_text != QUEUED_TASK_TRANSCRIPT_TEXT
        {
            return None;
        }
        let planning_worker = &self.planning_worker_panel_state;
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
            planning_worker.status.label()
        )];
        if let Some(summary) = summary.filter(|summary: &&str| !summary.trim().is_empty()) {
            lines.push(format!("planning worker summary: {summary}"));
        }
        append_debug_detail_preview_block(&mut lines, "planning worker prompt:", prompt);
        append_debug_detail_preview_block(&mut lines, "planning worker response:", response);

        Some(lines.join("\n"))
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
        "transcript_text": transcript_text,
        "transcript_text_len": transcript_text.len(),
        "prompt": prompt,
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
    fn user_prompt_submit_detail_keeps_operator_text_and_final_prompt() {
        let detail = user_prompt_submit_detail(
            "final wrapper\noperator text",
            "operator text",
            &PromptOrigin::Manual,
            true,
        );

        assert_eq!(detail["origin"], "Manual");
        assert_eq!(detail["transcript_text"], "operator text");
        assert_eq!(detail["transcript_text_len"], 13);
        assert_eq!(detail["prompt"], "final wrapper\noperator text");
        assert_eq!(detail["prompt_len"], 27);
        assert_eq!(detail["parallel_mode_enabled"], true);
    }

    #[test]
    fn prompt_origin_maps_to_core_origin_without_tui_context() {
        assert_eq!(
            core_prompt_origin(&PromptOrigin::Manual),
            CorePromptOrigin::Manual
        );
    }
}
