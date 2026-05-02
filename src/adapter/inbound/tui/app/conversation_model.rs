// 학습 주석: conversation model test는 화면 줄 formatting 결과까지 검증해야 하므로, production
// module에서는 숨겨진 shell presentation helper를 test build에서만 이 model namespace로 끌어옵니다.
#[cfg(test)]
pub(super) use super::shell_presentation::format_conversation_lines;
pub(super) use super::{
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD,
    INFINITE_AUTO_FOLLOW_MAX_TURNS, INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN,
};
// 학습 주석: 테스트 fixture는 domain conversation message를 직접 만들어 view model mapping을
// 검증합니다. runtime build에는 필요 없는 import라 test cfg 안에만 노출합니다.
#[cfg(test)]
pub(super) use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

// 학습 주석: auto_follow module은 사용자가 한 번 요청한 뒤 agent가 자동으로 이어서 턴을 실행할지
// 판단하는 상태와 정책을 담습니다. path attribute는 파일을 `conversation_model/` 아래에 두면서
// 이 index module의 하위 module로 연결합니다.
#[path = "conversation_model/auto_follow.rs"]
mod auto_follow;
// 학습 주석: turn_activity module은 현재 턴이 streaming 중인지, 완료됐는지, follow-up 판단 중인지
// 같은 runtime activity를 표현합니다. conversation UI가 메시지 목록과 별도로 표시할 상태입니다.
#[path = "conversation_model/turn_activity.rs"]
mod turn_activity;
// 학습 주석: view_model module은 domain conversation, 입력창, planning repair 상태를 TUI가 바로
// 그릴 수 있는 `ConversationViewModel` 계층으로 묶습니다.
#[path = "conversation_model/view_model.rs"]
mod view_model;

// 학습 주석: auto-follow 관련 type은 conversation shell과 tests가 함께 쓰는 runtime state surface입니다.
// 이 re-export 덕분에 caller는 하위 파일 구조 대신 `conversation_model::AutoFollowState`처럼
// conversation model 경계만 의존합니다.
pub(crate) use auto_follow::{
    AutoFollowRuntimePhase, AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason,
    StopKeywordRule,
};
// 학습 주석: `TurnActivityState`는 현재 tests에서만 직접 assertion합니다. production UI는 더 높은
// 수준의 view model re-export를 통해 이 상태를 간접적으로 다룹니다.
#[cfg(test)]
pub(crate) use turn_activity::TurnActivityState;
// 학습 주석: view model re-export는 TUI app이 conversation state, input state, repair state를
// 한 module surface에서 가져오게 합니다. mapping logic은 하위 module에 남기고 type contract만
// 여기서 공개합니다.
pub(crate) use view_model::{
    ConversationInputState, ConversationState, ConversationViewModel, PlanningRepairState,
};

// 학습 주석: conversation model tests는 runtime binary에는 필요 없으므로 test build에서만 compile합니다.
#[cfg(test)]
// 학습 주석: tests file을 sibling `conversation_model_tests.rs`로 빼 두어 이 index file은 module wiring과
// public surface 설명에 집중하고, detailed behavior assertions는 별도 파일에 둡니다.
#[path = "conversation_model_tests.rs"]
mod tests;
