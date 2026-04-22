#[cfg(test)]
pub(super) use super::shell_presentation::format_conversation_lines;
pub(super) use super::{
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD,
    INFINITE_AUTO_FOLLOW_MAX_TURNS, INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN,
};
#[cfg(test)]
pub(super) use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

#[path = "conversation_model/auto_follow.rs"]
mod auto_follow;
#[path = "conversation_model/turn_activity.rs"]
mod turn_activity;
#[path = "conversation_model/view_model.rs"]
mod view_model;

pub(crate) use auto_follow::{
    AutoFollowRuntimePhase, AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason,
    StopKeywordRule,
};
#[cfg(test)]
pub(crate) use turn_activity::TurnActivityState;
pub(crate) use view_model::{
    ConversationInputState, ConversationState, ConversationViewModel, PlanningRepairState,
};

#[cfg(test)]
#[path = "conversation_model_tests.rs"]
mod tests;
