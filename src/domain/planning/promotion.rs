use super::PriorityQueueProjection;

#[derive(Default, Clone)]
// proposal promotion policy는 queue projection만 읽고 promotion 가능 여부를 결정한다.
pub struct PlanningProposalPromotionPolicy;

impl PlanningProposalPromotionPolicy {
    pub fn new() -> Self {
        Self
    }

    pub fn decide(
        &self,
        projection: &PriorityQueueProjection,
    ) -> PlanningProposalPromotionDecision {
        if projection.next_task.is_some() {
            return PlanningProposalPromotionDecision::Noop(
                PlanningProposalPromotionNoopReason::ExecutableQueueHeadExists,
            );
        }
        let Some(top_proposal) = projection.proposed_tasks.first() else {
            return PlanningProposalPromotionDecision::Noop(
                PlanningProposalPromotionNoopReason::NoPromotableProposal,
            );
        };
        PlanningProposalPromotionDecision::Promote(PlanningProposalPromotionCandidate {
            task_id: top_proposal.task_id.trim().to_string(),
            task_title: top_proposal.task_title.trim().to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningProposalPromotionDecision {
    Promote(PlanningProposalPromotionCandidate),
    Noop(PlanningProposalPromotionNoopReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProposalPromotionCandidate {
    pub task_id: String,
    pub task_title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningProposalPromotionNoopReason {
    ExecutableQueueHeadExists,
    NoPromotableProposal,
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningProposalPromotionDecision, PlanningProposalPromotionNoopReason,
        PlanningProposalPromotionPolicy,
    };
    use crate::domain::planning::{
        PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus,
    };

    fn queue_task(
        rank: usize,
        task_id: &str,
        title: &str,
        status: TaskStatus,
    ) -> PriorityQueueTask {
        PriorityQueueTask {
            rank,
            task_id: task_id.to_string(),
            direction_id: "general-workstream".to_string(),
            direction_title: "General".to_string(),
            task_title: title.to_string(),
            status,
            combined_priority: 50,
            updated_at: "2026-04-10T00:00:00Z".to_string(),
            rank_reasons: Vec::new(),
        }
    }

    fn projection(
        next_task: Option<PriorityQueueTask>,
        proposed_tasks: Vec<PriorityQueueTask>,
    ) -> PriorityQueueProjection {
        PriorityQueueProjection {
            next_task,
            active_tasks: Vec::new(),
            proposed_tasks,
            skipped_tasks: Vec::<PriorityQueueSkippedTask>::new(),
        }
    }

    #[test]
    fn keeps_existing_executable_queue_head_authoritative() {
        let decision = PlanningProposalPromotionPolicy::new().decide(&projection(
            Some(queue_task(1, "ready-task", "Ready task", TaskStatus::Ready)),
            vec![queue_task(
                1,
                "proposal-task",
                "Proposal task",
                TaskStatus::Proposed,
            )],
        ));

        assert_eq!(
            decision,
            PlanningProposalPromotionDecision::Noop(
                PlanningProposalPromotionNoopReason::ExecutableQueueHeadExists
            )
        );
    }

    #[test]
    fn skips_when_no_promotable_proposal_exists() {
        let decision = PlanningProposalPromotionPolicy::new().decide(&projection(None, Vec::new()));

        assert_eq!(
            decision,
            PlanningProposalPromotionDecision::Noop(
                PlanningProposalPromotionNoopReason::NoPromotableProposal
            )
        );
    }

    #[test]
    fn selects_top_projected_proposal_for_promotion() {
        let decision = PlanningProposalPromotionPolicy::new().decide(&projection(
            None,
            vec![
                queue_task(1, " proposal-a ", " Proposal A ", TaskStatus::Proposed),
                queue_task(2, "proposal-b", "Proposal B", TaskStatus::Proposed),
            ],
        ));

        assert!(matches!(
            decision,
            PlanningProposalPromotionDecision::Promote(candidate)
                if candidate.task_id == "proposal-a" && candidate.task_title == "Proposal A"
        ));
    }
}
