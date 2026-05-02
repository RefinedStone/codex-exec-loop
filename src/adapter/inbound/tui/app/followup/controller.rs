// 학습 주석: live activity pulse는 현재 시각과 turn 시작 시각의 차이를 표시하기 때문에 Instant를 사용합니다.
// wall-clock 시간이 아니라 monotonic duration을 써서 시스템 시간이 바뀌어도 pulse가 뒤틀리지 않습니다.
use std::time::Instant;

// 학습 주석: follow-up overlay의 max auto turns editor는 raw terminal key를 직접 받습니다.
// 여기서는 Enter/Esc/Ctrl-C/Backspace/문자 입력을 UI event로 번역합니다.
use crossterm::event::{self, KeyCode, KeyModifiers};

// 학습 주석: 이 controller는 NativeTuiApp 내부 상태를 읽고, follow-up control event와 overlay UI event를
// dispatch하는 얇은 adapter입니다. 실제 state mutation은 reducer 쪽으로 넘겨 입력 처리와 상태 변경을 분리합니다.
use super::super::{
    ConversationState, DEFAULT_AUTO_FOLLOW_MAX_TURNS, FollowupControlEvent, FollowupOverlayUiEvent,
    NativeTuiApp, PlanningInitOverlayStep, ShellOverlay,
};

