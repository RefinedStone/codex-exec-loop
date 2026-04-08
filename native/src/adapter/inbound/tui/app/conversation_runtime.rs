use super::*;
use crate::domain::conversation::ConversationToolActivityKind;

#[derive(Debug, Clone)]
pub(super) enum ConversationRuntimeEvent {
    SubmitPrompt {
        prompt: String,
        origin: PromptOrigin,
    },
    StreamUpdated(ConversationStreamEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConversationRuntimeEffect {
    StartStream {
        cwd: String,
        thread_id: Option<String>,
        prompt: String,
    },
    QueueAutoPrompt {
        prompt: String,
        queued_from_turn_id: String,
        template_label: String,
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
            let cwd = state.cwd.clone();
            match &origin {
                PromptOrigin::Manual => {
                    state.auto_follow_state.reset_for_manual_turn();
                    state.clear_auto_followup_skip();
                }
                PromptOrigin::AutoFollow(context) => {
                    state.auto_follow_state.mark_auto_turn_submitted();
                    state.record_auto_followup_submission(
                        &context.queued_from_turn_id,
                        &context.template_label,
                    );
                }
            }
            let auto_follow_progress = state.auto_follow_state.progress_label();
            state.messages.push(ConversationMessage::new(
                ConversationMessageKind::User,
                prompt.clone(),
                None,
                None,
            ));
            state.refresh_conversation_lines();
            state.input_buffer.clear();
            state.mark_turn_submitting();
            state.status_text = match origin {
                PromptOrigin::Manual => "starting turn".to_string(),
                PromptOrigin::AutoFollow(context) => format!(
                    "auto follow-up submitted / turn {auto_follow_progress} / queued from {} / template: {}",
                    context.queued_from_turn_id, context.template_label
                ),
            };
            effects.push(ConversationRuntimeEffect::StartStream {
                cwd,
                thread_id,
                prompt,
            });
        }
        ConversationRuntimeEvent::StreamUpdated(event) => {
            let mut should_refresh_lines = false;

            match event {
                ConversationStreamEvent::ThreadPrepared {
                    thread_id,
                    title,
                    cwd,
                } => {
                    state.thread_id = thread_id;
                    state.title = title;
                    state.cwd = cwd;
                    state.status_text = "thread started".to_string();
                }
                ConversationStreamEvent::TurnStarted { turn_id } => {
                    state.mark_turn_started(turn_id);
                    state.status_text = "turn started".to_string();
                }
                ConversationStreamEvent::StatusUpdated { text } => {
                    state.status_text = text;
                }
                ConversationStreamEvent::AgentMessageDelta {
                    item_id,
                    phase,
                    delta,
                } => {
                    push_agent_delta(&mut state.messages, item_id, phase, delta);
                    should_refresh_lines = true;
                }
                ConversationStreamEvent::AgentMessageCompleted {
                    item_id,
                    phase,
                    text,
                } => {
                    complete_agent_message(&mut state.messages, item_id, phase, text);
                    should_refresh_lines = true;
                }
                ConversationStreamEvent::ToolActivity { activity } => {
                    if activity.kind == ConversationToolActivityKind::FileChange {
                        state
                            .turn_activity
                            .register_file_change(activity.file_change_count);
                    }
                    state.messages.push(ConversationMessage::new(
                        ConversationMessageKind::Tool,
                        activity.text,
                        None,
                        None,
                    ));
                    should_refresh_lines = true;
                }
                ConversationStreamEvent::TurnCompleted { turn_id } => {
                    state.turn_activity.complete_turn();
                    state.mark_turn_finished();
                    match state.decide_auto_followup() {
                        AutoFollowupDecision::QueuePrompt(prompt) => {
                            state.clear_auto_followup_skip();
                            let template_label =
                                state.auto_follow_state.template_label().to_string();
                            state.record_auto_followup_queue(&turn_id, &template_label);
                            state.status_text = format!(
                                "turn completed: {turn_id} / queued auto follow-up with template {template_label}"
                            );
                            effects.push(ConversationRuntimeEffect::QueueAutoPrompt {
                                prompt,
                                queued_from_turn_id: turn_id,
                                template_label,
                            });
                        }
                        AutoFollowupDecision::Skip(skip_reason) => {
                            state.record_auto_followup_skip(skip_reason);
                            state.status_text =
                                skip_reason.runtime_status(&turn_id, &state.auto_follow_state);
                        }
                    }
                }
                ConversationStreamEvent::Failed { message } => {
                    state.mark_turn_finished();
                    state.status_text = "turn failed".to_string();
                    state.messages.push(ConversationMessage::new(
                        ConversationMessageKind::Status,
                        message,
                        None,
                        None,
                    ));
                    should_refresh_lines = true;
                }
            }

            if should_refresh_lines {
                state.refresh_conversation_lines();
            }
        }
    }

    ConversationRuntimeReduction { state, effects }
}

fn push_agent_delta(
    messages: &mut Vec<ConversationMessage>,
    item_id: String,
    phase: Option<String>,
    delta: String,
) {
    if let Some(message) = find_message_by_item_id_mut(messages, &item_id) {
        message.text.push_str(&delta);
        if phase.is_some() {
            message.phase = phase;
        }
        return;
    }

    messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        delta,
        phase,
        Some(item_id),
    ));
}

