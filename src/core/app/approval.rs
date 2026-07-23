use super::TurnSubmissionCorrelation;
use crate::domain::conversation::{ConversationApprovalDecision, ConversationApprovalReview};
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalReviewPersistenceCorrelation {
    pub generation: u64,
    pub turn_submission: TurnSubmissionCorrelation,
    pub workspace_directory: String,
    pub thread_id: String,
    pub review: ConversationApprovalReview,
}

impl ApprovalReviewPersistenceCorrelation {
    pub fn new(
        generation: u64,
        turn_submission: TurnSubmissionCorrelation,
        workspace_directory: impl Into<String>,
        thread_id: impl Into<String>,
        review: ConversationApprovalReview,
    ) -> Self {
        Self {
            generation,
            turn_submission,
            workspace_directory: workspace_directory.into(),
            thread_id: thread_id.into(),
            review,
        }
    }

    fn matches_update(
        &self,
        turn_submission: TurnSubmissionCorrelation,
        workspace_directory: &str,
        thread_id: &str,
        review: &ConversationApprovalReview,
    ) -> bool {
        self.turn_submission == turn_submission
            && self.workspace_directory == workspace_directory
            && self.thread_id == thread_id
            && self.review == *review
    }
}

#[derive(Debug, Clone)]
pub(super) struct ApprovalReviewPersistenceCoordinator {
    current_conversation_turn_submission: Option<TurnSubmissionCorrelation>,
    next_generation: u64,
    active: Option<ApprovalReviewPersistenceCorrelation>,
    queued: VecDeque<ApprovalReviewPersistenceCorrelation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ApprovalReviewPersistenceSettlement {
    pub(super) completion_matches_current_turn: bool,
    pub(super) next: Option<ApprovalReviewPersistenceCorrelation>,
}

impl ApprovalReviewPersistenceCoordinator {
    pub(super) fn new() -> Self {
        Self {
            current_conversation_turn_submission: None,
            next_generation: 1,
            active: None,
            queued: VecDeque::new(),
        }
    }

    pub(super) fn begin_conversation_turn(&mut self, turn_submission: TurnSubmissionCorrelation) {
        self.current_conversation_turn_submission = Some(turn_submission);
    }

    pub(super) fn invalidate_conversation(&mut self) {
        self.current_conversation_turn_submission = None;
    }

    pub(super) fn enqueue(
        &mut self,
        turn_submission: TurnSubmissionCorrelation,
        workspace_directory: String,
        thread_id: String,
        review: ConversationApprovalReview,
    ) -> Option<ApprovalReviewPersistenceCorrelation> {
        if self
            .queued
            .back()
            .or(self.active.as_ref())
            .is_some_and(|pending| {
                pending.matches_update(turn_submission, &workspace_directory, &thread_id, &review)
            })
        {
            return None;
        }
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .expect("approval review persistence generation exhausted");
        let correlation = ApprovalReviewPersistenceCorrelation::new(
            generation,
            turn_submission,
            workspace_directory,
            thread_id,
            review,
        );
        if self.active.is_none() {
            self.active = Some(correlation.clone());
            return Some(correlation);
        }
        self.queued.push_back(correlation);
        None
    }

    pub(super) fn complete(
        &mut self,
        correlation: &ApprovalReviewPersistenceCorrelation,
    ) -> Option<ApprovalReviewPersistenceSettlement> {
        if self.active.as_ref() != Some(correlation) {
            return None;
        }
        self.active = None;
        let next = self.queued.pop_front();
        self.active = next.clone();
        Some(ApprovalReviewPersistenceSettlement {
            completion_matches_current_turn: self.current_conversation_turn_submission
                == Some(correlation.turn_submission),
            next,
        })
    }
}

impl Default for ApprovalReviewPersistenceCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::ConversationApprovalReviewStatus;

    #[test]
    fn coordinator_coalesces_only_consecutive_identical_updates() {
        let mut coordinator = ApprovalReviewPersistenceCoordinator::new();
        let turn = TurnSubmissionCorrelation::new(1);
        let waiting = review("waiting");
        let approved = review("approved");

        let first = coordinator
            .enqueue(
                turn,
                "/tmp/workspace".to_string(),
                "thread-1".to_string(),
                waiting.clone(),
            )
            .expect("first update should start");
        assert!(
            coordinator
                .enqueue(
                    turn,
                    "/tmp/workspace".to_string(),
                    "thread-1".to_string(),
                    waiting.clone(),
                )
                .is_none()
        );
        assert!(
            coordinator
                .enqueue(
                    turn,
                    "/tmp/workspace".to_string(),
                    "thread-1".to_string(),
                    approved,
                )
                .is_none()
        );
        assert!(
            coordinator
                .enqueue(
                    turn,
                    "/tmp/workspace".to_string(),
                    "thread-1".to_string(),
                    waiting,
                )
                .is_none()
        );

        let second = coordinator
            .complete(&first)
            .expect("first completion should settle")
            .next
            .expect("status progression should retain the second update");
        let third = coordinator
            .complete(&second)
            .expect("second completion should settle")
            .next
            .expect("A-B-A progression should retain the final update");
        assert_eq!(third.generation, 3);
    }

    #[test]
    fn coordinator_requires_full_correlation_and_rejects_aba_completion() {
        let mut coordinator = ApprovalReviewPersistenceCoordinator::new();
        let turn = TurnSubmissionCorrelation::new(1);
        coordinator.begin_conversation_turn(turn);
        let first = coordinator
            .enqueue(
                turn,
                "/tmp/workspace".to_string(),
                "thread-1".to_string(),
                review("waiting"),
            )
            .expect("first update should start");

        let mut mismatches = Vec::new();
        let mut wrong_generation = first.clone();
        wrong_generation.generation += 1;
        mismatches.push(wrong_generation);
        let mut wrong_turn = first.clone();
        wrong_turn.turn_submission = TurnSubmissionCorrelation::new(2);
        mismatches.push(wrong_turn);
        let mut wrong_workspace = first.clone();
        wrong_workspace.workspace_directory = "/tmp/other".to_string();
        mismatches.push(wrong_workspace);
        let mut wrong_thread = first.clone();
        wrong_thread.thread_id = "thread-2".to_string();
        mismatches.push(wrong_thread);
        let mut wrong_review = first.clone();
        wrong_review.review = review("approved");
        mismatches.push(wrong_review);

        for mismatch in mismatches {
            assert!(
                coordinator.complete(&mismatch).is_none(),
                "a partial correlation match must not settle the active write"
            );
        }

        assert!(
            coordinator
                .enqueue(
                    turn,
                    "/tmp/workspace".to_string(),
                    "thread-1".to_string(),
                    review("approved"),
                )
                .is_none()
        );
        let second = coordinator
            .complete(&first)
            .expect("the exact completion should settle")
            .next
            .expect("the queued update should start");
        assert!(
            coordinator.complete(&first).is_none(),
            "an old completion must not settle the new active write"
        );
        assert!(
            coordinator
                .complete(&second)
                .expect("the second exact completion should settle")
                .next
                .is_none()
        );
    }

    fn review(status: &str) -> ConversationApprovalReview {
        ConversationApprovalReview {
            target_item_id: "tool-1".to_string(),
            status: ConversationApprovalReviewStatus::Unknown(status.to_string()),
            risk_level: None,
            rationale: None,
        }
    }
}