impl NativeTuiApp {
    pub(crate) fn pause_post_turn_continuation(&mut self) {
        // 학습 주석: auto-follow 일시정지는 UI local flag가 아니라 follow-up control event입니다.
        // reducer를 거치게 해야 현재 turn 이후 이어쓰기 여부와 footer copy가 같은 상태를 공유합니다.
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowPaused);
    }

    pub(crate) fn current_max_auto_turns_label(&self) -> String {
        // 학습 주석: max auto turns 값은 ready conversation의 auto_follow_state가 source of truth입니다.
        // 아직 conversation이 없으면 editor 초기값으로 repo 기본값을 표시해 startup 상태에서도 copy가 안정적입니다.
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.auto_follow_state.max_auto_turns_label()
            }
            ConversationState::Loading | ConversationState::Failed(_) => {
                DEFAULT_AUTO_FOLLOW_MAX_TURNS.to_string()
            }
        }
    }

    pub(crate) fn planner_shows_debug_details(&self) -> bool {
        // 학습 주석: shell presentation은 planner_visibility 내부 enum을 알 필요 없이, debug detail을
        // 보여도 되는지만 묻습니다. 이 helper가 rendering layer의 조건문을 작게 유지합니다.
        self.planner_visibility.shows_debug_details()
    }

    pub(crate) fn live_activity_pulse(&self, now: Instant) -> Option<u64> {
        // 학습 주석: live activity pulse는 ready conversation에서 active live activity가 있을 때만 footer에
        // 표시됩니다. loading/failed 상태는 transcript runtime이 없으므로 pulse row를 숨깁니다.
        match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                // 학습 주석: conversation model이 live activity 시작 시각을 알고 있으며, 없으면 None이 전파됩니다.
                .live_activity_started_at()
                // 학습 주석: saturating_duration_since로 now가 started_at보다 앞서는 비정상 입력도 0초로 낮춥니다.
                .map(|started_at| now.saturating_duration_since(started_at).as_secs()),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        }
    }

    pub(crate) fn is_max_auto_turns_editing(&self) -> bool {
        // 학습 주석: editor 활성 여부는 overlay UI state에만 있습니다. conversation state의 실제
        // max_auto_turns 값과 분리해, 사용자가 입력 중인 buffer를 저장 전까지 격리합니다.
        self.followup_overlay_ui_state
            .max_auto_turns_editor
            .is_editing
    }

    pub(crate) fn start_max_auto_turns_edit(&mut self) {
        // 학습 주석: 실제 auto-follow 설정은 ready conversation에만 적용됩니다. loading/failed 상태에서
        // editor를 열면 저장할 대상이 없어지므로 입력 시작을 무시합니다.
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        // 학습 주석: max auto turns editor는 planning init의 SimpleReview 단계 안에 있는 inline control입니다.
        // 다른 overlay나 다른 step에서 키가 들어오면 해당 화면의 입력 계약을 침범하지 않도록 무시합니다.
        if self.shell_overlay != ShellOverlay::PlanningInit
            || self.planning_init_overlay_ui_state.step() != PlanningInitOverlayStep::SimpleReview
        {
            return;
        }

        // 학습 주석: edit start는 overlay UI event입니다. 현재 실제 값을 buffer로 복사해 사용자가
        // 취소해도 conversation auto_follow_state는 그대로 남게 합니다.
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsEditStarted {
            current_value: self.current_max_auto_turns_label(),
        });
    }

    pub(crate) fn save_max_auto_turns_edit(&mut self) {
        // 학습 주석: 저장은 editor가 열려 있을 때만 의미가 있습니다. 닫힌 상태의 Enter 입력이
        // 실제 auto-follow 설정을 덮어쓰지 않도록 방어합니다.
        if !self.is_max_auto_turns_editing() {
            return;
        }

        // 학습 주석: 저장 시점에는 UI buffer를 control event로 넘깁니다. reducer가 값 검증과
        // conversation state 갱신을 담당해 controller가 parsing 규칙을 복제하지 않습니다.
        self.dispatch_followup_controls(FollowupControlEvent::MaxAutoTurnsUpdated {
            value: self
                .followup_overlay_ui_state
                .max_auto_turns_editor
                .buffer
                .clone(),
        });
    }

    pub(crate) fn cancel_max_auto_turns_edit(&mut self) {
        // 학습 주석: cancel은 실제 설정을 되돌리는 동작이 아니라 editor buffer를 현재 설정값으로 재동기화하고
        // 편집 상태를 닫는 UI event입니다.
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsEditCanceled {
            current_value: self.current_max_auto_turns_label(),
        });
    }

    pub(crate) fn push_max_auto_turns_character(&mut self, character: char) {
        // 학습 주석: 문자 입력은 바로 파싱하지 않고 UI buffer에만 반영합니다. 저장 시점까지
        // invalid intermediate state를 허용해야 일반 텍스트 editor처럼 동작합니다.
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsCharacterTyped {
            character,
        });
    }

    pub(crate) fn pop_max_auto_turns_character(&mut self) {
        // 학습 주석: Backspace도 overlay UI state만 수정합니다. 실제 max_auto_turns 값은 save event가
        // 발생하기 전까지 유지됩니다.
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsBackspacePressed);
    }

    pub(crate) fn handle_max_auto_turns_editor_key(&mut self, key: event::KeyEvent) -> bool {
        // 학습 주석: 이 handler의 반환값은 "키를 editor가 소비했는가"입니다. editor가 닫혀 있으면
        // 상위 key router가 일반 shell/planning 단축키로 처리할 수 있게 false를 반환합니다.
        if !self.is_max_auto_turns_editing() {
            return false;
        }

        // 학습 주석: 편집 중이라도 화면이 SimpleReview에서 벗어났다면 더 이상 이 editor가 키를
        // 소유하면 안 됩니다. overlay 전환 중 stale editing flag가 남아도 입력 탈취를 막습니다.
        let editor_supported = self.shell_overlay == ShellOverlay::PlanningInit
            && self.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::SimpleReview;
        if !editor_supported {
            return false;
        }

        // 학습 주석: 여기부터는 editor가 키를 소유합니다. 지원하지 않는 키도 true를 반환해
        // 바깥 overlay가 같은 키를 두 번 해석하지 않게 합니다.
        match key.code {
            // 학습 주석: modifier 없는 Enter만 저장으로 인정해 Shift/Alt 조합이 의도치 않게 설정을 확정하지 않게 합니다.
            KeyCode::Enter if key.modifiers.is_empty() => self.save_max_auto_turns_edit(),
            // 학습 주석: Esc는 일반적인 inline editor 취소 동작입니다.
            KeyCode::Esc => self.cancel_max_auto_turns_edit(),
            // 학습 주석: Ctrl-C도 terminal UI에서 취소로 해석해 사용자가 editor에 갇히지 않게 합니다.
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_max_auto_turns_edit()
            }
            // 학습 주석: Backspace는 buffer에서 마지막 문자를 제거합니다.
            KeyCode::Backspace => self.pop_max_auto_turns_character(),
            // 학습 주석: max turns label은 숫자뿐 아니라 "infinite" 같은 identifier-like 값을 허용하므로
            // ASCII alphanumeric 입력을 받습니다. Shift는 대문자 입력을 위해 허용합니다.
            KeyCode::Char(character)
                if (key.modifiers == KeyModifiers::NONE
                    || key.modifiers == KeyModifiers::SHIFT)
                    && character.is_ascii_alphanumeric() =>
            {
                self.push_max_auto_turns_character(character);
            }
            // 학습 주석: 화살표나 punctuation 같은 입력은 현재 editor가 지원하지 않지만, 이미 editor
            // 모드이므로 상위 router로 흘리지 않고 소비 처리합니다.
            _ => {}
        }

        true
    }
}
