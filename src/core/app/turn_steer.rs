use super::TurnSubmissionCorrelation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnSteerCorrelation {
    pub generation: u64,
    pub turn_submission: TurnSubmissionCorrelation,
}

impl TurnSteerCorrelation {
    pub const fn new(generation: u64, turn_submission: TurnSubmissionCorrelation) -> Self {
        Self {
            generation,
            turn_submission,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSteerAdmission {
    Accepted {
        correlation: TurnSteerCorrelation,
    },
    RejectedActive {
        active_correlation: TurnSteerCorrelation,
    },
    RejectedUnavailable,
}
