#[path = "turn_submission_runtime/post_turn_execution.rs"]
mod post_turn_execution;
#[path = "turn_submission_runtime/stream_execution.rs"]
mod stream_execution;

use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::{
    BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT, PlanningTaskHandoff,
};
use crate::domain::parallel_mode::ParallelModeSlotLeaseRequest;
use post_turn_execution::PostTurnEvaluationRequest;
use stream_execution::PreparedTurnStreamRequest;

use super::planner_debug_preview::build_debug_preview_lines;
use super::{
    AutoFollowupSubmitContext, ConversationInputEvent, ConversationRuntimeEffect,
    ConversationRuntimeEvent, ConversationState, InlineShellCommandInput, NativeTuiApp,
    PromptOrigin, ShellActionAvailability, ShellChromeEvent,
};

const AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES: usize = 32;

impl NativeTuiApp {
    fn parallel_mode_turn_service(&self) -> ParallelModeTurnService {
        ParallelModeTurnService::new(self.parallel_mode_service.clone())
    }

    pub(super) fn start_turn_submission(&mut self) {
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
            ConversationState::Ready(conversation) if conversation.can_submit_prompt() => {
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
        match effect {
            ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
            } => self.execute_start_stream(self.build_turn_stream_request(
                workspace_directory,
                thread_id,
                prompt,
            )),
            ConversationRuntimeEffect::EvaluateAutoFollowup {
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
            } => self.execute_post_turn_evaluation(PostTurnEvaluationRequest {
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
            }),
            ConversationRuntimeEffect::QueueAutoPrompt {
                prompt,
                queued_from_turn_id,
                mode_label,
                transcript_text,
                handoff_task,
            } => {
                let debug_detail = self.build_auto_follow_transcript_debug_detail(&transcript_text);
                let _ = self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(Box::new(AutoFollowupSubmitContext {
                        queued_from_turn_id,
                        mode_label,
                        transcript_text,
                        debug_detail,
                        handoff_task,
                    })),
                );
            }
        }
    }

    fn build_turn_stream_request(
        &self,
        workspace_directory: String,
        thread_id: Option<String>,
        prompt: String,
    ) -> PreparedTurnStreamRequest {
        PreparedTurnStreamRequest {
            workspace_directory,
            thread_id,
            prompt,
            slot_lease_request: self.build_parallel_mode_slot_lease_request(),
        }
    }

    fn build_parallel_mode_slot_lease_request(&self) -> Option<ParallelModeSlotLeaseRequest> {
        if !self.parallel_mode_enabled() {
            return None;
        }

        let ConversationState::Ready(conversation) = &self.conversation_state else {
            return None;
        };
        let handoff_task = conversation.last_planning_task_handoff()?;

        Some(parallel_mode_slot_lease_request(handoff_task))
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

        let prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) => self
                .planning
                .runtime
                .build_manual_prompt(&transcript_text, &conversation.planning_runtime_snapshot),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        };
        let Some(prompt) = prompt else {
            return;
        };

        let _ = self.submit_prompt_with_transcript(prompt, transcript_text, PromptOrigin::Manual);
    }

    pub(super) fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) -> bool {
        let transcript_text = match &prompt_origin {
            PromptOrigin::Manual => prompt.trim().to_string(),
            PromptOrigin::AutoFollow(context) => context.transcript_text.clone(),
            PromptOrigin::ParallelDispatch(context) => context.transcript_text.clone(),
        };
        self.submit_prompt_with_transcript(prompt, transcript_text, prompt_origin)
    }

    pub(super) fn submit_prompt_with_transcript(
        &mut self,
        prompt: String,
        transcript_text: String,
        prompt_origin: PromptOrigin,
    ) -> bool {
        if matches!(prompt_origin, PromptOrigin::Manual)
            && matches!(
                self.shell_action_availability(),
                ShellActionAvailability::Pending
            )
        {
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

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            transcript_text,
            origin: prompt_origin,
        })
    }

    fn ensure_manual_planning_workspace(&mut self, manual_prompt: &str) -> bool {
        let workspace_directory = self.planning_workspace_directory();
        let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        if snapshot.workspace_present() {
            self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                &workspace_directory,
            );
            return true;
        }

        if manual_prompt.trim().is_empty() {
            return false;
        }

        match self
            .planning
            .workspace
            .stage_simple_mode_draft(&workspace_directory)
        {
            Ok(stage_result) => {
                let draft_name = stage_result.draft_name.clone();
                match self
                    .planning
                    .workspace
                    .promote_staged_draft(&workspace_directory, &draft_name)
                {
                    Ok(result) => {
                        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
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
        if !self.planner_shows_debug_details()
            || transcript_text != BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT
        {
            return None;
        }

        let planner = &self.planner_worker_panel_state;
        let operation_label = planner.last_operation_label.as_deref().unwrap_or("unknown");
        let prompt = planner.last_prompt.as_deref();
        let response = planner.last_response.as_deref();
        let summary = planner.last_summary.as_deref();

        if prompt.is_none() && response.is_none() && summary.is_none() {
            return None;
        }

        let mut lines = vec![format!(
            "planner temp session: {operation_label} / {}",
            planner.status.label()
        )];
        if let Some(summary) = summary.filter(|summary: &&str| !summary.trim().is_empty()) {
            lines.push(format!("planner summary: {summary}"));
        }
        append_debug_detail_preview_block(&mut lines, "planner prompt:", prompt);
        append_debug_detail_preview_block(&mut lines, "planner response:", response);

        Some(lines.join("\n"))
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

fn parallel_mode_slot_lease_request(
    handoff_task: &PlanningTaskHandoff,
) -> ParallelModeSlotLeaseRequest {
    let task_id = handoff_task.task_id.trim();
    let task_title = handoff_task.task_title.trim();
    let common_slug = sanitize_parallel_mode_identifier(task_id)
        .or_else(|| sanitize_parallel_mode_identifier(task_title));
    let task_slug = common_slug.clone().unwrap_or_else(|| "task".to_string());
    let agent_slug = common_slug.unwrap_or_else(|| "agent".to_string());

    ParallelModeSlotLeaseRequest::new(
        task_id,
        task_title,
        format!("agent-{agent_slug}"),
        task_slug,
    )
}

fn sanitize_parallel_mode_identifier(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_was_dash = false;

    for character in input.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_was_dash = false;
            continue;
        }

        if !previous_was_dash && !slug.is_empty() {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_mode_slot_lease_request_uses_handoff_task_identity() {
        let request = parallel_mode_slot_lease_request(&PlanningTaskHandoff {
            task_id: "task-supersession-runtime".to_string(),
            task_title: "Wire runtime into slot lease lifecycle".to_string(),
            direction_id: "supersession-git-worktree-pool".to_string(),
            combined_priority: 96,
            updated_at: "2026-04-17T05:20:00Z".to_string(),
            status_label: "ready".to_string(),
        });

        assert_eq!(request.task_id, "task-supersession-runtime");
        assert_eq!(request.task_title, "Wire runtime into slot lease lifecycle");
        assert_eq!(request.agent_id, "agent-task-supersession-runtime");
        assert_eq!(request.task_slug, "task-supersession-runtime");
    }
}
