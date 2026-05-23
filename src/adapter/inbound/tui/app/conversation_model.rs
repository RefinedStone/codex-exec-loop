pub(super) use super::{
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD,
    INFINITE_AUTO_FOLLOW_MAX_TURNS, INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN,
};
// 테스트 fixture는 domain message를 직접 만들어 shell runtime을 띄우지 않고도
// view model mapping 회귀를 좁게 확인한다.
#[cfg(test)]
pub(super) use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

// 이 index는 conversation UI 상태를 하나의 module boundary 뒤에 묶고, 구현은
// follow-up 정책, 현재 turn activity, renderer-facing view model로 나눠 둔다.
#[path = "conversation_model/auto_follow.rs"]
mod auto_follow;
#[path = "conversation_model/turn_activity.rs"]
mod turn_activity;
#[path = "conversation_model/view_model.rs"]
mod view_model;

// auto-follow 상태는 shell input handling과 테스트가 함께 쓰므로, 호출부는
// policy 파일 배치가 아니라 conversation model surface에만 의존한다.
#[cfg(test)]
pub(crate) use auto_follow::AutoFollowDecision;
pub(crate) use auto_follow::{
    AutoFollowRuntimePhase, AutoFollowSkipReason, AutoFollowState, StopKeywordRule,
};
// shell은 conversation state, input state, planning-repair state를 이 surface에서
// 가져오고, 실제 mapping logic은 `view_model.rs` 안에 남긴다.
#[cfg(test)]
pub(crate) use view_model::RecordedAutoFollowActivity;
pub(crate) use view_model::{
    ConversationInputState, ConversationState, ConversationViewModel, PlanningRepairState,
};

#[cfg(test)]
#[path = "conversation_model_tests.rs"]
mod tests;
