#[derive(Debug, Default, Clone)]
// queue-follow policy는 runtime projection이나 UI state를 모르는 순수 automation gate다.
pub struct PlanningQueueFollowPolicy;

impl PlanningQueueFollowPolicy {
    pub fn new() -> Self {
        Self
    }

    pub fn decide(&self, facts: PlanningQueueFollowFacts) -> PlanningQueueFollowDecision {
        if !facts.workspace_valid {
            return PlanningQueueFollowDecision::Blocked(
                PlanningQueueFollowBlockReason::InvalidWorkspace,
            );
        }
        if facts.repeated_queue_head {
            return PlanningQueueFollowDecision::Blocked(
                PlanningQueueFollowBlockReason::RepeatedQueueHead,
            );
        }
        if !facts.has_actionable_queue_head {
            return PlanningQueueFollowDecision::Blocked(
                PlanningQueueFollowBlockReason::ActionableQueueRequired,
            );
        }
        PlanningQueueFollowDecision::QueuePrompt(PlanningQueueFollowPromptMode::ContinueQueuedTask)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningQueueFollowFacts {
    pub workspace_valid: bool,
    pub has_actionable_queue_head: bool,
    pub repeated_queue_head: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueFollowBlockReason {
    InvalidWorkspace,
    ActionableQueueRequired,
    RepeatedQueueHead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueFollowDecision {
    Blocked(PlanningQueueFollowBlockReason),
    QueuePrompt(PlanningQueueFollowPromptMode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueFollowPromptMode {
    ContinueQueuedTask,
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningQueueFollowBlockReason, PlanningQueueFollowDecision, PlanningQueueFollowFacts,
        PlanningQueueFollowPolicy, PlanningQueueFollowPromptMode,
    };

    fn decide(facts: PlanningQueueFollowFacts) -> PlanningQueueFollowDecision {
        PlanningQueueFollowPolicy::new().decide(facts)
    }

    #[test]
    fn invalid_workspace_blocks_before_repeat_detection() {
        assert_eq!(
            decide(PlanningQueueFollowFacts {
                workspace_valid: false,
                has_actionable_queue_head: true,
                repeated_queue_head: true,
            }),
            PlanningQueueFollowDecision::Blocked(PlanningQueueFollowBlockReason::InvalidWorkspace)
        );
    }

    #[test]
    fn repeated_queue_head_blocks_before_continue() {
        assert_eq!(
            decide(PlanningQueueFollowFacts {
                workspace_valid: true,
                has_actionable_queue_head: true,
                repeated_queue_head: true,
            }),
            PlanningQueueFollowDecision::Blocked(PlanningQueueFollowBlockReason::RepeatedQueueHead)
        );
    }

    #[test]
    fn queue_idle_requires_actionable_queue_head() {
        assert_eq!(
            decide(PlanningQueueFollowFacts {
                workspace_valid: true,
                has_actionable_queue_head: false,
                repeated_queue_head: false,
            }),
            PlanningQueueFollowDecision::Blocked(
                PlanningQueueFollowBlockReason::ActionableQueueRequired
            )
        );
    }

    #[test]
    fn actionable_queue_head_continues_queued_task() {
        assert_eq!(
            decide(PlanningQueueFollowFacts {
                workspace_valid: true,
                has_actionable_queue_head: true,
                repeated_queue_head: false,
            }),
            PlanningQueueFollowDecision::QueuePrompt(
                PlanningQueueFollowPromptMode::ContinueQueuedTask
            )
        );
    }
}
