use crate::application::service::planning::PlanningRuntimeQueuedAutoFollowPrompt;

use super::super::turn_activity::TurnActivityState;
use super::AutoFollowState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoFollowupDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Skip(AutoFollowupSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoFollowupSkipReason {
    PostTurnContinuationPaused,
    LimitReached,
    NoAgentReply,
    StopKeywordMatched,
    NoFileChanges,
    PlanningBlocked,
    PlanningQueueIdlePolicyStop,
    PlanningQueueHeadRequired,
    PlanningRepeatedQueueHead,
    ParallelSessionCompleted,
}

impl AutoFollowupSkipReason {
    pub(crate) fn detail(
        self,
        auto_follow_state: &AutoFollowState,
        turn_activity: &TurnActivityState,
    ) -> String {
        match self {
            Self::PostTurnContinuationPaused => {
                "post-turn continuation is paused for this internal runtime cycle".to_string()
            }
            Self::LimitReached => format!(
                "reached the configured auto-turn budget ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                "a non-empty agent reply is required before the next auto turn can be queued"
                    .to_string()
            }
            Self::StopKeywordMatched => format!(
                "the latest agent reply matched the stop keyword {}",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => format!(
                "the last completed turn changed {} files while the no-file stop rule is on",
                turn_activity.last_completed_file_change_count()
            ),
            Self::PlanningBlocked => {
                "planning files are invalid or incomplete; auto follow-up stays paused until they validate"
                    .to_string()
            }
            Self::PlanningQueueIdlePolicyStop => {
                "the planning queue is idle and queue_idle.policy is stop".to_string()
            }
            Self::PlanningQueueHeadRequired => {
                "queue-driven auto follow-up requires an actionable planning queue head"
                    .to_string()
            }
            Self::PlanningRepeatedQueueHead => {
                "the planning queue selected the same task again; auto follow-up stays paused until the queue advances"
                    .to_string()
            }
            Self::ParallelSessionCompleted => {
                "parallel agent session completed its assigned task; follow-up stays with the supervisor instead of reusing the same slot session"
                    .to_string()
            }
        }
    }

    pub(crate) fn activity_summary(self) -> &'static str {
        match self {
            Self::PostTurnContinuationPaused => "paused: internal continuation",
            Self::LimitReached => "stopped: turn limit reached",
            Self::NoAgentReply => "skipped: no agent reply",
            Self::StopKeywordMatched => "stopped: stop keyword matched",
            Self::NoFileChanges => "stopped: no file changes",
            Self::PlanningBlocked => "paused: planning files invalid",
            Self::PlanningQueueIdlePolicyStop => "stopped: queue idle policy stop",
            Self::PlanningQueueHeadRequired => "paused: planning queue empty",
            Self::PlanningRepeatedQueueHead => "paused: planning queue repeated the same task",
            Self::ParallelSessionCompleted => "stopped: parallel session completed",
        }
    }

    pub(crate) fn runtime_status(self, auto_follow_state: &AutoFollowState) -> String {
        match self {
            Self::PostTurnContinuationPaused => {
                "turn completed / internal continuation paused".to_string()
            }
            Self::LimitReached => format!(
                "turn completed / auto follow-up stopped: turn limit reached ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                "turn completed / auto follow-up skipped: no agent reply".to_string()
            }
            Self::StopKeywordMatched => format!(
                "turn completed / auto follow-up stopped: stop keyword matched ({})",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => {
                "turn completed / auto follow-up stopped: no file changes".to_string()
            }
            Self::PlanningBlocked => {
                "turn completed / auto follow-up paused: planning files invalid".to_string()
            }
            Self::PlanningQueueIdlePolicyStop => {
                "turn completed / auto follow-up stopped: planning queue idle policy is stop"
                    .to_string()
            }
            Self::PlanningQueueHeadRequired => {
                "turn completed / auto follow-up paused: planning queue has no next task"
                    .to_string()
            }
            Self::PlanningRepeatedQueueHead => {
                "turn completed / auto follow-up paused: planning queue repeated the previous task"
                    .to_string()
            }
            Self::ParallelSessionCompleted => {
                "turn completed / auto follow-up stopped: parallel session completion is handed back to the supervisor"
                    .to_string()
            }
        }
    }
}
