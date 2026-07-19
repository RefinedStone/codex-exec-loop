use super::TurnSubmissionCorrelation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopRequestCorrelation {
    pub generation: u64,
    pub turn_submission: Option<TurnSubmissionCorrelation>,
}

impl StopRequestCorrelation {
    pub const fn new(generation: u64, turn_submission: Option<TurnSubmissionCorrelation>) -> Self {
        Self {
            generation,
            turn_submission,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRequestAttempt {
    Initial,
    AfterTurnStarted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRequestAdmission {
    Accepted {
        correlation: StopRequestCorrelation,
    },
    RejectedActive {
        active_correlation: StopRequestCorrelation,
    },
}
