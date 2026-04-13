#[path = "turn_submission_runtime/post_turn_execution.rs"]
mod post_turn_execution;
#[path = "turn_submission_runtime/stream_execution.rs"]
mod stream_execution;

use post_turn_execution::PostTurnEvaluationRequest;
use stream_execution::PreparedTurnStreamRequest;

use super::*;

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
                self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(AutoFollowupSubmitContext {
                        queued_from_turn_id,
                        template_label,
                        transcript_text,
                        handoff_task,
                    }),
                );
            }
            ConversationRuntimeEffect::QueuePlanningRepairPrompt {
                prompt,
                queued_from_turn_id,
                attempt_number,
                max_attempts,
            } => {
                self.submit_prompt(
                    prompt,
                    PromptOrigin::PlanningRepair(PlanningRepairSubmitContext {
                        queued_from_turn_id,
                        attempt_number,
                        max_attempts,
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
}
