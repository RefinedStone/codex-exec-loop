use super::TurnSubmissionCorrelation;
use crate::domain::conversation::ConversationApprovalDecision;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDecisionCorrelation {
    pub generation: u64,
    pub turn_submission: TurnSubmissionCorrelation,
    pub approval_id: String,
    pub decision: ConversationApprovalDecision,
}

impl ApprovalDecisionCorrelation {
    pub fn new(
        generation: u64,
        turn_submission: TurnSubmissionCorrelation,
        approval_id: impl Into<String>,
        decision: ConversationApprovalDecision,
    ) -> Self {
        Self {
            generation,
            turn_submission,
            approval_id: approval_id.into(),
            decision,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecisionAdmission {
    Accepted {
        correlation: ApprovalDecisionCorrelation,
    },
    RejectedActive {
        active_correlation: ApprovalDecisionCorrelation,
    },
    RejectedUnavailable,
}
