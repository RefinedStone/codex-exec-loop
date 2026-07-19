use super::ManualPromptPreparationAdmission;
use super::ReviewCenterSnapshot;
use super::{
    AppSnapshot, ConversationReadySnapshot, ConversationSnapshot, SessionCatalogReadySnapshot,
    SessionCatalogSnapshot,
};
use super::{ApprovalDecisionAdmission, ApprovalDecisionCorrelation};
use super::{
    ConversationLoadCorrelation, GithubReviewPollCorrelation, ParallelPeekLoadCorrelation,
    ReviewCenterLoadCorrelation, SessionCatalogLoadCorrelation, SessionRenameCorrelation,
    StartupCheckCorrelation,
};
use super::{QueueAuthorityLoadCorrelation, QueueAuthorityLoadError, QueueAuthoritySnapshot};
use super::{StartupReadySnapshot, StartupSnapshot};
use super::{TurnSteerAdmission, TurnSteerCorrelation};
use super::{
    TurnStreamEvent, TurnStreamSnapshot, TurnSubmissionAdmission, TurnSubmissionCorrelation,
};
use crate::domain::conversation::ConversationTurnSteerReceipt;
use crate::domain::github_review::GithubPullRequestPollResult;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::{ManualPromptOutcome, PostTurnExecution, RuntimeProjection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRenameAcceptedSnapshot {
    pub session_catalog: SessionCatalogSnapshot,
    pub turn_stream: Option<Box<TurnStreamSnapshot>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreInput {
    Command(super::AppCommand),
    EffectCompleted(CoreEffectCompletion),
    ConversationStreamUpdated {
        correlation: TurnSubmissionCorrelation,
        event: TurnStreamEvent,
    },
    ConversationRuntimeNotice(String),
    ConversationTurnRuntimeNotice {
        correlation: TurnSubmissionCorrelation,
        notice: String,
    },
    ConversationTurnWorkspaceChanged {
        correlation: TurnSubmissionCorrelation,
        workspace_directory: String,
    },
    ParallelModeSupervisorSnapshotInvalidated,
    RuntimeProjectionChanged(Box<RuntimeProjection>),
    ParallelModeReadinessProjectionChanged(Option<Box<ParallelModeReadinessSnapshot>>),
    ParallelModeSupervisorProjectionChanged(Option<Box<ParallelModeSupervisorSnapshot>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffectCompletion {
    StartupChecksLoaded {
        correlation: StartupCheckCorrelation,
        result: Result<Box<StartupReadySnapshot>, String>,
    },
    SessionCatalogLoaded {
        correlation: SessionCatalogLoadCorrelation,
        result: Result<SessionCatalogReadySnapshot, String>,
    },
    SessionRenamed {
        correlation: SessionRenameCorrelation,
        result: Result<(), String>,
    },
    ConversationLoaded {
        correlation: ConversationLoadCorrelation,
        result: Result<Box<ConversationReadySnapshot>, String>,
    },
    ParallelPeekConversationLoaded {
        correlation: ParallelPeekLoadCorrelation,
        result: Result<Box<ConversationReadySnapshot>, String>,
    },
    ReviewCenterLoaded {
        correlation: ReviewCenterLoadCorrelation,
        snapshot: ReviewCenterSnapshot,
    },
    QueueAuthorityLoaded {
        correlation: QueueAuthorityLoadCorrelation,
        result: Result<Box<QueueAuthoritySnapshot>, QueueAuthorityLoadError>,
    },
    TurnSteered {
        correlation: TurnSteerCorrelation,
        result: Result<ConversationTurnSteerReceipt, String>,
    },
    ApprovalDecisionSubmitted {
        correlation: ApprovalDecisionCorrelation,
        result: Result<(), String>,
    },
    GithubReviewPollCompleted {
        correlation: GithubReviewPollCorrelation,
        result: Result<Box<GithubPullRequestPollResult>, String>,
    },
    ManualPromptPrepared(Box<ManualPromptOutcome>),
    PostTurnEvaluationCompleted(Box<PostTurnExecution>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /*
     * SnapshotChanged is the neutral output event for the skeleton. Concrete
     * slices can add narrower events such as StartupChanged while preserving the
     * same core-to-inbound adapter direction.
     */
    SnapshotChanged(AppSnapshot),
    StartupChanged {
        correlation: StartupCheckCorrelation,
        snapshot: StartupSnapshot,
    },
    SessionCatalogChanged(SessionCatalogSnapshot),
    SessionRenameCompleted {
        correlation: SessionRenameCorrelation,
        result: Result<SessionRenameAcceptedSnapshot, String>,
    },
    ConversationChanged {
        correlation: Option<ConversationLoadCorrelation>,
        snapshot: ConversationSnapshot,
    },
    ParallelPeekConversationLoaded {
        correlation: ParallelPeekLoadCorrelation,
        result: Result<Box<ConversationReadySnapshot>, String>,
    },
    ReviewCenterLoadStarted {
        correlation: ReviewCenterLoadCorrelation,
    },
    ReviewCenterLoaded {
        correlation: ReviewCenterLoadCorrelation,
        snapshot: ReviewCenterSnapshot,
    },
    QueueAuthorityLoadStarted {
        correlation: QueueAuthorityLoadCorrelation,
    },
    QueueAuthorityLoaded {
        correlation: QueueAuthorityLoadCorrelation,
        result: Result<Box<QueueAuthoritySnapshot>, QueueAuthorityLoadError>,
    },
    ManualPromptPreparationAdmissionResolved(ManualPromptPreparationAdmission),
    TurnSubmissionAdmissionResolved(TurnSubmissionAdmission),
    TurnSteerAdmissionResolved(TurnSteerAdmission),
    TurnSteerCompleted {
        correlation: TurnSteerCorrelation,
        result: Result<ConversationTurnSteerReceipt, String>,
    },
    ApprovalDecisionAdmissionResolved(ApprovalDecisionAdmission),
    ApprovalDecisionSubmissionCompleted {
        correlation: ApprovalDecisionCorrelation,
        result: Result<(), String>,
    },
    GithubReviewPollStarted {
        correlation: GithubReviewPollCorrelation,
    },
    GithubReviewPollCompleted {
        correlation: GithubReviewPollCorrelation,
        result: Result<Box<GithubPullRequestPollResult>, String>,
    },
    TurnStreamSnapshotChanged(Box<TurnStreamSnapshot>),
    ManualPromptPrepared(Box<ManualPromptOutcome>),
    PostTurnEvaluationCompleted(Box<PostTurnExecution>),
    ConversationTurnWorkspaceChanged {
        workspace_directory: String,
    },
    ParallelModeSupervisorSnapshotInvalidated,
}

impl AppEvent {
    pub fn turn_stream_snapshot_changed(snapshot: TurnStreamSnapshot) -> Self {
        Self::TurnStreamSnapshotChanged(Box::new(snapshot))
    }
}
