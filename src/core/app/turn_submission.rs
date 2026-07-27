use crate::domain::conversation::ConversationTurnOptions;
use crate::domain::planning::{ParallelTurnHandoff, TaskHandoff};

use super::{PostTurnEvaluationCorrelation, StopRequestCorrelation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnSubmissionCorrelation {
    pub generation: u64,
}

impl TurnSubmissionCorrelation {
    pub const fn new(generation: u64) -> Self {
        Self { generation }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSubmissionAdmission {
    Accepted {
        correlation: TurnSubmissionCorrelation,
    },
    RejectedActive {
        active_correlation: TurnSubmissionCorrelation,
    },
    RejectedStopPending {
        stop_correlation: StopRequestCorrelation,
    },
    RejectedUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorePromptOrigin {
    Manual,
    ManualIntake,
    AutoFollow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSubmissionRequest {
    pub workspace_directory: String,
    pub thread_id: Option<String>,
    pub prompt: String,
    pub prompt_origin: CorePromptOrigin,
    pub auto_follow_source: Option<PostTurnEvaluationCorrelation>,
    /*
     * Manual-intake handoff identity enters Core with the turn intent. Plain
     * manual turns must not carry one, while auto-follow derives its handoff
     * only from the exact Core-owned post-turn route.
     */
    pub planning_handoff: Option<TaskHandoff>,
    pub turn_options: ConversationTurnOptions,
    pub slot_lease_handoff: Option<ParallelTurnHandoff>,
}

impl TurnSubmissionRequest {
    pub(crate) fn request_label(&self) -> &'static str {
        if self.thread_id.is_some() {
            "turn stream"
        } else {
            "new-thread stream"
        }
    }
}
