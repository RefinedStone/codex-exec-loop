use super::{
    AppSnapshot, ConversationReadySnapshot, ConversationSnapshot, SessionCatalogReadySnapshot,
    SessionCatalogSnapshot,
};
use super::{ConversationLoadCorrelation, StartupCheckCorrelation};
use super::{StartupReadySnapshot, StartupSnapshot};
use super::{TurnStreamEvent, TurnStreamSnapshot, TurnSubmissionCorrelation};
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::{ManualPromptOutcome, PostTurnExecution, RuntimeProjection};

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
    SessionCatalogLoaded(Result<SessionCatalogReadySnapshot, String>),
    ConversationLoaded {
        correlation: ConversationLoadCorrelation,
        result: Result<Box<ConversationReadySnapshot>, String>,
    },
    ParallelPeekConversationLoaded {
        request_id: u64,
        thread_id: String,
        result: Result<Box<ConversationReadySnapshot>, String>,
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
    ConversationChanged {
        correlation: Option<ConversationLoadCorrelation>,
        snapshot: ConversationSnapshot,
    },
    ParallelPeekConversationLoaded {
        request_id: u64,
        thread_id: String,
        result: Result<Box<ConversationReadySnapshot>, String>,
    },
    TurnStreamSnapshotChanged(TurnStreamSnapshot),
    ManualPromptPrepared(Box<ManualPromptOutcome>),
    PostTurnEvaluationCompleted(Box<PostTurnExecution>),
    ConversationTurnWorkspaceChanged {
        workspace_directory: String,
    },
    ParallelModeSupervisorSnapshotInvalidated,
}
