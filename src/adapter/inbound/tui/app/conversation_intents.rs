use crate::adapter::inbound::tui::conversation_text::interrupt_blocked_status_text;
use crate::domain::conversation::ConversationControlSupport;
use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConversationIntentMode {
    Loading,
    Failed,
    BlankDraft,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConversationIntentState {
    pub has_running_turn: bool,
    pub mode: ConversationIntentMode,
    pub interrupt_support: ConversationControlSupport,
}

#[derive(Debug, Clone)]
pub(super) enum ConversationIntentEvent {
    NewDraftRequested,
    SessionOpenRequested {
        session: Option<Box<SessionSummary>>,
    },
    CtrlCPressed,
}

#[derive(Debug, Clone)]
pub(super) enum ConversationIntentEffect {
    ShowStatus { status_text: String },
    OpenNewDraft,
    OpenSession { session: SessionSummary },
    ShowExitConfirmation,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationIntentReduction {
    pub effects: Vec<ConversationIntentEffect>,
}

pub(super) fn reduce_conversation_intents(
    state: ConversationIntentState,
    event: ConversationIntentEvent,
) -> ConversationIntentReduction {
    let mut effects = Vec::new();

    match event {
        ConversationIntentEvent::NewDraftRequested => {
            if state.has_running_turn {
                effects.push(ConversationIntentEffect::ShowStatus {
                    status_text:
                        "turn still running; wait for completion before starting a new draft"
                            .to_string(),
                });
            } else {
                effects.push(ConversationIntentEffect::OpenNewDraft);
            }
        }
        ConversationIntentEvent::SessionOpenRequested { session } => {
            if state.has_running_turn {
                effects.push(ConversationIntentEffect::ShowStatus {
                    status_text:
                        "turn still running; wait for completion before switching sessions"
                            .to_string(),
                });
            } else if let Some(session) = session {
                effects.push(ConversationIntentEffect::OpenSession { session: *session });
            }
        }
        ConversationIntentEvent::CtrlCPressed => {
            if state.has_running_turn {
                effects.push(ConversationIntentEffect::ShowStatus {
                    status_text: interrupt_blocked_status_text(state.interrupt_support),
                });
            } else {
                match state.mode {
                    ConversationIntentMode::Ready | ConversationIntentMode::Failed => {
                        effects.push(ConversationIntentEffect::OpenNewDraft);
                    }
                    ConversationIntentMode::BlankDraft => {
                        effects.push(ConversationIntentEffect::ShowExitConfirmation);
                    }
                    ConversationIntentMode::Loading => {}
                }
            }
        }
    }

    ConversationIntentReduction { effects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::ConversationControlSupport;

    #[test]
    fn new_draft_requested_while_running_turn_only_shows_status() {
        let reduced = reduce_conversation_intents(
            ConversationIntentState {
                has_running_turn: true,
                mode: ConversationIntentMode::Ready,
                interrupt_support: ConversationControlSupport::Unsupported,
            },
            ConversationIntentEvent::NewDraftRequested,
        );

        assert_eq!(reduced.effects.len(), 1);
        assert!(matches!(
            reduced.effects.first(),
            Some(ConversationIntentEffect::ShowStatus { status_text })
                if status_text
                    == "turn still running; wait for completion before starting a new draft"
        ));
    }

    #[test]
    fn ctrl_c_from_blank_draft_shows_exit_confirmation() {
        let reduced = reduce_conversation_intents(
            ConversationIntentState {
                has_running_turn: false,
                mode: ConversationIntentMode::BlankDraft,
                interrupt_support: ConversationControlSupport::Unsupported,
            },
            ConversationIntentEvent::CtrlCPressed,
        );

        assert!(matches!(
            reduced.effects.as_slice(),
            [ConversationIntentEffect::ShowExitConfirmation]
        ));
    }

    #[test]
    fn ctrl_c_from_failed_shell_opens_new_draft() {
        let reduced = reduce_conversation_intents(
            ConversationIntentState {
                has_running_turn: false,
                mode: ConversationIntentMode::Failed,
                interrupt_support: ConversationControlSupport::Unsupported,
            },
            ConversationIntentEvent::CtrlCPressed,
        );

        assert!(matches!(
            reduced.effects.as_slice(),
            [ConversationIntentEffect::OpenNewDraft]
        ));
    }

    #[test]
    fn session_open_requested_without_selection_emits_no_effect() {
        let reduced = reduce_conversation_intents(
            ConversationIntentState {
                has_running_turn: false,
                mode: ConversationIntentMode::Ready,
                interrupt_support: ConversationControlSupport::Unsupported,
            },
            ConversationIntentEvent::SessionOpenRequested { session: None },
        );

        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn ctrl_c_while_turn_runs_surfaces_interrupt_truth() {
        let reduced = reduce_conversation_intents(
            ConversationIntentState {
                has_running_turn: true,
                mode: ConversationIntentMode::Ready,
                interrupt_support: ConversationControlSupport::Unsupported,
            },
            ConversationIntentEvent::CtrlCPressed,
        );

        assert!(matches!(
            reduced.effects.as_slice(),
            [ConversationIntentEffect::ShowStatus { status_text }]
                if status_text
                    == "turn still running; this runtime does not expose interrupt control in the shell"
        ));
    }
}