fn complete_agent_message(
    messages: &mut Vec<ConversationMessage>,
    item_id: String,
    phase: Option<String>,
    text: String,
) {
    if let Some(message) = find_message_by_item_id_mut(messages, &item_id) {
        message.text = text;
        message.phase = phase;
        return;
    }

    messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        text,
        phase,
        Some(item_id),
    ));
}

fn find_message_by_item_id_mut<'a>(
    messages: &'a mut [ConversationMessage],
    item_id: &str,
) -> Option<&'a mut ConversationMessage> {
    messages
        .iter_mut()
        .rev()
        .find(|message| message.item_id.as_deref() == Some(item_id))
}

#[cfg(test)]
mod tests {
    use super::*;
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
                cwd: "/tmp/workspace".to_string(),
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
                }),
            },
        );

        assert_eq!(
            reduced.state.status_text,
            "auto follow-up submitted / turn 1/3 / queued from turn-1 / template: builtin next-task"
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
            Some("queued after turn turn-1 completed; submitted with template builtin next-task")
        );
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
            }),
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::ReadyToContinue
        );
        assert_eq!(
            reduced.state.status_text,
            "turn completed: turn-1 / queued auto follow-up with template builtin next-task"
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("queued auto turn 1/3")
        );
        assert!(reduced.state.last_auto_followup_skip.is_none());
        assert!(matches!(
            reduced.effects.as_slice(),
            [ConversationRuntimeEffect::QueueAutoPrompt { .. }]
        ));
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
    fn turn_completed_records_no_agent_reply_skip_when_auto_followup_cannot_continue() {
        let state = sample_active_turn_conversation();

        let reduced = reduce_conversation_runtime(
            state,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
            }),
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::ReadyToContinue
        );
        assert_eq!(
            reduced.state.status_text,
            "turn completed: turn-1 / auto follow-up skipped: no agent reply"
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("skipped: no agent reply")
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_skip
                .as_ref()
                .map(|skip| skip.reason),
            Some(AutoFollowupSkipReason::NoAgentReply)
        );
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn turn_completed_records_no_file_change_skip_when_rule_is_enabled() {
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
            }),
        );

        assert_eq!(
            reduced.state.input_state,
            ConversationInputState::ReadyToContinue
        );
        assert_eq!(
            reduced.state.status_text,
            "turn completed: turn-1 / auto follow-up stopped: no file changes"
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("stopped: no file changes")
        );
        assert_eq!(
            reduced
                .state
                .last_auto_followup_skip
                .as_ref()
                .map(|skip| skip.reason),
            Some(AutoFollowupSkipReason::NoFileChanges)
        );
        assert!(reduced.effects.is_empty());
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

    fn sample_conversation() -> ConversationViewModel {
        ConversationViewModel {
            thread_id: "thread-1".to_string(),
            title: "Existing session".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            cached_conversation_lines: format_conversation_lines(&[]),
            base_warnings: Vec::new(),
            template_warnings: Vec::new(),
            warnings: Vec::new(),
            input_buffer: "ship it".to_string(),
            active_turn_id: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(FollowupTemplateCatalog {
                items: vec![FollowupTemplateDefinition {
                    id: "builtin-next-task".to_string(),
                    label: "builtin next-task".to_string(),
                    body: "follow up {auto_turn}/{max_auto_turns}\n{last_message}".to_string(),
                    source: FollowupTemplateSource::Builtin,
                }],
            }),
            turn_activity: TurnActivityState::default(),
            last_auto_followup_activity: None,
            last_auto_followup_skip: None,
            status_text: "thread loaded".to_string(),
        }
    }

    fn sample_active_turn_conversation() -> ConversationViewModel {
        let mut state = sample_conversation();
        state.input_buffer.clear();
        state.input_state = ConversationInputState::StreamingTurn;
        state.active_turn_id = Some("turn-1".to_string());
        state
    }
}
