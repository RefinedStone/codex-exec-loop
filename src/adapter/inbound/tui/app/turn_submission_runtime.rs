#[path = "turn_submission_runtime/post_turn_execution.rs"]
mod post_turn_execution;
#[path = "turn_submission_runtime/stream_execution.rs"]
mod stream_execution;

use crate::application::service::planning_auto_follow_copy::BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT;
use post_turn_execution::PostTurnEvaluationRequest;
use stream_execution::PreparedTurnStreamRequest;

use super::planner_debug_preview::build_debug_preview_lines;
use super::*;

const AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES: usize = 32;

impl NativeTuiApp {
    pub(super) fn start_turn_submission(&mut self) {
        let inline_command = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                InlineShellCommandInput::parse(&conversation.input_buffer)
            }
            _ => None,
        };
        if let Some(command) = inline_command {
            self.execute_inline_shell_command_input(command);
            return;
        }

        let prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.can_submit_prompt() => {
                self.assemble_manual_prompt(conversation)
            }
            _ => return,
        };
        let Some(prompt) = prompt else {
            return;
        };

        self.submit_prompt(prompt, PromptOrigin::Manual);
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
            } => self.execute_start_stream(PreparedTurnStreamRequest {
                workspace_directory,
                thread_id,
                prompt,
            }),
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
                template_label,
                transcript_text,
                handoff_task,
            } => {
                let debug_detail = self.build_auto_follow_transcript_debug_detail(&transcript_text);
                self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(AutoFollowupSubmitContext {
                        queued_from_turn_id,
                        template_label,
                        transcript_text,
                        debug_detail,
                        handoff_task,
                    }),
                );
            }
        }
    }

    pub(super) fn resolve_startup_submit_queue(&mut self) {
        let (startup_submit_armed, prompt) = match &self.conversation_state {
            ConversationState::Ready(conversation) => (
                conversation.startup_submit_armed,
                self.assemble_manual_prompt(conversation),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };
        if !startup_submit_armed {
            return;
        }

        match self.shell_action_availability() {
            super::ShellActionAvailability::Ready if prompt.is_none() => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: None,
                });
            }
            super::ShellActionAvailability::Ready => {
                let prompt =
                    prompt.expect("ready startup submit should preserve a non-empty prompt");
                self.submit_prompt(prompt, PromptOrigin::Manual);
            }
            super::ShellActionAvailability::Pending => {}
            super::ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: Some(format!(
                        "{}; queued prompt kept in buffer",
                        self.submission_blocked_status(PromptOrigin::Manual)
                    )),
                });
            }
        }
    }

    pub(super) fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) {
        if matches!(prompt_origin, PromptOrigin::Manual)
            && matches!(
                self.shell_action_availability(),
                super::ShellActionAvailability::Pending
            )
        {
            self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            });
            return;
        }

        if !self.shell_action_availability().allows_actions() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.submission_blocked_status(prompt_origin),
            });
            return;
        }

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            origin: prompt_origin,
        });
    }

    fn assemble_manual_prompt(&self, conversation: &ConversationViewModel) -> Option<String> {
        self.planning_services.runtime_facade.build_manual_prompt(
            &conversation.input_buffer,
            &conversation.planning_runtime_snapshot,
        )
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
        if let Some(summary) = summary.filter(|summary| !summary.trim().is_empty()) {
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
