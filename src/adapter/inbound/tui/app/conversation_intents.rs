use crate::adapter::inbound::tui::conversation_text::interrupt_blocked_status_text;
use crate::domain::conversation::ConversationControlSupport;
use crate::domain::session_summary::SessionSummary;

/*
학습 주석: conversation intent reducer는 TUI key handler와 app runtime 사이의 작은 정책 계층입니다.
shell controller나 session overlay는 "새 draft", "session open", "Ctrl-C" 같은 사용자의 의도만
전달하고, 여기서 현재 conversation 상태와 실행 중 turn 여부를 합쳐 실제 effect를 결정합니다.

이 분리를 두면 key 입력 경로가 바로 session을 바꾸거나 draft를 열지 않아도 되며, running turn
보호 규칙을 한곳에서 유지할 수 있습니다.
*/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConversationIntentMode {
    // 학습 주석: Loading은 아직 열 수 있는 conversation surface가 안정되지 않은 상태라 Ctrl-C를
    // draft 전환이나 종료 확인으로 해석하지 않습니다.
    Loading,
    // 학습 주석: Failed는 현재 shell/session surface가 복구 화면에 머문 상태입니다. Ctrl-C는
    // 앱 종료가 아니라 새 draft로 빠지는 recovery affordance가 됩니다.
    Failed,
    // 학습 주석: BlankDraft는 사용자가 이미 비어 있는 작성 화면에 있으므로 Ctrl-C를 종료 확인으로
    // 연결해 accidental exit를 막습니다.
    BlankDraft,
    // 학습 주석: Ready는 기존 conversation을 보고 있는 정상 상태입니다. Ctrl-C는 새 draft를
    // 여는 빠른 navigation intent로 쓰입니다.
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConversationIntentState {
    // 학습 주석: running turn은 session switch와 new draft를 모두 막는 최상위 guard입니다.
    // 현재 conversation의 출력 stream과 follow-up 상태가 끊기면 안 되기 때문입니다.
    pub has_running_turn: bool,
    // 학습 주석: mode는 화면 복구/빈 draft/준비 상태별로 Ctrl-C 의미를 다르게 해석하는
    // 최소한의 presentation state입니다.
    pub mode: ConversationIntentMode,
    // 학습 주석: running turn 중 Ctrl-C는 실제 interrupt 가능 여부를 runtime truth에 맞춰
    // status text로만 노출합니다. 이 reducer가 interrupt command 자체를 발행하지 않는 이유입니다.
    pub interrupt_support: ConversationControlSupport,
}

#[derive(Debug, Clone)]
pub(super) enum ConversationIntentEvent {
    // 학습 주석: shell controller의 새 대화 요청입니다. reducer는 running turn guard를 먼저
    // 적용한 뒤 runtime에게 draft opening effect만 넘깁니다.
    NewDraftRequested,
    // 학습 주석: session list selection은 선택이 없을 수도 있습니다. `None`은 no-op로 남겨
    // overlay navigation과 conversation navigation을 억지로 결합하지 않습니다.
    SessionOpenRequested {
        session: Option<Box<SessionSummary>>,
    },
    // 학습 주석: Ctrl-C는 상태에 따라 interrupt status, new draft, exit confirmation으로
    // 갈라지는 overloaded key라 reducer 경계에서 명시적으로 정책화합니다.
    CtrlCPressed,
}

#[derive(Debug, Clone)]
pub(super) enum ConversationIntentEffect {
    // 학습 주석: status는 navigation을 거부한 이유를 operator에게 남기는 side effect입니다.
    ShowStatus { status_text: String },
    // 학습 주석: app_runtime이 실제 draft state 초기화와 shell view 전환을 담당합니다.
    OpenNewDraft,
    // 학습 주석: reducer는 SessionSummary ownership만 넘기고, session loading과 UI state 갱신은
    // runtime의 effect executor에 맡깁니다.
    OpenSession { session: SessionSummary },
    // 학습 주석: 빈 draft에서 Ctrl-C가 곧장 앱 종료로 이어지지 않도록 confirmation overlay를 엽니다.
    ShowExitConfirmation,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationIntentReduction {
    // 학습 주석: effect vector를 쓰면 하나의 intent가 이후 여러 UI side effect로 확장되어도
    // controller call site를 바꾸지 않고 reducer contract를 유지할 수 있습니다.
    pub effects: Vec<ConversationIntentEffect>,
}

pub(super) fn reduce_conversation_intents(
    state: ConversationIntentState,
    event: ConversationIntentEvent,
) -> ConversationIntentReduction {
    let mut effects = Vec::new();

    match event {
        ConversationIntentEvent::NewDraftRequested => {
            // 학습 주석: 새 draft는 기존 turn의 stream, pending completion, 후속 action을 끊을 수 있어
            // running 중에는 실제 전환 대신 설명 status만 냅니다.
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
            // 학습 주석: session switch도 running turn을 잃게 만드는 navigation입니다. 선택이 없는
            // 요청은 list cursor 상태만 의미하므로 effect 없이 흘려보냅니다.
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
            // 학습 주석: 실행 중 Ctrl-C는 "navigation"이 아니라 "interrupt 가능 여부 안내"입니다.
            // 실제 interrupt 제어가 없는 runtime에서도 같은 키가 안전하게 동작해야 합니다.
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
        // 학습 주석: running turn guard는 새 draft navigation보다 우선합니다. 이 테스트는
        // shell controller가 실수로 draft를 여는 effect를 받지 않는지 고정합니다.
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
        // 학습 주석: 빈 draft에서 Ctrl-C는 conversation cleanup이 아니라 앱 종료 가능성이 있어
        // runtime이 confirmation overlay를 열도록 effect를 제한합니다.
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
        // 학습 주석: Failed mode는 shell recovery 화면이므로 Ctrl-C를 종료가 아니라 새 draft
        // 전환으로 해석해 operator가 막힌 화면에서 빠져나오게 합니다.
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
        // 학습 주석: session overlay에서 선택이 없으면 conversation runtime에 아무 effect도
        // 전달하지 않습니다. no-op를 명시해 overlay cursor 상태와 session open을 분리합니다.
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
        // 학습 주석: reducer는 Ctrl-C를 interrupt command로 실행하지 않고 runtime capability에
        // 맞는 안내 문구로 낮춥니다. 이 테스트가 unsupported runtime 문구를 고정합니다.
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
