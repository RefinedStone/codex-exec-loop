use crate::adapter::inbound::tui::conversation_text::interrupt_blocked_status_text;
use crate::domain::conversation::ConversationControlSupport;
use crate::domain::session_summary::SessionSummary;

/*
Conversation intent reducer는 TUI key handler와 app runtime 사이의 navigation
policy 경계다. Shell controller나 session overlay는 "새 draft", "session open",
"Ctrl-C" 같은 operator intent만 전달하고, 여기서 현재 conversation mode와 running
turn 여부를 합쳐 실제 effect를 결정한다.

이 분리를 두면 key 입력 경로가 session lifecycle이나 draft 초기화를 직접 호출하지
않는다. Running turn 보호, blank draft exit confirmation, failed-screen recovery 같은
정책은 이 reducer에 모이고, app_runtime은 effect executor로 남는다.
*/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConversationIntentMode {
    // Loading은 아직 안정적인 conversation surface가 없어 Ctrl-C를 navigation으로 해석하지 않는다.
    Loading,
    // Failed는 recovery surface다. Ctrl-C는 종료가 아니라 새 draft로 빠지는 탈출구가 된다.
    Failed,
    // BlankDraft에서는 더 지울 conversation이 없으므로 Ctrl-C를 exit confirmation으로 보낸다.
    BlankDraft,
    // Ready는 기존 conversation surface다. Ctrl-C는 새 draft를 여는 빠른 navigation intent다.
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConversationIntentState {
    // Running turn은 session switch와 new draft를 모두 막는 최상위 guard다.
    // 현재 stream, pending completion, auto-follow state가 navigation으로 끊기면 안 된다.
    pub has_running_turn: bool,
    // Mode는 Ctrl-C 의미를 loading/recovery/blank/ready surface별로 나누는 최소 state다.
    pub mode: ConversationIntentMode,
    // Running turn 중 Ctrl-C는 runtime truth에 맞는 안내로만 낮춘다.
    // 이 reducer는 interrupt command를 만들지 않아 control support와 navigation을 섞지 않는다.
    pub interrupt_support: ConversationControlSupport,
}

#[derive(Debug, Clone)]
pub(super) enum ConversationIntentEvent {
    // Shell controller의 새 대화 요청이다. Guard를 통과하면 lifecycle draft effect로 바뀐다.
    NewDraftRequested,
    // Session list selection은 선택이 없을 수 있다. `None`은 overlay cursor no-op로 유지한다.
    SessionOpenRequested {
        session: Option<Box<SessionSummary>>,
    },
    // Ctrl-C는 interrupt status, new draft, exit confirmation으로 갈라지는 overloaded key다.
    CtrlCPressed,
}

#[derive(Debug, Clone)]
pub(super) enum ConversationIntentEffect {
    // Status effect는 navigation을 거부한 이유를 conversation input reducer에 남긴다.
    ShowStatus { status_text: String },
    // Draft opening은 app_runtime이 lifecycle, shell chrome, auto-follow overlay reset으로 확장한다.
    OpenNewDraft,
    // Session opening은 summary ownership만 넘긴다. Snapshot load는 lifecycle effect가 맡는다.
    OpenSession { session: SessionSummary },
    // Blank draft Ctrl-C는 shell chrome의 exit confirmation overlay로만 이어진다.
    ShowExitConfirmation,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationIntentReduction {
    // Effect vector는 한 intent가 나중에 여러 UI side effect로 확장되어도 call site를 유지한다.
    pub effects: Vec<ConversationIntentEffect>,
}

pub(super) fn reduce_conversation_intents(
    state: ConversationIntentState,
    event: ConversationIntentEvent,
) -> ConversationIntentReduction {
    let mut effects = Vec::new();

    match event {
        ConversationIntentEvent::NewDraftRequested => {
            // 새 draft는 current turn의 stream과 post-turn action을 버릴 수 있어 running 중에는 막는다.
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
            // Session switch도 current turn을 잃게 만드는 navigation이다. 선택 없음은 no-op다.
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
            // 실행 중 Ctrl-C는 navigation이 아니라 interrupt capability 안내다.
            // 실제 interrupt 제어가 없는 runtime에서도 같은 키가 안전하게 동작해야 한다.
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
        // Running turn guard는 새 draft navigation보다 우선해 shell controller가 draft effect를 받지 않는다.
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
        // Blank draft Ctrl-C는 conversation cleanup이 아니라 exit confirmation으로 제한된다.
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
        // Failed mode는 recovery 화면이므로 Ctrl-C를 새 draft 전환으로 해석한다.
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
        // 선택 없는 session overlay 요청은 cursor state일 뿐이라 conversation effect를 만들지 않는다.
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
        // Reducer는 Ctrl-C를 interrupt command가 아니라 runtime capability 안내로 낮춘다.
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
