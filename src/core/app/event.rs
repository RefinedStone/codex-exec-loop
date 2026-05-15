use super::{
    AppSnapshot, ConversationReadySnapshot, ConversationSnapshot, SessionCatalogReadySnapshot,
    SessionCatalogSnapshot,
};
use super::{StartupReadySnapshot, StartupSnapshot};
use super::{TurnStreamEvent, TurnStreamSnapshot};
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::{
    ManualPromptOutcome, PostTurnExecution, RuntimeProjection, TurnSnapshotCapture,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreInput {
    Command(super::AppCommand),
    EffectCompleted(CoreEffectCompletion),
    ConversationStreamUpdated(TurnStreamEvent),
    ConversationTurnCompleted {
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: TurnSnapshotCapture,
    },
    ConversationRuntimeNotice(String),
    ConversationTurnWorkspaceChanged {
        workspace_directory: String,
    },
    ParallelModeSupervisorSnapshotInvalidated,
    RuntimeProjectionChanged(Box<RuntimeProjection>),
    ParallelModeReadinessProjectionChanged(Option<Box<ParallelModeReadinessSnapshot>>),
    ParallelModeSupervisorProjectionChanged(Option<Box<ParallelModeSupervisorSnapshot>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffectCompletion {
    StartupChecksLoaded(Result<Box<StartupReadySnapshot>, String>),
    SessionCatalogLoaded(Result<SessionCatalogReadySnapshot, String>),
    ConversationLoaded(Result<Box<ConversationReadySnapshot>, String>),
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
    StartupChanged(StartupSnapshot),
    SessionCatalogChanged(SessionCatalogSnapshot),
    ConversationChanged(ConversationSnapshot),
    TurnStreamSnapshotChanged(TurnStreamSnapshot),
    ManualPromptPrepared(Box<ManualPromptOutcome>),
    PostTurnEvaluationCompleted(Box<PostTurnExecution>),
    ConversationTurnWorkspaceChanged { workspace_directory: String },
    ParallelModeSupervisorSnapshotInvalidated,
}
