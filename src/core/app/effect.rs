use super::{
    ApprovalDecisionCorrelation, ConversationLoadCorrelation, GithubReviewPollCorrelation,
    ParallelPeekLoadCorrelation, PlanningRuntimeRefreshCorrelation, QueueAuthorityLoadCorrelation,
    QueueMutationCorrelation, ReviewCenterLoadCorrelation, SessionCatalogLoadCorrelation,
    SessionRenameCorrelation, StartupCheckCorrelation, StopRequestAttempt, StopRequestCorrelation,
    TurnSteerCorrelation, TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::domain::conversation::ConversationTurnSteerRequest;
use crate::domain::github_review::GithubPullRequestPollState;
use crate::domain::planning::{ManualPromptRequest, PostTurnRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffect {
    RunStartupChecks {
        correlation: StartupCheckCorrelation,
    },
    LoadSessionCatalog {
        correlation: SessionCatalogLoadCorrelation,
        limit: usize,
        workspace_directory: String,
    },
    RenameSession {
        correlation: SessionRenameCorrelation,
    },
    LoadConversation {
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
    },
    LoadParallelPeekConversation {
        correlation: ParallelPeekLoadCorrelation,
    },
    LoadReviewCenter {
        correlation: ReviewCenterLoadCorrelation,
    },
    LoadQueueAuthority {
        correlation: QueueAuthorityLoadCorrelation,
    },
    LoadPlanningRuntime {
        correlation: PlanningRuntimeRefreshCorrelation,
    },
    ExecuteQueueMutation {
        correlation: QueueMutationCorrelation,
    },
    PrepareManualPrompt(Box<ManualPromptRequest>),
    SubmitTurn {
        correlation: TurnSubmissionCorrelation,
        request: TurnSubmissionRequest,
    },
    RequestStopAllSessions {
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
    },
    SteerTurn {
        correlation: TurnSteerCorrelation,
        request: ConversationTurnSteerRequest,
    },
    SubmitApprovalDecision {
        correlation: ApprovalDecisionCorrelation,
    },
    PollGithubReview {
        correlation: GithubReviewPollCorrelation,
        previous_state: Option<GithubPullRequestPollState>,
    },
    EvaluatePostTurn(Box<PostTurnRequest>),
}
