use super::TurnStreamSnapshot;
use super::{
    AppSnapshot, ConversationReadySnapshot, ConversationSnapshot, SessionCatalogReadySnapshot,
    SessionCatalogSnapshot,
};
use super::{StartupReadySnapshot, StartupSnapshot};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::{
    PlanningRuntimeProjection, PlanningTurnExecutionSnapshotCapture,
};
use crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreInput {
    Command(super::AppCommand),
    EffectCompleted(CoreEffectCompletion),
    ConversationStreamUpdated(ConversationStreamEvent),
    ConversationTurnCompleted {
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: PlanningTurnExecutionSnapshotCapture,
    },
    ConversationRuntimeNotice(String),
    ConversationTurnWorkspaceChanged {
        workspace_directory: String,
    },
    ParallelModeSupervisorSnapshotInvalidated,
    PlanningRuntimeProjectionChanged(Box<PlanningRuntimeProjection>),
    ParallelModeReadinessProjectionChanged(Option<Box<ParallelModeReadinessSnapshot>>),
    ParallelModeSupervisorProjectionChanged(Option<Box<ParallelModeSupervisorSnapshot>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffectCompletion {
    StartupChecksLoaded(Result<Box<StartupReadySnapshot>, String>),
    SessionCatalogLoaded(Result<SessionCatalogReadySnapshot, String>),
    ConversationLoaded(Result<Box<ConversationReadySnapshot>, String>),
    PostTurnEvaluationCompleted(Box<PostTurnEvaluationExecution>),
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
    PostTurnEvaluationCompleted(Box<PostTurnEvaluationExecution>),
    ConversationTurnWorkspaceChanged { workspace_directory: String },
    ParallelModeSupervisorSnapshotInvalidated,
}
