use super::ManualPromptPreparationAdmission;
use super::ReviewCenterSnapshot;
use super::{
    AppSnapshot, ConversationReadySnapshot, ConversationRuntimeSnapshot, ConversationSnapshot,
    SessionCatalogReadySnapshot, SessionCatalogSnapshot,
};
use super::{
    ApprovalDecisionAdmission, ApprovalDecisionCorrelation, ApprovalReviewPersistenceCorrelation,
};
use super::{
    ConversationLoadCorrelation, GithubReviewPollCorrelation, GithubReviewPollingSetupCorrelation,
    GithubReviewPollingSetupResult, ParallelPeekLoadCorrelation, PlanningDoctorSnapshot,
    PlanningEditorMutationResult, PlanningEditorSessionSnapshot, PlanningEditorStageSnapshot,
    PlanningRuntimeRefreshCorrelation, PlanningRuntimeRefreshSnapshot,
    PlanningSimpleDraftPromotionSnapshot, PlanningSimpleDraftStageSnapshot,
    PlanningWorkspaceOperationAdmission, PlanningWorkspaceOperationCorrelation,
    PlanningWorkspaceResetSnapshot, PostTurnEvaluationCorrelation, PostTurnRouteResolution,
    ReviewCenterLoadCorrelation, SessionCatalogLoadCorrelation, SessionRenameAdmission,
    SessionRenameCorrelation, StartupCheckCorrelation,
};
use super::{
    ConversationPreferencePersistenceCorrelation, ConversationPreferencePersistenceResult,
};
use super::{DirectionsMaintenanceLoadCorrelation, DirectionsMaintenanceSummarySnapshot};
use super::{
    QueueAuthorityLoadCorrelation, QueueAuthorityLoadError, QueueAuthoritySnapshot,
    QueueMutationCorrelation, QueueMutationResult,
};
use super::{StartupReadySnapshot, StartupSnapshot};
use super::{StopRequestAdmission, StopRequestAttempt, StopRequestCorrelation};
use super::{TurnSteerAdmission, TurnSteerCorrelation};
use super::{
    TurnStreamEvent, TurnStreamSnapshot, TurnSubmissionAdmission, TurnSubmissionCorrelation,
};
use crate::domain::conversation::ConversationTurnSteerReceipt;
use crate::domain::github_review::GithubPullRequestPollResult;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::{
    ManualPromptOutcome, PlanningWorkerPanelState, PostTurnExecution, RuntimeProjection,
};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRenameAcceptedSnapshot {
    pub session_catalog: SessionCatalogSnapshot,
    pub turn_stream: Option<Box<TurnStreamSnapshot>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreInput {
    Command(super::AppCommand),
    // 호스트의 시계이다 (plan 18 K1-1). 무엇을 할지는 core 가 정한다 (예: GitHub 리뷰 폴링).
    Tick {
        now: std::time::Instant,
    },
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
    RuntimeProjectionChanged {
        workspace_directory: String,
        projection: Box<RuntimeProjection>,
    },
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
    DirectionsMaintenanceLoaded {
        correlation: DirectionsMaintenanceLoadCorrelation,
        result: Result<Box<DirectionsMaintenanceSummarySnapshot>, String>,
    },
    PlanningRuntimeLoaded {
        correlation: PlanningRuntimeRefreshCorrelation,
        result: Result<Box<PlanningRuntimeRefreshSnapshot>, String>,
    },
    PlanningWorkspaceResetCompleted {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningWorkspaceResetSnapshot>, String>,
    },
    PlanningSimpleDraftStaged {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningSimpleDraftStageSnapshot>, String>,
    },
    PlanningEditorStaged {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningEditorStageSnapshot>, String>,
    },
    PlanningEditorMutationCompleted {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningEditorMutationResult>, String>,
    },
    PlanningSimpleEditorLoaded {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningEditorSessionSnapshot>, String>,
    },
    PlanningSimpleDraftPromoted {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningSimpleDraftPromotionSnapshot>, String>,
    },
    QueueMutationCompleted {
        correlation: QueueMutationCorrelation,
        result: Box<QueueMutationResult>,
    },
    StopRequestAttemptCompleted {
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
        result: Result<(), String>,
    },
    TurnSteered {
        correlation: TurnSteerCorrelation,
        result: Result<ConversationTurnSteerReceipt, String>,
    },
    ApprovalDecisionSubmitted {
        correlation: ApprovalDecisionCorrelation,
        result: Result<(), String>,
    },
    ApprovalReviewPersisted {
        correlation: ApprovalReviewPersistenceCorrelation,
        result: Result<(), String>,
    },
    ConversationPreferencesPersisted {
        correlation: ConversationPreferencePersistenceCorrelation,
        result: ConversationPreferencePersistenceResult,
    },
    GithubReviewPollingSetupCompleted {
        correlation: GithubReviewPollingSetupCorrelation,
        result: Result<GithubReviewPollingSetupResult, String>,
    },
    GithubReviewPollCompleted {
        correlation: GithubReviewPollCorrelation,
        result: Result<Box<GithubPullRequestPollResult>, String>,
    },
    ManualPromptPrepared(Box<ManualPromptOutcome>),
    PostTurnEvaluationCompleted {
        correlation: PostTurnEvaluationCorrelation,
        execution: Box<PostTurnExecution>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /*
     * SnapshotChanged is the neutral output event for the skeleton. Concrete
     * slices can add narrower events such as StartupChanged while preserving the
     * same core-to-inbound adapter direction.
     */
    SnapshotChanged(Arc<AppSnapshot>),
    StartupChanged {
        correlation: StartupCheckCorrelation,
        snapshot: StartupSnapshot,
    },
    SessionCatalogChanged(SessionCatalogSnapshot),
    SessionRenameAdmissionResolved(SessionRenameAdmission),
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
    DirectionsMaintenanceLoadStarted {
        correlation: DirectionsMaintenanceLoadCorrelation,
    },
    DirectionsMaintenanceLoaded {
        correlation: DirectionsMaintenanceLoadCorrelation,
        result: Result<Box<DirectionsMaintenanceSummarySnapshot>, String>,
    },
    PlanningRuntimeRefreshStarted {
        correlation: PlanningRuntimeRefreshCorrelation,
    },
    PlanningRuntimeRefreshed {
        correlation: PlanningRuntimeRefreshCorrelation,
        result: Result<PlanningDoctorSnapshot, String>,
    },
    PlanningRuntimeRefreshCancelled {
        correlation: PlanningRuntimeRefreshCorrelation,
    },
    PlanningWorkspaceOperationAdmissionResolved(PlanningWorkspaceOperationAdmission),
    PlanningWorkspaceResetCompleted {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningWorkspaceResetSnapshot>, String>,
    },
    PlanningSimpleDraftStaged {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningSimpleDraftStageSnapshot>, String>,
    },
    PlanningEditorStaged {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningEditorStageSnapshot>, String>,
    },
    PlanningEditorMutationCompleted {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningEditorMutationResult>, String>,
    },
    PlanningSimpleEditorLoaded {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningEditorSessionSnapshot>, String>,
    },
    PlanningSimpleDraftPromoted {
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningSimpleDraftPromotionSnapshot>, String>,
    },
    QueueMutationStarted {
        correlation: QueueMutationCorrelation,
    },
    QueueMutationCompleted {
        correlation: QueueMutationCorrelation,
        result: Box<QueueMutationResult>,
    },
    StopRequestAdmissionResolved(StopRequestAdmission),
    StopRequestAttemptCompleted {
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
        result: Result<(), String>,
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
    ConversationPreferencesPersisted {
        correlation: ConversationPreferencePersistenceCorrelation,
        result: ConversationPreferencePersistenceResult,
    },
    ConversationRuntimeAuthorityChanged(Box<ConversationRuntimeSnapshot>),
    GithubReviewPollingSetupStarted {
        correlation: GithubReviewPollingSetupCorrelation,
    },
    GithubReviewPollingSetupCompleted {
        correlation: GithubReviewPollingSetupCorrelation,
        result: Result<GithubReviewPollingSetupResult, String>,
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
    PostTurnEvaluationStarted(PlanningWorkerPanelState),
    // 내부 이벤트이다 (plan 18 K1-4). 턴이 끝나서 평가를 기다린다. 호스트(NativeClientRuntime)가 병렬 값을
    // 붙여 `AppCommand::EvaluateCompletedTurn` 으로 다시 넣고, 클라이언트에는 보내지 않는다.
    PostTurnEvaluationDue {
        planning_workspace_directory: String,
    },
    // 호스트가 제출한 자동 후속 턴의 결과이다 (plan 18 K1-5). 화면은 이 값으로 자동 후속 메시지를 그린다.
    AutoFollowTurnSubmissionResolved {
        source: PostTurnEvaluationCorrelation,
        completed_turn_id: String,
        transcript_text: String,
        mode_label: String,
        admitted: bool,
    },
    PostTurnContinuationRoutingRequested {
        correlation: PostTurnEvaluationCorrelation,
        execution: Box<PostTurnExecution>,
    },
    PostTurnEvaluationCompleted {
        correlation: PostTurnEvaluationCorrelation,
        execution: Box<PostTurnExecution>,
        route_resolution: PostTurnRouteResolution,
    },
    ConversationTurnWorkspaceChanged {
        workspace_directory: String,
    },
    // 일이 남은 thread(보는 thread 포함)의 요약이 바뀌었다 (plan 18 K2-4, K3-3a).
    ThreadActivityChanged(Vec<super::ThreadActivitySummary>),
    // thread 를 지정한 프롬프트의 받아들임 결과이다 (plan 18 K3-3b).
    ThreadPromptAdmissionResolved {
        thread_id: String,
        admission: super::ThreadPromptAdmission,
    },
    // thread 를 지정한 프롬프트의 준비가 끝나고 제출까지 한 결과이다 (plan 18 K3-3b).
    ThreadPromptSubmitted {
        thread_id: String,
        outcome: super::ThreadPromptOutcome,
    },
    ParallelModeSupervisorSnapshotInvalidated,
}

impl AppEvent {
    pub fn turn_stream_snapshot_changed(snapshot: TurnStreamSnapshot) -> Self {
        Self::TurnStreamSnapshotChanged(Box::new(snapshot))
    }
}
