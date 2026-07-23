/*
 * The app module owns core-facing contracts. Inbound adapters send AppCommand
 * through CoreInput, then read AppEvent/AppSnapshot without depending on TUI
 * state or terminal framework types.
 */
pub mod approval;
pub mod command;
mod controller;
pub mod conversation;
pub mod directions;
pub mod effect;
pub mod event;
pub mod github_review_poll;
pub mod manual_prompt;
pub mod planning_runtime;
pub mod planning_workspace;
pub mod projection;
pub mod queue;
pub mod request;
pub mod review_center;
pub mod session;
pub mod snapshot;
pub mod startup;
mod state;
pub mod turn_interrupt;
pub mod turn_steer;
pub mod turn_stream;
pub mod turn_submission;

pub use approval::{
    ApprovalDecisionAdmission, ApprovalDecisionCorrelation, ApprovalReviewPersistenceCorrelation,
};
pub use command::AppCommand;
pub(in crate::core) use controller::CoreController;
pub use controller::CoreDispatchOutcome;
pub use conversation::{
    ConversationReadySnapshot, ConversationSnapshot, ConversationState,
    ConversationThreadReviewSnapshot,
};
pub use directions::{
    DirectionsMaintenanceDirectionSnapshot, DirectionsMaintenanceSummarySnapshot,
    DirectionsSupportingFileStatus,
};
pub use effect::CoreEffect;
pub use event::{AppEvent, CoreEffectCompletion, CoreInput, SessionRenameAcceptedSnapshot};
pub(crate) use github_review_poll::github_review_polling_target_is_valid;
pub use github_review_poll::{
    GithubReviewPollCorrelation, GithubReviewPollingSetupCorrelation, GithubReviewPollingSetupMode,
    GithubReviewPollingSetupRequest, GithubReviewPollingSetupResult,
};
pub use manual_prompt::{ManualPromptPreparationAdmission, ManualPromptPreparationIntent};
pub use planning_runtime::{
    PlanningDoctorSnapshot, PlanningDoctorSnapshotState, PlanningRuntimeRefreshSnapshot,
};
pub use planning_workspace::{
    PlanningEditorFileSnapshot, PlanningEditorMutationAction, PlanningEditorMutationIdentity,
    PlanningEditorMutationRequest, PlanningEditorMutationResult, PlanningEditorMutationTarget,
    PlanningEditorSessionIdentity, PlanningEditorSessionSnapshot, PlanningEditorStageSnapshot,
    PlanningEditorStageTarget, PlanningSimpleDraftPromotionSnapshot,
    PlanningSimpleDraftStageSnapshot, PlanningWorkspaceOperationAdmission,
    PlanningWorkspaceOperationCorrelation, PlanningWorkspaceOperationIntent,
    PlanningWorkspaceOperationKind, PlanningWorkspaceResetIntent, PlanningWorkspaceResetSnapshot,
    PlanningWorkspaceResetTarget,
};
pub use projection::{
    ParallelModeProjection, PlanningParallelProjection, RevisionedPlanningParallelProjection,
};
pub use queue::{
    QueueAuthorityLoadError, QueueAuthoritySnapshot, QueueMutationCommitSnapshot,
    QueueMutationCorrelation, QueueMutationIntent, QueueMutationKind, QueueMutationResult,
    QueueMutationTarget,
};
pub use request::{
    ConversationLoadCorrelation, DirectionsMaintenanceLoadCorrelation, ParallelPeekLoadCorrelation,
    PlanningRuntimeRefreshCorrelation, PostTurnEvaluationCorrelation,
    QueueAuthorityLoadCorrelation, ReviewCenterLoadCorrelation, SessionCatalogLoadCorrelation,
    SessionRenameAdmission, SessionRenameCorrelation, StartupCheckCorrelation,
};
pub use review_center::{
    ReviewCenterHistoryEntrySnapshot, ReviewCenterInboxItemSnapshot, ReviewCenterSnapshot,
};
pub use session::{SessionCatalogReadySnapshot, SessionCatalogSnapshot, SessionCatalogState};
pub use snapshot::AppSnapshot;
pub use startup::{
    StartupAttachmentSnapshot, StartupDiagnosticSnapshot, StartupReadySnapshot, StartupSnapshot,
    StartupState,
};
pub use turn_interrupt::{StopRequestAdmission, StopRequestAttempt, StopRequestCorrelation};
pub use turn_steer::{TurnSteerAdmission, TurnSteerCorrelation};
pub(in crate::core) use turn_stream::TurnStreamState;
#[cfg(test)]
pub(crate) use turn_stream::TurnStreamTestHarness;
pub use turn_stream::{
    TurnStreamEvent, TurnStreamProgressiveActivityUpdate, TurnStreamRuntimeEnvelopeRejection,
    TurnStreamSnapshot, TurnStreamTerminalSnapshot, TurnStreamUpdate,
};
pub use turn_submission::{
    CorePromptOrigin, TurnSubmissionAdmission, TurnSubmissionCorrelation, TurnSubmissionRequest,
};
