use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Result;

use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};
use crate::application::service::conversation_service::{
    ConversationService, LoadedConversationThreadSnapshot,
};
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::manual_prompt_preparation::ManualPromptPreparationService;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::{
    DirectionsMaintenanceSummary as ApplicationDirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus as ApplicationDirectionsSupportingFileStatus,
    PlanningQueueAuthorityProjection, PlanningQueueAuthorityRefreshError,
    PlanningQueueCancellationRequest, PlanningQueueCancellationTarget,
    PlanningQueueCancellationTransactionResult, PlanningQueueUseCases, PlanningRuntimeProjection,
    PlanningRuntimeUseCases, PlanningServices, PlanningTaskMutationCommitResult,
    PlanningWorkspaceUseCases,
};
use crate::application::service::post_turn_evaluation::{
    POST_TURN_EVALUATION_TIMEOUT, PostTurnEvaluationService,
};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::core_turn_submission;
use crate::core::app::{
    ApprovalDecisionCorrelation, ApprovalReviewPersistenceCorrelation, ConversationLoadCorrelation,
    ConversationReadySnapshot, ConversationThreadReviewSnapshot,
    DirectionsMaintenanceDirectionSnapshot, DirectionsMaintenanceLoadCorrelation,
    DirectionsMaintenanceSummarySnapshot,
    DirectionsSupportingFileStatus as CoreDirectionsSupportingFileStatus,
    GithubReviewPollCorrelation, ParallelPeekLoadCorrelation, PlanningRuntimeRefreshCorrelation,
    PlanningRuntimeRefreshSnapshot, QueueAuthorityLoadCorrelation, QueueAuthorityLoadError,
    QueueAuthoritySnapshot, QueueMutationCommitSnapshot, QueueMutationCorrelation,
    QueueMutationIntent, QueueMutationResult, ReviewCenterHistoryEntrySnapshot,
    ReviewCenterInboxItemSnapshot, ReviewCenterLoadCorrelation, ReviewCenterSnapshot,
    SessionCatalogLoadCorrelation, SessionCatalogReadySnapshot, SessionRenameCorrelation,
    StartupCheckCorrelation, StopRequestAttempt, StopRequestCorrelation,
};
use crate::core::app::{CoreEffect, CoreEffectCompletion, CoreInput, StartupReadySnapshot};
use crate::core::runtime::CoreEffectExecutor;
use crate::core::runtime::CoreInputSender;
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Clone)]
pub struct CoreEffectRunner {
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
    planning_workspace: PlanningWorkspaceUseCases,
    planning_runtime: PlanningRuntimeUseCases,
    planning_queue: PlanningQueueUseCases,
    parallel_mode_turn_service: ParallelModeTurnService,
    manual_prompt_preparation_service: ManualPromptPreparationService,
    post_turn_evaluation_service: PostTurnEvaluationService,
    github_review_poller_service: Option<GithubReviewPollerService>,
    manual_prompt_workers: Arc<EffectExecutionRegistry>,
    stop_request_workers: Arc<EffectExecutionRegistry>,
    input_sender: CoreInputSender,
}

#[derive(Clone)]
struct EffectExecutionPermit {
    active: Arc<AtomicBool>,
}

impl EffectExecutionPermit {
    fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(true)),
        }
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn invalidate(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct EffectExecutionRegistry {
    permits: Mutex<HashMap<u64, EffectExecutionPermit>>,
}

impl EffectExecutionRegistry {
    fn register(&self, generation: u64) -> EffectExecutionPermit {
        let permit = EffectExecutionPermit::new();
        if let Some(previous) = self
            .permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(generation, permit.clone())
        {
            previous.invalidate();
        }
        permit
    }

    fn invalidate(&self, generation: u64) {
        if let Some(permit) = self
            .permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&generation)
        {
            permit.invalidate();
        }
    }

    fn finish(&self, generation: u64, permit: &EffectExecutionPermit) {
        let mut permits = self
            .permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if permits
            .get(&generation)
            .is_some_and(|current| Arc::ptr_eq(&current.active, &permit.active))
        {
            permits.remove(&generation);
        }
    }
}

impl CoreEffectRunner {
    pub fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        post_turn_evaluation_service: PostTurnEvaluationService,
        input_sender: CoreInputSender,
    ) -> Self {
        let planning_workspace = planning_feature.workspace.clone();
        let planning_runtime = planning_feature.runtime.clone();
        let planning_queue = planning_feature.queue.clone();
        let manual_prompt_preparation_service =
            ManualPromptPreparationService::new(planning_feature);
        Self {
            startup_service,
            session_service,
            conversation_service,
            planning_workspace,
            planning_runtime,
            planning_queue,
            parallel_mode_turn_service,
            manual_prompt_preparation_service,
            post_turn_evaluation_service,
            github_review_poller_service: None,
            manual_prompt_workers: Arc::new(EffectExecutionRegistry::default()),
            stop_request_workers: Arc::new(EffectExecutionRegistry::default()),
            input_sender,
        }
    }

    pub fn with_github_review_poller_service(
        mut self,
        service: Option<GithubReviewPollerService>,
    ) -> Self {
        self.github_review_poller_service = service;
        self
    }

    pub fn spawn_startup_checks(&self, correlation: StartupCheckCorrelation) {
        let startup_service = self.startup_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let completion = startup_checks_completion(correlation, startup_service.run_checks());
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
        match effect {
            CoreEffect::RunStartupChecks { correlation } => {
                self.spawn_startup_checks(correlation);
                None
            }
            CoreEffect::LoadSessionCatalog {
                correlation,
                limit,
                workspace_directory,
            } => {
                self.spawn_session_catalog_load(correlation, limit, workspace_directory);
                None
            }
            CoreEffect::RenameSession { correlation } => {
                self.spawn_session_rename(correlation);
                None
            }
            CoreEffect::LoadConversation {
                correlation,
                fallback_workspace_directory,
            } => {
                self.spawn_conversation_load(correlation, fallback_workspace_directory);
                None
            }
            CoreEffect::LoadParallelPeekConversation { correlation } => {
                self.spawn_parallel_peek_conversation_load(correlation);
                None
            }
            CoreEffect::LoadReviewCenter { correlation } => {
                self.spawn_review_center_load(correlation);
                None
            }
            CoreEffect::LoadQueueAuthority { correlation } => {
                self.spawn_queue_authority_load(correlation);
                None
            }
            CoreEffect::LoadDirectionsMaintenance { correlation } => {
                self.spawn_directions_maintenance_load(correlation);
                None
            }
            CoreEffect::LoadPlanningRuntime { correlation } => {
                self.spawn_planning_runtime_projection_load(correlation);
                None
            }
            CoreEffect::ExecuteQueueMutation { correlation } => {
                self.spawn_queue_mutation(correlation);
                None
            }
            CoreEffect::PollGithubReview {
                correlation,
                previous_state,
            } => {
                let Some(service) = self.github_review_poller_service.clone() else {
                    return Some(CoreInput::EffectCompleted(github_review_poll_completion(
                        correlation,
                        Err(anyhow::anyhow!("github review poller is not configured")),
                    )));
                };
                self.spawn_github_review_poll(service, correlation, previous_state);
                None
            }
            CoreEffect::PrepareManualPrompt(request) => {
                let permit = self
                    .manual_prompt_workers
                    .register(request.correlation.generation);
                self.spawn_manual_prompt_preparation(*request, permit);
                None
            }
            CoreEffect::CancelManualPromptPreparation { correlation } => {
                self.manual_prompt_workers
                    .invalidate(correlation.generation);
                None
            }
            CoreEffect::SubmitApprovalDecision { correlation } => {
                self.spawn_approval_decision_submission(correlation);
                None
            }
            CoreEffect::PersistApprovalReview { correlation } => {
                self.spawn_approval_review_persistence(correlation);
                None
            }
            CoreEffect::SubmitTurn {
                correlation,
                request,
            } => {
                self.spawn_turn_submission(correlation, request);
                None
            }
            CoreEffect::RequestStopAllSessions {
                correlation,
                attempt,
            } => {
                let permit = self.stop_request_workers.register(correlation.generation);
                self.spawn_stop_request_attempt(correlation, attempt, permit);
                None
            }
            CoreEffect::InvalidateStopRequest { correlation } => {
                self.stop_request_workers.invalidate(correlation.generation);
                None
            }
            CoreEffect::SteerTurn {
                correlation,
                request,
            } => {
                self.spawn_turn_steer(correlation, request);
                None
            }
            CoreEffect::EvaluatePostTurn(request) => {
                self.spawn_post_turn_evaluation(*request);
                None
            }
        }
    }

    pub fn spawn_session_catalog_load(
        &self,
        correlation: SessionCatalogLoadCorrelation,
        limit: usize,
        workspace_directory: String,
    ) {
        let session_service = self.session_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let request = SessionCatalogRequest::for_workspace(limit, workspace_directory);
            let completion = session_catalog_completion(
                correlation,
                session_service.load_session_catalog(request),
            );
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_session_rename(&self, correlation: SessionRenameCorrelation) {
        let session_service = self.session_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = session_service.rename_session(correlation.request.clone());
            let completion = session_rename_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_conversation_load(
        &self,
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
    ) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = conversation_service.load_thread_snapshot(
                correlation.requested_thread_id.as_str(),
                fallback_workspace_directory.as_str(),
            );
            let completion = conversation_snapshot_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_parallel_peek_conversation_load(&self, correlation: ParallelPeekLoadCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result =
                conversation_service.load_snapshot(correlation.requested_thread_id.as_str());
            let completion = parallel_peek_conversation_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_review_center_load(&self, correlation: ReviewCenterLoadCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let snapshot = load_review_center_snapshot(&conversation_service, &correlation);
            let _ = input_sender.send(CoreInput::EffectCompleted(
                CoreEffectCompletion::ReviewCenterLoaded {
                    correlation,
                    snapshot,
                },
            ));
        });
    }

    pub fn spawn_queue_authority_load(&self, correlation: QueueAuthorityLoadCorrelation) {
        let planning_queue = self.planning_queue.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = queue_authority_result(
                planning_queue.load_coherent_authority(&correlation.workspace_directory),
            );
            let _ = input_sender.send(CoreInput::EffectCompleted(
                CoreEffectCompletion::QueueAuthorityLoaded {
                    correlation,
                    result: result.map(Box::new),
                },
            ));
        });
    }

    pub fn spawn_directions_maintenance_load(
        &self,
        correlation: DirectionsMaintenanceLoadCorrelation,
    ) {
        let planning_workspace = self.planning_workspace.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = directions_maintenance_result(
                planning_workspace.load_summary(&correlation.workspace_directory),
            );
            let _ = input_sender.send(CoreInput::EffectCompleted(
                CoreEffectCompletion::DirectionsMaintenanceLoaded {
                    correlation,
                    result,
                },
            ));
        });
    }

    pub fn spawn_planning_runtime_projection_load(
        &self,
        correlation: PlanningRuntimeRefreshCorrelation,
    ) {
        let planning_runtime = self.planning_runtime.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = planning_runtime
                .inspect_runtime_projection(&correlation.workspace_directory)
                .map(PlanningRuntimeRefreshSnapshot::new)
                .map(Box::new)
                .map_err(|error| error.to_string());
            let _ = input_sender.send(CoreInput::EffectCompleted(
                CoreEffectCompletion::PlanningRuntimeLoaded {
                    correlation,
                    result,
                },
            ));
        });
    }

    pub fn spawn_queue_mutation(&self, correlation: QueueMutationCorrelation) {
        let planning_queue = self.planning_queue.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let request = planning_queue_cancellation_request(&correlation.intent);
            let completion = queue_mutation_completion(
                correlation,
                planning_queue.execute_cancellation_transaction(request),
            );
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    fn spawn_github_review_poll(
        &self,
        service: GithubReviewPollerService,
        correlation: GithubReviewPollCorrelation,
        previous_state: Option<crate::domain::github_review::GithubPullRequestPollState>,
    ) {
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = service.poll(&correlation.target, previous_state.as_ref());
            let completion = github_review_poll_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    fn spawn_manual_prompt_preparation(
        &self,
        request: crate::domain::planning::ManualPromptRequest,
        permit: EffectExecutionPermit,
    ) {
        let service = self.manual_prompt_preparation_service.clone();
        let input_sender = self.input_sender.clone();
        let workers = self.manual_prompt_workers.clone();
        thread::spawn(move || {
            let generation = request.correlation.generation;
            let panic_correlation = request.correlation.clone();
            let panic_transcript = request.raw_prompt.trim().to_string();
            let result = catch_unwind(AssertUnwindSafe(|| {
                service.prepare_guarded(request, &|| permit.is_active())
            }))
            .unwrap_or_else(|_| {
                crate::domain::planning::ManualPromptOutcome::Rejected {
                    correlation: panic_correlation,
                    transcript_text: panic_transcript,
                    runtime_projection: Box::new(PlanningRuntimeProjection::invalid(
                        "manual prompt preparation worker panicked",
                    )),
                    reason: "manual prompt preparation worker panicked".to_string(),
                }
            });
            workers.finish(generation, &permit);
            let completion = CoreEffectCompletion::ManualPromptPrepared(Box::new(result));
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    fn spawn_stop_request_attempt(
        &self,
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
        permit: EffectExecutionPermit,
    ) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        let workers = self.stop_request_workers.clone();
        thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                if !permit.is_active() {
                    return Err(anyhow::anyhow!(
                        "stop request was superseded before provider execution"
                    ));
                }
                conversation_service.request_stop_all_sessions()
            }))
            .map_err(|_| anyhow::anyhow!("stop request worker panicked"))
            .and_then(|result| result);
            workers.finish(correlation.generation, &permit);
            let completion = stop_request_attempt_completion(correlation, attempt, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_approval_decision_submission(&self, correlation: ApprovalDecisionCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = conversation_service
                .resolve_approval_request(&correlation.approval_id, correlation.decision);
            let completion = approval_decision_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_approval_review_persistence(
        &self,
        correlation: ApprovalReviewPersistenceCorrelation,
    ) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = conversation_service.persist_review_center_approval_review_for_workspace(
                &correlation.workspace_directory,
                &correlation.thread_id,
                &correlation.review,
            );
            let completion = approval_review_persistence_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_turn_submission(
        &self,
        correlation: crate::core::app::TurnSubmissionCorrelation,
        request: crate::core::app::TurnSubmissionRequest,
    ) {
        core_turn_submission::spawn_turn_submission_worker(
            correlation,
            request,
            self.conversation_service.clone(),
            self.planning_runtime.clone(),
            self.parallel_mode_turn_service.clone(),
            self.input_sender.clone(),
        );
    }

    pub fn spawn_turn_steer(
        &self,
        correlation: crate::core::app::TurnSteerCorrelation,
        request: crate::domain::conversation::ConversationTurnSteerRequest,
    ) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let completion =
                turn_steer_completion(correlation, conversation_service.steer_turn(request));
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_post_turn_evaluation(&self, request: crate::domain::planning::PostTurnRequest) {
        let service = self.post_turn_evaluation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let execution = service.evaluate_with_timeout(request, POST_TURN_EVALUATION_TIMEOUT);
            let _ = input_sender.send(CoreInput::EffectCompleted(
                CoreEffectCompletion::PostTurnEvaluationCompleted(Box::new(execution)),
            ));
        });
    }
}

impl CoreEffectExecutor for CoreEffectRunner {
    fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
        CoreEffectRunner::run_effect(self, effect)
    }
}

fn startup_checks_completion(
    correlation: StartupCheckCorrelation,
    result: Result<StartupDiagnostics>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::StartupChecksLoaded {
        correlation,
        result: result
            .map(StartupReadySnapshot::from_diagnostics)
            .map(Box::new)
            .map_err(|error| format!("{error:#}")),
    }
}

fn session_catalog_completion(
    correlation: SessionCatalogLoadCorrelation,
    result: Result<SessionCatalog>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::SessionCatalogLoaded {
        correlation,
        result: result
            .map(SessionCatalogReadySnapshot::from_catalog)
            .map_err(|error| error.to_string()),
    }
}

fn session_rename_completion(
    correlation: SessionRenameCorrelation,
    result: Result<()>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::SessionRenamed {
        correlation,
        result: result.map_err(|error| error.to_string()),
    }
}

fn conversation_snapshot_completion(
    correlation: ConversationLoadCorrelation,
    result: Result<LoadedConversationThreadSnapshot>,
) -> CoreEffectCompletion {
    let requested_thread_id = correlation.requested_thread_id.clone();
    CoreEffectCompletion::ConversationLoaded {
        correlation,
        result: result
            .and_then(|snapshot| {
                if snapshot.conversation.thread_id == requested_thread_id {
                    Ok(snapshot)
                } else {
                    Err(anyhow::anyhow!(
                        "conversation provider returned a different thread"
                    ))
                }
            })
            .map(conversation_ready_snapshot)
            .map(Box::new)
            .map_err(|error| error.to_string()),
    }
}

fn parallel_peek_conversation_completion(
    correlation: ParallelPeekLoadCorrelation,
    result: Result<crate::domain::conversation::ConversationSnapshot>,
) -> CoreEffectCompletion {
    let requested_thread_id = correlation.requested_thread_id.clone();
    let result = result
        .and_then(|snapshot| {
            if snapshot.thread_id == requested_thread_id {
                Ok(snapshot)
            } else {
                Err(anyhow::anyhow!(
                    "conversation provider returned a different thread"
                ))
            }
        })
        .map(ConversationReadySnapshot::from)
        .map(Box::new)
        .map_err(|error| error.to_string());
    CoreEffectCompletion::ParallelPeekConversationLoaded {
        correlation,
        result,
    }
}

fn turn_steer_completion(
    correlation: crate::core::app::TurnSteerCorrelation,
    result: Result<crate::domain::conversation::ConversationTurnSteerReceipt>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::TurnSteered {
        correlation,
        result: result.map_err(|error| error.to_string()),
    }
}

fn approval_decision_completion(
    correlation: ApprovalDecisionCorrelation,
    result: Result<()>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::ApprovalDecisionSubmitted {
        correlation,
        result: result.map_err(|error| error.to_string()),
    }
}

fn approval_review_persistence_completion(
    correlation: ApprovalReviewPersistenceCorrelation,
    result: Result<()>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::ApprovalReviewPersisted {
        correlation,
        result: result.map_err(|error| error.to_string()),
    }
}

fn github_review_poll_completion(
    correlation: GithubReviewPollCorrelation,
    result: Result<crate::domain::github_review::GithubPullRequestPollResult>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::GithubReviewPollCompleted {
        correlation,
        result: result.map(Box::new).map_err(|error| error.to_string()),
    }
}

fn load_review_center_snapshot(
    conversation_service: &ConversationService,
    correlation: &ReviewCenterLoadCorrelation,
) -> ReviewCenterSnapshot {
    let current_thread_reviews = match correlation.active_thread_id.as_deref() {
        Some(thread_id) => conversation_service.load_review_center_thread_reviews_for_workspace(
            &correlation.workspace_directory,
            thread_id,
        ),
        None => Ok(Vec::new()),
    };
    let pending_inbox = conversation_service
        .load_review_center_pending_inbox_for_workspace(&correlation.workspace_directory);
    let recent_history = conversation_service
        .load_review_center_recent_history_for_workspace(&correlation.workspace_directory);
    review_center_snapshot(current_thread_reviews, pending_inbox, recent_history)
}

fn review_center_snapshot(
    current_thread_reviews: Result<Vec<ReviewCenterThreadProjection>>,
    pending_inbox: Result<Vec<ReviewCenterInboxItem>>,
    recent_history: Result<Vec<ReviewCenterHistoryEntry>>,
) -> ReviewCenterSnapshot {
    ReviewCenterSnapshot {
        current_thread_reviews: current_thread_reviews
            .map(|reviews| {
                reviews
                    .into_iter()
                    .map(review_center_thread_snapshot)
                    .collect()
            })
            .map_err(|error| error.to_string()),
        pending_inbox: pending_inbox
            .map(|items| {
                items
                    .into_iter()
                    .map(review_center_inbox_item_snapshot)
                    .collect()
            })
            .map_err(|error| error.to_string()),
        recent_history: recent_history
            .map(|entries| {
                entries
                    .into_iter()
                    .map(review_center_history_entry_snapshot)
                    .collect()
            })
            .map_err(|error| error.to_string()),
    }
}

fn review_center_thread_snapshot(
    review: ReviewCenterThreadProjection,
) -> ConversationThreadReviewSnapshot {
    ConversationThreadReviewSnapshot {
        thread_id: review.thread_id,
        review_id: review.review_id,
        review_label: review.review_label,
        review_state: review.review_state,
        review_summary: review.review_summary,
        requested_at: review.requested_at,
        updated_at: review.updated_at,
        handoff_target: review.handoff_target,
        handoff_note: review.handoff_note,
    }
}

fn review_center_inbox_item_snapshot(item: ReviewCenterInboxItem) -> ReviewCenterInboxItemSnapshot {
    ReviewCenterInboxItemSnapshot {
        review_id: item.review_id,
        thread_id: item.thread_id,
        inbox_state: item.inbox_state,
        summary: item.summary,
        requested_at: item.requested_at,
        last_activity_at: item.last_activity_at,
        handoff_target: item.handoff_target,
    }
}

fn review_center_history_entry_snapshot(
    entry: ReviewCenterHistoryEntry,
) -> ReviewCenterHistoryEntrySnapshot {
    ReviewCenterHistoryEntrySnapshot {
        review_id: entry.review_id,
        thread_id: entry.thread_id,
        event_kind: entry.event_kind,
        summary: entry.summary,
        recorded_at: entry.recorded_at,
    }
}

fn queue_authority_result(
    result: Result<PlanningQueueAuthorityProjection, PlanningQueueAuthorityRefreshError>,
) -> Result<QueueAuthoritySnapshot, QueueAuthorityLoadError> {
    result
        .map(|authority| QueueAuthoritySnapshot {
            runtime_projection: authority.runtime_projection,
            planning_revision: authority.queue_authority.planning_revision,
            tasks: authority.queue_authority.tasks,
        })
        .map_err(|error| match error {
            PlanningQueueAuthorityRefreshError::AuthorityUnavailable(detail) => {
                QueueAuthorityLoadError::AuthorityUnavailable(detail)
            }
            PlanningQueueAuthorityRefreshError::RevisionsKeptChanging {
                projection_revision,
                authority_revision,
            } => QueueAuthorityLoadError::RevisionsKeptChanging {
                projection_revision,
                authority_revision,
            },
            PlanningQueueAuthorityRefreshError::RuntimeProjectionUnavailable => {
                QueueAuthorityLoadError::RuntimeProjectionUnavailable
            }
        })
}

fn directions_maintenance_result(
    result: Result<ApplicationDirectionsMaintenanceSummary>,
) -> Result<Box<DirectionsMaintenanceSummarySnapshot>, String> {
    result
        .map(directions_maintenance_summary_snapshot)
        .map(Box::new)
        .map_err(|error| error.to_string())
}

fn directions_maintenance_summary_snapshot(
    summary: ApplicationDirectionsMaintenanceSummary,
) -> DirectionsMaintenanceSummarySnapshot {
    DirectionsMaintenanceSummarySnapshot {
        directions: summary
            .directions
            .into_iter()
            .map(|direction| DirectionsMaintenanceDirectionSnapshot {
                id: direction.id,
                title: direction.title,
                detail_doc_path: direction.detail_doc_path,
                detail_doc_status: directions_supporting_file_status_snapshot(
                    direction.detail_doc_status,
                ),
            })
            .collect(),
        missing_detail_doc_count: summary.missing_detail_doc_count,
        broken_detail_doc_count: summary.broken_detail_doc_count,
        queue_idle_policy: summary.queue_idle_policy,
        queue_idle_prompt_path: summary.queue_idle_prompt_path,
        queue_idle_prompt_status: directions_supporting_file_status_snapshot(
            summary.queue_idle_prompt_status,
        ),
        parse_error: summary.parse_error,
    }
}

fn directions_supporting_file_status_snapshot(
    status: ApplicationDirectionsSupportingFileStatus,
) -> CoreDirectionsSupportingFileStatus {
    match status {
        ApplicationDirectionsSupportingFileStatus::MissingMapping => {
            CoreDirectionsSupportingFileStatus::MissingMapping
        }
        ApplicationDirectionsSupportingFileStatus::Ready => {
            CoreDirectionsSupportingFileStatus::Ready
        }
        ApplicationDirectionsSupportingFileStatus::BrokenMapping => {
            CoreDirectionsSupportingFileStatus::BrokenMapping
        }
    }
}

fn planning_queue_cancellation_request(
    intent: &QueueMutationIntent,
) -> PlanningQueueCancellationRequest {
    PlanningQueueCancellationRequest {
        workspace_directory: intent.workspace_directory.clone(),
        expected_planning_revision: intent.expected_planning_revision,
        targets: intent
            .targets
            .iter()
            .map(|target| PlanningQueueCancellationTarget {
                task_id: target.task_id.clone(),
                expected_status: target.expected_status,
                expected_updated_at: target.expected_updated_at.clone(),
            })
            .collect(),
    }
}

fn queue_mutation_commit_snapshot(
    result: PlanningTaskMutationCommitResult,
) -> QueueMutationCommitSnapshot {
    QueueMutationCommitSnapshot {
        committed_planning_revision: result.committed_planning_revision,
        committed_task_ids: result.committed_task_ids,
    }
}

fn queue_mutation_result(
    result: PlanningQueueCancellationTransactionResult,
) -> QueueMutationResult {
    QueueMutationResult {
        mutation: result
            .mutation
            .map(queue_mutation_commit_snapshot)
            .map_err(|error| error.to_string()),
        authority: queue_authority_result(result.authority),
    }
}

fn queue_mutation_completion(
    correlation: QueueMutationCorrelation,
    result: PlanningQueueCancellationTransactionResult,
) -> CoreEffectCompletion {
    CoreEffectCompletion::QueueMutationCompleted {
        correlation,
        result: Box::new(queue_mutation_result(result)),
    }
}

fn stop_request_attempt_completion(
    correlation: StopRequestCorrelation,
    attempt: StopRequestAttempt,
    result: anyhow::Result<()>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::StopRequestAttemptCompleted {
        correlation,
        attempt,
        result: result.map_err(|error| error.to_string()),
    }
}

fn conversation_ready_snapshot(
    snapshot: LoadedConversationThreadSnapshot,
) -> ConversationReadySnapshot {
    ConversationReadySnapshot::from_parts(
        snapshot.conversation,
        snapshot
            .thread_review
            .into_iter()
            .map(review_center_thread_snapshot)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::adapter::outbound::github::GithubAutomationAdapter;
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::{
        AppServerStartupContext, StartupProbePort,
    };
    use crate::application::service::parallel_mode::{
        ParallelModeService, turn::ParallelModeTurnService,
    };
    use crate::application::service::planning::{
        DirectionsMaintenanceDirectionSummary as ApplicationDirectionsMaintenanceDirectionSummary,
        PlanningQueueAuthoritySnapshot,
    };
    use crate::core::app::{
        AppCommand, AppEvent, CoreDispatchOutcome, CorePromptOrigin,
        ManualPromptPreparationAdmission, ManualPromptPreparationIntent, QueueMutationKind,
        QueueMutationTarget, StopRequestAdmission, TurnStreamEvent, TurnSubmissionAdmission,
        TurnSubmissionCorrelation, TurnSubmissionRequest,
    };
    use crate::core::runtime::{CoreRuntime, core_input_channel};
    use crate::domain::conversation::{
        ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessage,
        ConversationMessageKind, ConversationRuntimeControlTruth, ConversationSnapshot,
    };
    use crate::domain::planning::{
        ManualPromptCorrelation, ManualPromptOutcome, QueueIdlePolicy, RuntimeProjection,
        TaskActor, TaskDefinition, TaskMutationProvenance, TaskStatus,
    };
    use crate::domain::recent_sessions::{
        RecentSessions, SessionCatalogRequest, SessionCatalogTier, SessionRenameRequest,
    };
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    const NONBLOCKING_DISPATCH_TIMEOUT: Duration = Duration::from_secs(1);
    const WORKER_COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);

    struct OneShotGate {
        armed: AtomicBool,
        entered: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl OneShotGate {
        fn wait_once(&self) {
            if !self.armed.swap(false, Ordering::SeqCst) {
                return;
            }
            let _ = self.entered.send(());
            let _ = self
                .release
                .lock()
                .expect("gate release mutex should not be poisoned")
                .recv();
        }
    }

    fn one_shot_gate() -> (Arc<OneShotGate>, mpsc::Receiver<()>, mpsc::SyncSender<()>) {
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        (
            Arc::new(OneShotGate {
                armed: AtomicBool::new(true),
                entered: entered_tx,
                release: Mutex::new(release_rx),
            }),
            entered_rx,
            release_tx,
        )
    }

    struct GatedPlanningWorkspacePort {
        load_gate: Arc<OneShotGate>,
        stage_call_count: Arc<AtomicUsize>,
        promote_call_count: Arc<AtomicUsize>,
        panic_load_once: AtomicBool,
    }

    impl PlanningWorkspacePort for GatedPlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            self.stage_call_count.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("synthetic stage should not complete")
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            self.promote_call_count.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("synthetic promote should not complete")
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            unreachable!("manual preparation should stop after the gated authority load error")
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            if self.panic_load_once.swap(false, Ordering::SeqCst) {
                panic!("synthetic manual preparation provider panic");
            }
            self.load_gate.wait_once();
            anyhow::bail!("manual preparation authority unavailable after gate release")
        }

        fn load_planning_workspace_candidate_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            unreachable!("manual preparation should stop after the gated authority load error")
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            _record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            unreachable!("manual preparation should stop after the gated authority load error")
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            unreachable!("manual preparation should stop after the gated authority load error")
        }

        fn load_optional_planning_candidate_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            unreachable!("manual preparation should stop after the gated authority load error")
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
            _body: Option<&str>,
        ) -> Result<()> {
            unreachable!("manual preparation should stop after the gated authority load error")
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<()> {
            unreachable!("manual preparation should stop after the gated authority load error")
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            unreachable!("manual preparation should stop after the gated authority load error")
        }
    }

    #[derive(Default)]
    struct GatedRuntimePort {
        stop_gate: Option<Arc<OneShotGate>>,
        stop_call_count: AtomicUsize,
        panic_stop_once: AtomicBool,
    }

    impl GatedRuntimePort {
        fn with_stop_gate(stop_gate: Arc<OneShotGate>) -> Self {
            Self {
                stop_gate: Some(stop_gate),
                stop_call_count: AtomicUsize::new(0),
                panic_stop_once: AtomicBool::new(false),
            }
        }

        fn panicking_stop_once() -> Self {
            Self {
                stop_gate: None,
                stop_call_count: AtomicUsize::new(0),
                panic_stop_once: AtomicBool::new(true),
            }
        }
    }

    impl StartupProbePort for GatedRuntimePort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
                initialize_detail: "ready".to_string(),
                account_detail: "ready".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }
    }

    impl SessionCatalogPort for GatedRuntimePort {
        fn load_session_catalog(&self, _request: SessionCatalogRequest) -> Result<SessionCatalog> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }
    }

    impl InteractiveTurnRuntimePort for GatedRuntimePort {
        fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
            ConversationRuntimeControlTruth::codex_app_server()
        }

        fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "test".to_string(),
                cwd: "/tmp/workspace".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            })
        }

        fn request_stop_all_sessions(&self) -> Result<()> {
            self.stop_call_count.fetch_add(1, Ordering::SeqCst);
            if self.panic_stop_once.swap(false, Ordering::SeqCst) {
                panic!("synthetic stop provider panic");
            }
            if let Some(gate) = &self.stop_gate {
                gate.wait_once();
            }
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            _event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
        ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
            anyhow::bail!("conversation streaming is unused by this effect-runner test")
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            _event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
        ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
            anyhow::bail!("conversation streaming is unused by this effect-runner test")
        }
    }

    fn test_effect_runner(
        planning_workspace: Arc<dyn PlanningWorkspacePort>,
        runtime_port: Arc<GatedRuntimePort>,
        input_sender: CoreInputSender,
    ) -> CoreEffectRunner {
        let planning = PlanningServices::from_ports(
            planning_workspace,
            Arc::new(NoopPlanningAuthorityPort::default()),
            Arc::new(NoopPlanningTaskRepositoryPort),
            Arc::new(NoopPlanningWorkerPort),
        );
        let parallel_turns = ParallelModeTurnService::new(ParallelModeService::new(
            Arc::new(NoopPlanningAuthorityPort::default()),
            Arc::new(GithubAutomationAdapter::new()),
            Arc::new(GitParallelModeRuntimeAdapter::new()),
        ));
        CoreEffectRunner::new(
            StartupService::new(runtime_port.clone()),
            SessionService::new(runtime_port.clone()),
            ConversationService::new(runtime_port),
            planning.clone(),
            parallel_turns.clone(),
            PostTurnEvaluationService::new(planning, parallel_turns),
            input_sender,
        )
    }

    fn dispatch_while_provider_is_gated(
        runtime: CoreRuntime<CoreEffectRunner>,
        command: AppCommand,
        gate_entered: mpsc::Receiver<()>,
        gate_release: mpsc::SyncSender<()>,
    ) -> (CoreRuntime<CoreEffectRunner>, CoreDispatchOutcome, bool) {
        let (dispatch_tx, dispatch_rx) = mpsc::sync_channel(1);
        let dispatcher = thread::spawn(move || {
            let mut runtime = runtime;
            let outcome = runtime.dispatch_command(command);
            let _ = dispatch_tx.send((runtime, outcome));
        });

        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("provider call should reach its gate");
        let dispatched = dispatch_rx.recv_timeout(NONBLOCKING_DISPATCH_TIMEOUT);
        let returned_before_release = dispatched.is_ok();
        let _ = gate_release.send(());
        let (runtime, outcome) = match dispatched {
            Ok(dispatched) => dispatched,
            Err(_) => dispatch_rx
                .recv_timeout(WORKER_COMPLETION_TIMEOUT)
                .expect("dispatch should finish after the provider gate is released"),
        };
        dispatcher
            .join()
            .expect("effect dispatch thread should not panic");
        (runtime, outcome, returned_before_release)
    }

    fn poll_until(
        runtime: &mut CoreRuntime<CoreEffectRunner>,
        matches: impl Fn(&CoreDispatchOutcome) -> bool,
    ) -> CoreDispatchOutcome {
        let deadline = Instant::now() + WORKER_COMPLETION_TIMEOUT;
        loop {
            if let Some(outcome) = runtime.poll_pending_input()
                && matches(&outcome)
            {
                return outcome;
            }
            assert!(
                Instant::now() < deadline,
                "background effect completion should re-enter Core before the deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn poll_next(runtime: &mut CoreRuntime<CoreEffectRunner>) -> CoreDispatchOutcome {
        let deadline = Instant::now() + WORKER_COMPLETION_TIMEOUT;
        loop {
            if let Some(outcome) = runtime.poll_pending_input() {
                return outcome;
            }
            assert!(
                Instant::now() < deadline,
                "background effect completion should re-enter Core before the deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn manual_correlation(generation: u64, workspace_directory: &str) -> ManualPromptCorrelation {
        ManualPromptCorrelation {
            request_id: generation,
            generation,
            workspace_directory: workspace_directory.to_string(),
        }
    }

    fn rejected_manual_outcome(correlation: ManualPromptCorrelation) -> ManualPromptOutcome {
        ManualPromptOutcome::Rejected {
            correlation,
            transcript_text: String::new(),
            runtime_projection: Box::new(RuntimeProjection::invalid("test completion")),
            reason: "test completion".to_string(),
        }
    }

    fn startup_correlation() -> StartupCheckCorrelation {
        StartupCheckCorrelation::new(7)
    }

    fn session_catalog_correlation() -> SessionCatalogLoadCorrelation {
        SessionCatalogLoadCorrelation::new(8)
    }

    fn session_rename_correlation() -> SessionRenameCorrelation {
        SessionRenameCorrelation::new(9, SessionRenameRequest::new("thread-1", "Renamed"))
    }

    fn conversation_correlation(thread_id: &str) -> ConversationLoadCorrelation {
        ConversationLoadCorrelation::new(9, thread_id)
    }

    fn turn_steer_correlation() -> crate::core::app::TurnSteerCorrelation {
        crate::core::app::TurnSteerCorrelation::new(
            3,
            crate::core::app::TurnSubmissionCorrelation::new(2),
        )
    }

    fn approval_decision_correlation() -> ApprovalDecisionCorrelation {
        ApprovalDecisionCorrelation::new(
            4,
            crate::core::app::TurnSubmissionCorrelation::new(2),
            "approval-1",
            crate::domain::conversation::ConversationApprovalDecision::Accept,
        )
    }

    fn approval_review_persistence_correlation() -> ApprovalReviewPersistenceCorrelation {
        ApprovalReviewPersistenceCorrelation::new(
            5,
            TurnSubmissionCorrelation::new(2),
            "/tmp/workspace",
            "thread-1",
            ConversationApprovalReview {
                target_item_id: "tool-1".to_string(),
                status: ConversationApprovalReviewStatus::InProgress,
                risk_level: Some("medium".to_string()),
                rationale: Some("needs approval".to_string()),
            },
        )
    }

    fn github_review_poll_correlation() -> GithubReviewPollCorrelation {
        GithubReviewPollCorrelation::new(
            5,
            crate::domain::github_review::GithubPullRequestTarget::new("acme/widgets", 42),
        )
    }

    fn github_review_poll_result() -> crate::domain::github_review::GithubPullRequestPollResult {
        crate::domain::github_review::GithubPullRequestPollResult {
            snapshot: crate::domain::github_review::GithubPullRequestActivitySnapshot {
                target: crate::domain::github_review::GithubPullRequestTarget::new(
                    "acme/widgets",
                    42,
                ),
                title: "Move polling authority".to_string(),
                url: "https://example.invalid/acme/widgets/pull/42".to_string(),
                head_branch: "refactor/poll".to_string(),
                base_branch: "prerelease".to_string(),
                events: Vec::new(),
            },
            changes: Vec::new(),
            next_state: crate::domain::github_review::GithubPullRequestPollState::default(),
        }
    }

    fn queue_task() -> TaskDefinition {
        TaskDefinition {
            id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_relation_note: String::new(),
            title: "Review queue authority".to_string(),
            description: "Preserve the authoritative queue row".to_string(),
            status: TaskStatus::Ready,
            base_priority: 50,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: TaskMutationProvenance::default(),
            updated_at: "2026-07-19T00:00:00Z".to_string(),
        }
    }

    fn queue_mutation_intent() -> QueueMutationIntent {
        QueueMutationIntent {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-1".to_string()),
            kind: QueueMutationKind::RemoveSelected,
            expected_planning_revision: 41,
            targets: vec![QueueMutationTarget {
                task_id: "task-1".to_string(),
                expected_status: TaskStatus::Ready,
                expected_updated_at: "2026-07-19T00:00:00Z".to_string(),
            }],
            receipt_at_start: None,
        }
    }

    #[test]
    fn manual_prompt_preparation_dispatch_returns_before_blocked_authority_work() {
        let workspace_directory = "/tmp/gated-manual-preparation";
        let correlation = manual_correlation(1, workspace_directory);
        let (planning_gate, gate_entered, gate_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: planning_gate,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let runtime = CoreRuntime::new(runner, input_receiver);

        let (mut runtime, accepted, returned_before_release) = dispatch_while_provider_is_gated(
            runtime,
            AppCommand::PrepareManualPrompt(Box::new(ManualPromptPreparationIntent {
                workspace_directory: workspace_directory.to_string(),
                raw_prompt: "   ".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            })),
            gate_entered,
            gate_release,
        );
        assert!(
            returned_before_release,
            "manual prompt dispatch must return while authority work remains blocked"
        );
        assert_eq!(
            accepted.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted {
                    correlation: correlation.clone(),
                },
            )]
        );

        let stale = runtime.dispatch_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(Box::new(rejected_manual_outcome(
                manual_correlation(99, workspace_directory),
            ))),
        ));
        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());
        let duplicate_command = runtime.dispatch_command(AppCommand::PrepareManualPrompt(
            Box::new(ManualPromptPreparationIntent {
                workspace_directory: workspace_directory.to_string(),
                raw_prompt: "duplicate".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            }),
        ));
        assert_eq!(
            duplicate_command.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::RejectedActive {
                    active_correlation: correlation.clone(),
                },
            )]
        );

        let completed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::ManualPromptPrepared(result)]
                    if result.correlation() == &correlation
            )
        });
        assert!(matches!(
            completed.events.as_slice(),
            [AppEvent::ManualPromptPrepared(result)]
                if result.correlation() == &correlation
        ));

        let duplicate_completion = runtime.dispatch_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(Box::new(rejected_manual_outcome(
                correlation,
            ))),
        ));
        assert!(duplicate_completion.events.is_empty());
        assert!(duplicate_completion.effects.is_empty());
    }

    #[test]
    fn cancelled_manual_preparation_keeps_its_lease_and_blocks_stale_bootstrap_side_effects() {
        let workspace_a = "/tmp/gated-manual-a";
        let workspace_b = "/tmp/gated-manual-b";
        let (planning_gate, gate_entered, gate_release) = one_shot_gate();
        let stage_call_count = Arc::new(AtomicUsize::new(0));
        let promote_call_count = Arc::new(AtomicUsize::new(0));
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: planning_gate,
            stage_call_count: stage_call_count.clone(),
            promote_call_count: promote_call_count.clone(),
            panic_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);

        let accepted = runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(
            ManualPromptPreparationIntent {
                workspace_directory: workspace_a.to_string(),
                raw_prompt: "old A prompt".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            },
        )));
        assert!(matches!(
            accepted.events.as_slice(),
            [AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted { correlation }
            )] if correlation == &manual_correlation(1, workspace_a)
        ));
        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("manual preparation should block at the authority gate");

        let cancelled = runtime.dispatch_command(AppCommand::CancelManualPromptPreparation);
        assert_eq!(
            cancelled.effects,
            vec![CoreEffect::CancelManualPromptPreparation {
                correlation: manual_correlation(1, workspace_a),
            }]
        );
        let blocked_b = runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(
            ManualPromptPreparationIntent {
                workspace_directory: workspace_b.to_string(),
                raw_prompt: "B prompt".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            },
        )));
        assert!(matches!(
            blocked_b.events.as_slice(),
            [AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::RejectedActive { active_correlation }
            )] if active_correlation == &manual_correlation(1, workspace_a)
        ));

        gate_release
            .send(())
            .expect("cancelled provider gate should release");
        let cancelled_completion = poll_next(&mut runtime);
        assert!(cancelled_completion.events.is_empty());
        assert!(cancelled_completion.effects.is_empty());
        assert_eq!(stage_call_count.load(Ordering::SeqCst), 0);
        assert_eq!(promote_call_count.load(Ordering::SeqCst), 0);

        let accepted_b = runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(
            ManualPromptPreparationIntent {
                workspace_directory: workspace_b.to_string(),
                raw_prompt: "B prompt".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            },
        )));
        assert!(matches!(
            accepted_b.events.as_slice(),
            [AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted { correlation }
            )] if correlation == &manual_correlation(2, workspace_b)
        ));
        let _ = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::ManualPromptPrepared(result)]
                    if result.correlation() == &manual_correlation(2, workspace_b)
            )
        });

        let accepted_a_again = runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(
            ManualPromptPreparationIntent {
                workspace_directory: workspace_a.to_string(),
                raw_prompt: "new A prompt".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            },
        )));
        assert!(matches!(
            accepted_a_again.events.as_slice(),
            [AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted { correlation }
            )] if correlation == &manual_correlation(3, workspace_a)
        ));
    }

    #[test]
    fn stop_dispatch_returns_before_provider_and_preserves_pre_turn_synchronization() {
        let (stop_gate, gate_entered, gate_release) = one_shot_gate();
        let runtime_port = Arc::new(GatedRuntimePort::with_stop_gate(stop_gate));
        let (unused_planning_gate, _unused_entered, _unused_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_planning_gate,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
        });
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port.clone(), input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);
        let turn_submission = runtime.begin_test_turn_submission();
        let correlation = StopRequestCorrelation::new(1, Some(turn_submission));

        let (mut runtime, accepted, returned_before_release) = dispatch_while_provider_is_gated(
            runtime,
            AppCommand::RequestStopAllSessions,
            gate_entered,
            gate_release,
        );
        assert!(
            returned_before_release,
            "stop dispatch must return while provider I/O remains blocked"
        );
        assert_eq!(
            accepted.events,
            vec![AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::Accepted { correlation },
            )]
        );

        let stale = runtime.dispatch_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation: StopRequestCorrelation::new(99, Some(turn_submission)),
                attempt: StopRequestAttempt::Initial,
                result: Ok(()),
            },
        ));
        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());
        let duplicate_command = runtime.dispatch_command(AppCommand::RequestStopAllSessions);
        assert_eq!(
            duplicate_command.events,
            vec![AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::RejectedActive {
                    active_correlation: correlation,
                },
            )]
        );

        let started = runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        });
        assert!(
            started.effects.is_empty(),
            "TurnStarted must wait for the in-flight initial stop attempt"
        );

        let initial_completed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::StopRequestAttemptCompleted {
                    correlation: completed,
                    attempt: StopRequestAttempt::Initial,
                    result: Ok(()),
                }] if *completed == correlation
            )
        });
        assert_eq!(
            initial_completed.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation,
                attempt: StopRequestAttempt::AfterTurnStarted,
            }]
        );

        let duplicate_initial = runtime.dispatch_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt: StopRequestAttempt::Initial,
                result: Ok(()),
            },
        ));
        assert!(duplicate_initial.events.is_empty());
        assert!(duplicate_initial.effects.is_empty());

        let synchronized = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::StopRequestAttemptCompleted {
                    correlation: completed,
                    attempt: StopRequestAttempt::AfterTurnStarted,
                    result: Ok(()),
                }] if *completed == correlation
            )
        });
        assert!(synchronized.effects.is_empty());
        assert_eq!(runtime_port.stop_call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stop_settlement_precedes_a_new_turn_and_deferred_session_load() {
        let (stop_gate, gate_entered, gate_release) = one_shot_gate();
        let runtime_port = Arc::new(GatedRuntimePort::with_stop_gate(stop_gate));
        let (unused_planning_gate, _unused_entered, _unused_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_planning_gate,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
        });
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port.clone(), input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);
        let turn_request = TurnSubmissionRequest {
            workspace_directory: "/tmp/new-workspace".to_string(),
            thread_id: Some("thread-new".to_string()),
            prompt: "new work".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        };

        let stop = runtime.dispatch_command(AppCommand::RequestStopAllSessions);
        assert!(matches!(
            stop.events.as_slice(),
            [AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::Accepted { correlation }
            )] if *correlation == StopRequestCorrelation::new(1, None)
        ));
        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("stop provider should reach its gate");

        let blocked_turn = runtime.dispatch_command(AppCommand::SubmitTurn(turn_request.clone()));
        assert_eq!(
            blocked_turn.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedStopPending {
                    stop_correlation: StopRequestCorrelation::new(1, None),
                },
            )]
        );
        let deferred_load = runtime.dispatch_command(AppCommand::LoadConversation {
            thread_id: "thread-new".to_string(),
            fallback_workspace_directory: "/tmp/new-workspace".to_string(),
        });
        assert_eq!(
            deferred_load.effects,
            vec![CoreEffect::InvalidateStopRequest {
                correlation: StopRequestCorrelation::new(1, None),
            }]
        );
        assert!(
            !deferred_load
                .effects
                .iter()
                .any(|effect| matches!(effect, CoreEffect::LoadConversation { .. }))
        );

        gate_release
            .send(())
            .expect("stop provider gate should release");
        let settled = poll_next(&mut runtime);
        assert_eq!(runtime_port.stop_call_count.load(Ordering::SeqCst), 1);
        assert!(
            settled
                .events
                .iter()
                .all(|event| !matches!(event, AppEvent::StopRequestAttemptCompleted { .. }))
        );
        assert!(settled.effects.iter().any(|effect| matches!(
            effect,
            CoreEffect::LoadConversation { correlation, .. }
                if correlation.requested_thread_id == "thread-new"
        )));

        let _ = poll_until(&mut runtime, |outcome| {
            outcome.events.iter().any(|event| {
                matches!(
                    event,
                    AppEvent::ConversationChanged { snapshot, .. }
                        if matches!(snapshot, crate::core::app::ConversationSnapshot::Ready(_))
                )
            })
        });
        let admitted_turn = runtime.dispatch_command(AppCommand::SubmitTurn(turn_request));
        assert!(matches!(
            admitted_turn.events.as_slice(),
            [AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::Accepted { .. }
            )]
        ));
    }

    #[test]
    fn manual_and_stop_worker_panics_return_exact_failures_and_reopen_admission() {
        let (manual_gate, _manual_entered, manual_release) = one_shot_gate();
        manual_release
            .send(())
            .expect("manual fallback gate should start open");
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: manual_gate,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(true),
        });
        let runtime_port = Arc::new(GatedRuntimePort::panicking_stop_once());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port.clone(), input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);

        let first_manual_correlation = manual_correlation(1, "/tmp/panicking-manual");
        runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(
            ManualPromptPreparationIntent {
                workspace_directory: first_manual_correlation.workspace_directory.clone(),
                raw_prompt: "survive the panic".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            },
        )));
        let manual_failed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::ManualPromptPrepared(result)]
                    if result.correlation() == &first_manual_correlation
            )
        });
        assert!(matches!(
            manual_failed.events.as_slice(),
            [AppEvent::ManualPromptPrepared(result)]
                if matches!(
                    result.as_ref(),
                    ManualPromptOutcome::Rejected { reason, .. }
                        if reason == "manual prompt preparation worker panicked"
                )
        ));
        let manual_retry = runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(
            ManualPromptPreparationIntent {
                workspace_directory: "/tmp/manual-retry".to_string(),
                raw_prompt: "retry".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            },
        )));
        assert!(matches!(
            manual_retry.events.as_slice(),
            [AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted { correlation }
            )] if correlation == &manual_correlation(2, "/tmp/manual-retry")
        ));
        let _ = poll_next(&mut runtime);

        let stop_correlation = StopRequestCorrelation::new(1, None);
        runtime.dispatch_command(AppCommand::RequestStopAllSessions);
        let stop_failed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::StopRequestAttemptCompleted {
                    correlation,
                    attempt: StopRequestAttempt::Initial,
                    result: Err(error),
                }] if *correlation == stop_correlation && error == "stop request worker panicked"
            )
        });
        assert!(matches!(
            stop_failed.events.as_slice(),
            [AppEvent::StopRequestAttemptCompleted {
                correlation,
                result: Err(error),
                ..
            }] if *correlation == stop_correlation
                && error == "stop request worker panicked"
        ));
        let stop_retry = runtime.dispatch_command(AppCommand::RequestStopAllSessions);
        assert!(matches!(
            stop_retry.events.as_slice(),
            [AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::Accepted { correlation }
            )] if *correlation == StopRequestCorrelation::new(2, None)
        ));
    }

    #[test]
    fn startup_success_maps_to_core_completion() {
        let diagnostics = StartupDiagnostics {
            cwd: "/tmp/workspace".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "/usr/bin/codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/workspace".to_string(),
            workspace_detail: "git repo: /tmp/workspace".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::default(),
            initialize_ok: true,
            initialize_detail: "initialized".to_string(),
            account_ok: true,
            account_detail: "authenticated".to_string(),
            warnings: Vec::new(),
            schema_snapshot: "embedded schema".to_string(),
        };

        assert_eq!(
            startup_checks_completion(startup_correlation(), Ok(diagnostics)),
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_correlation(),
                result: Ok(Box::new(StartupReadySnapshot {
                    cwd: "/tmp/workspace".to_string(),
                    workspace_path: "/tmp/workspace".to_string(),
                    can_continue: true,
                    codex_binary: crate::core::app::StartupDiagnosticSnapshot {
                        ok: true,
                        detail: "/usr/bin/codex".to_string(),
                    },
                    workspace: crate::core::app::StartupDiagnosticSnapshot {
                        ok: true,
                        detail: "git repo: /tmp/workspace".to_string(),
                    },
                    app_server_initialize: crate::core::app::StartupDiagnosticSnapshot {
                        ok: true,
                        detail: "initialized".to_string(),
                    },
                    account: crate::core::app::StartupDiagnosticSnapshot {
                        ok: true,
                        detail: "authenticated".to_string(),
                    },
                    attachment: crate::core::app::StartupAttachmentSnapshot {
                        mode_label: "provider-launched".to_string(),
                        recovery_anchor_label: "provider-thread-id".to_string(),
                    },
                    warnings: Vec::new(),
                    schema_snapshot: "embedded schema".to_string(),
                })),
            }
        );
    }

    #[test]
    fn startup_error_maps_to_core_completion() {
        let error = anyhow::anyhow!("unsafe executable ancestor")
            .context("failed to pin trusted Codex executable");
        assert_eq!(
            startup_checks_completion(startup_correlation(), Err(error)),
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_correlation(),
                result: Err(
                    "failed to pin trusted Codex executable: unsafe executable ancestor"
                        .to_string()
                ),
            }
        );
    }

    #[test]
    fn session_catalog_success_maps_to_core_completion() {
        let catalog = RecentSessions {
            items: Vec::new(),
            warnings: vec!["partial catalog".to_string()],
            next_cursor: None,
        }
        .into();

        assert_eq!(
            session_catalog_completion(session_catalog_correlation(), Ok(catalog)),
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(),
                result: Ok(SessionCatalogReadySnapshot {
                    catalog: Box::new(
                        RecentSessions {
                            items: Vec::new(),
                            warnings: vec!["partial catalog".to_string()],
                            next_cursor: None,
                        }
                        .into(),
                    ),
                    tier_label: SessionCatalogTier::ProviderBackedCatalog
                        .label()
                        .to_string(),
                    item_count: 0,
                    warnings: vec!["partial catalog".to_string()],
                })
            }
        );
    }

    #[test]
    fn session_catalog_error_maps_to_core_completion() {
        assert_eq!(
            session_catalog_completion(
                session_catalog_correlation(),
                Err(anyhow::anyhow!("catalog unavailable"))
            ),
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(),
                result: Err("catalog unavailable".to_string())
            }
        );
    }

    #[test]
    fn session_rename_result_maps_to_exact_core_completion() {
        assert_eq!(
            session_rename_completion(session_rename_correlation(), Ok(())),
            CoreEffectCompletion::SessionRenamed {
                correlation: session_rename_correlation(),
                result: Ok(()),
            }
        );
        assert_eq!(
            session_rename_completion(
                session_rename_correlation(),
                Err(anyhow::anyhow!("rename unavailable")),
            ),
            CoreEffectCompletion::SessionRenamed {
                correlation: session_rename_correlation(),
                result: Err("rename unavailable".to_string()),
            }
        );
    }

    #[test]
    fn turn_steer_result_maps_to_exact_core_completion() {
        let receipt = crate::domain::conversation::ConversationTurnSteerReceipt {
            turn_id: "turn-1".to_string(),
        };
        assert_eq!(
            turn_steer_completion(turn_steer_correlation(), Ok(receipt.clone())),
            CoreEffectCompletion::TurnSteered {
                correlation: turn_steer_correlation(),
                result: Ok(receipt),
            }
        );
        assert_eq!(
            turn_steer_completion(
                turn_steer_correlation(),
                Err(anyhow::anyhow!("steer unavailable")),
            ),
            CoreEffectCompletion::TurnSteered {
                correlation: turn_steer_correlation(),
                result: Err("steer unavailable".to_string()),
            }
        );
    }

    #[test]
    fn approval_decision_result_maps_to_exact_core_completion() {
        assert_eq!(
            approval_decision_completion(approval_decision_correlation(), Ok(())),
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: approval_decision_correlation(),
                result: Ok(()),
            }
        );
        assert_eq!(
            approval_decision_completion(
                approval_decision_correlation(),
                Err(anyhow::anyhow!("approval unavailable")),
            ),
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: approval_decision_correlation(),
                result: Err("approval unavailable".to_string()),
            }
        );
    }

    #[test]
    fn approval_review_persistence_result_maps_to_exact_core_completion() {
        assert_eq!(
            approval_review_persistence_completion(
                approval_review_persistence_correlation(),
                Ok(())
            ),
            CoreEffectCompletion::ApprovalReviewPersisted {
                correlation: approval_review_persistence_correlation(),
                result: Ok(()),
            }
        );
        assert_eq!(
            approval_review_persistence_completion(
                approval_review_persistence_correlation(),
                Err(anyhow::anyhow!("review authority unavailable")),
            ),
            CoreEffectCompletion::ApprovalReviewPersisted {
                correlation: approval_review_persistence_correlation(),
                result: Err("review authority unavailable".to_string()),
            }
        );
    }

    #[test]
    fn github_review_poll_result_maps_to_exact_core_completion() {
        let result = github_review_poll_result();
        assert_eq!(
            github_review_poll_completion(github_review_poll_correlation(), Ok(result.clone()),),
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: github_review_poll_correlation(),
                result: Ok(Box::new(result)),
            }
        );
        assert_eq!(
            github_review_poll_completion(
                github_review_poll_correlation(),
                Err(anyhow::anyhow!("github unavailable")),
            ),
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: github_review_poll_correlation(),
                result: Err("github unavailable".to_string()),
            }
        );
    }

    #[test]
    fn review_center_snapshot_maps_every_projection_field() {
        let mut thread_review = ReviewCenterThreadProjection::new(
            "thread-1",
            "review-1",
            "Manual review",
            "pending",
            "Need operator follow-up",
            "2026-07-06T10:00:00Z",
            "2026-07-06T11:00:00Z",
        );
        thread_review.handoff_target = Some("operator".to_string());
        thread_review.handoff_note = Some("resume in inbox".to_string());
        let mut inbox_item = ReviewCenterInboxItem::new(
            "review-2",
            "thread-2",
            "pending",
            "Approve filesystem access",
            "2026-07-07T10:00:00Z",
            "2026-07-07T11:00:00Z",
        );
        inbox_item.handoff_target = Some("security".to_string());
        let history_entry = ReviewCenterHistoryEntry::new(
            "review-3",
            "thread-3",
            "approved",
            "Operator approved",
            "2026-07-08T12:00:00Z",
        );

        assert_eq!(
            review_center_snapshot(
                Ok(vec![thread_review]),
                Ok(vec![inbox_item]),
                Ok(vec![history_entry]),
            ),
            ReviewCenterSnapshot {
                current_thread_reviews: Ok(vec![ConversationThreadReviewSnapshot {
                    thread_id: "thread-1".to_string(),
                    review_id: "review-1".to_string(),
                    review_label: "Manual review".to_string(),
                    review_state: "pending".to_string(),
                    review_summary: "Need operator follow-up".to_string(),
                    requested_at: "2026-07-06T10:00:00Z".to_string(),
                    updated_at: "2026-07-06T11:00:00Z".to_string(),
                    handoff_target: Some("operator".to_string()),
                    handoff_note: Some("resume in inbox".to_string()),
                }]),
                pending_inbox: Ok(vec![ReviewCenterInboxItemSnapshot {
                    review_id: "review-2".to_string(),
                    thread_id: "thread-2".to_string(),
                    inbox_state: "pending".to_string(),
                    summary: "Approve filesystem access".to_string(),
                    requested_at: "2026-07-07T10:00:00Z".to_string(),
                    last_activity_at: "2026-07-07T11:00:00Z".to_string(),
                    handoff_target: Some("security".to_string()),
                }]),
                recent_history: Ok(vec![ReviewCenterHistoryEntrySnapshot {
                    review_id: "review-3".to_string(),
                    thread_id: "thread-3".to_string(),
                    event_kind: "approved".to_string(),
                    summary: "Operator approved".to_string(),
                    recorded_at: "2026-07-08T12:00:00Z".to_string(),
                }]),
            }
        );
    }

    #[test]
    fn review_center_snapshot_preserves_partial_failures() {
        let thread_review = ReviewCenterThreadProjection::new(
            "thread-1",
            "review-1",
            "Manual review",
            "pending",
            "Needs review",
            "2026-07-06T10:00:00Z",
            "2026-07-06T11:00:00Z",
        );
        let history_entry = ReviewCenterHistoryEntry::new(
            "review-1",
            "thread-1",
            "requested",
            "Review requested",
            "2026-07-06T10:00:00Z",
        );

        let snapshot = review_center_snapshot(
            Ok(vec![thread_review]),
            Err(anyhow::anyhow!("inbox unavailable")),
            Ok(vec![history_entry]),
        );

        assert_eq!(snapshot.pending_inbox, Err("inbox unavailable".to_string()));
        assert_eq!(snapshot.current_thread_reviews.unwrap().len(), 1);
        assert_eq!(snapshot.recent_history.unwrap().len(), 1);
    }

    #[test]
    fn directions_maintenance_result_maps_application_projection_to_core_snapshot() {
        let result = directions_maintenance_result(Ok(ApplicationDirectionsMaintenanceSummary {
            directions: vec![
                ApplicationDirectionsMaintenanceDirectionSummary {
                    id: "missing".to_string(),
                    title: "Missing mapping".to_string(),
                    detail_doc_path: None,
                    detail_doc_status: ApplicationDirectionsSupportingFileStatus::MissingMapping,
                },
                ApplicationDirectionsMaintenanceDirectionSummary {
                    id: "ready".to_string(),
                    title: "Ready mapping".to_string(),
                    detail_doc_path: Some("docs/directions/ready.md".to_string()),
                    detail_doc_status: ApplicationDirectionsSupportingFileStatus::Ready,
                },
                ApplicationDirectionsMaintenanceDirectionSummary {
                    id: "broken".to_string(),
                    title: "Broken mapping".to_string(),
                    detail_doc_path: Some("docs/directions/missing.md".to_string()),
                    detail_doc_status: ApplicationDirectionsSupportingFileStatus::BrokenMapping,
                },
            ],
            missing_detail_doc_count: 1,
            broken_detail_doc_count: 1,
            queue_idle_policy: QueueIdlePolicy::ReviewAndEnqueue,
            queue_idle_prompt_path: Some("prompts/queue-idle.md".to_string()),
            queue_idle_prompt_status: ApplicationDirectionsSupportingFileStatus::BrokenMapping,
            parse_error: Some("legacy parse warning".to_string()),
        }));

        assert_eq!(
            result,
            Ok(Box::new(DirectionsMaintenanceSummarySnapshot {
                directions: vec![
                    DirectionsMaintenanceDirectionSnapshot {
                        id: "missing".to_string(),
                        title: "Missing mapping".to_string(),
                        detail_doc_path: None,
                        detail_doc_status: CoreDirectionsSupportingFileStatus::MissingMapping,
                    },
                    DirectionsMaintenanceDirectionSnapshot {
                        id: "ready".to_string(),
                        title: "Ready mapping".to_string(),
                        detail_doc_path: Some("docs/directions/ready.md".to_string()),
                        detail_doc_status: CoreDirectionsSupportingFileStatus::Ready,
                    },
                    DirectionsMaintenanceDirectionSnapshot {
                        id: "broken".to_string(),
                        title: "Broken mapping".to_string(),
                        detail_doc_path: Some("docs/directions/missing.md".to_string()),
                        detail_doc_status: CoreDirectionsSupportingFileStatus::BrokenMapping,
                    },
                ],
                missing_detail_doc_count: 1,
                broken_detail_doc_count: 1,
                queue_idle_policy: QueueIdlePolicy::ReviewAndEnqueue,
                queue_idle_prompt_path: Some("prompts/queue-idle.md".to_string()),
                queue_idle_prompt_status: CoreDirectionsSupportingFileStatus::BrokenMapping,
                parse_error: Some("legacy parse warning".to_string()),
            }))
        );
    }

    #[test]
    fn directions_maintenance_result_maps_load_error_to_string() {
        assert_eq!(
            directions_maintenance_result(Err(anyhow::anyhow!("directions unavailable"))),
            Err("directions unavailable".to_string())
        );
    }

    #[test]
    fn queue_authority_result_maps_core_owned_projection_and_tasks() {
        let runtime_projection =
            RuntimeProjection::ready("prompt".to_string(), "queue".to_string(), None)
                .with_planning_revision(Some(42));
        let task = queue_task();

        assert_eq!(
            queue_authority_result(Ok(PlanningQueueAuthorityProjection {
                runtime_projection: runtime_projection.clone(),
                queue_authority: PlanningQueueAuthoritySnapshot {
                    planning_revision: 42,
                    tasks: vec![task.clone()],
                },
            })),
            Ok(QueueAuthoritySnapshot {
                runtime_projection,
                planning_revision: 42,
                tasks: vec![task],
            })
        );
    }

    #[test]
    fn queue_authority_result_preserves_each_refresh_error() {
        for (application_error, core_error) in [
            (
                PlanningQueueAuthorityRefreshError::AuthorityUnavailable(
                    "store unavailable".to_string(),
                ),
                QueueAuthorityLoadError::AuthorityUnavailable("store unavailable".to_string()),
            ),
            (
                PlanningQueueAuthorityRefreshError::RevisionsKeptChanging {
                    projection_revision: 41,
                    authority_revision: 42,
                },
                QueueAuthorityLoadError::RevisionsKeptChanging {
                    projection_revision: 41,
                    authority_revision: 42,
                },
            ),
            (
                PlanningQueueAuthorityRefreshError::RuntimeProjectionUnavailable,
                QueueAuthorityLoadError::RuntimeProjectionUnavailable,
            ),
        ] {
            assert_eq!(
                queue_authority_result(Err(application_error)),
                Err(core_error)
            );
        }
    }

    #[test]
    fn queue_mutation_intent_maps_to_application_cancellation_request() {
        assert_eq!(
            planning_queue_cancellation_request(&queue_mutation_intent()),
            PlanningQueueCancellationRequest {
                workspace_directory: "/tmp/workspace".to_string(),
                expected_planning_revision: 41,
                targets: vec![PlanningQueueCancellationTarget {
                    task_id: "task-1".to_string(),
                    expected_status: TaskStatus::Ready,
                    expected_updated_at: "2026-07-19T00:00:00Z".to_string(),
                }],
            }
        );
    }

    #[test]
    fn queue_mutation_transaction_maps_commit_and_coherent_authority() {
        let runtime_projection =
            RuntimeProjection::ready("prompt".to_string(), "queue".to_string(), None)
                .with_planning_revision(Some(42));
        let task = queue_task();
        let correlation = QueueMutationCorrelation::new(7, queue_mutation_intent());

        assert_eq!(
            queue_mutation_completion(
                correlation.clone(),
                PlanningQueueCancellationTransactionResult {
                    mutation: Ok(PlanningTaskMutationCommitResult {
                        committed_planning_revision: 42,
                        queue_head: None,
                        task_authority_changed: true,
                        applied_command_count: 1,
                        committed_task_ids: vec!["task-1".to_string()],
                    }),
                    authority: Ok(PlanningQueueAuthorityProjection {
                        runtime_projection: runtime_projection.clone(),
                        queue_authority: PlanningQueueAuthoritySnapshot {
                            planning_revision: 42,
                            tasks: vec![task.clone()],
                        },
                    }),
                },
            ),
            CoreEffectCompletion::QueueMutationCompleted {
                correlation,
                result: Box::new(QueueMutationResult {
                    mutation: Ok(QueueMutationCommitSnapshot {
                        committed_planning_revision: 42,
                        committed_task_ids: vec!["task-1".to_string()],
                    }),
                    authority: Ok(QueueAuthoritySnapshot {
                        runtime_projection,
                        planning_revision: 42,
                        tasks: vec![task],
                    }),
                }),
            }
        );
    }

    #[test]
    fn queue_mutation_transaction_preserves_mutation_and_authority_failures() {
        assert_eq!(
            queue_mutation_result(PlanningQueueCancellationTransactionResult {
                mutation: Err(anyhow::anyhow!("revision conflict")),
                authority: Err(PlanningQueueAuthorityRefreshError::RevisionsKeptChanging {
                    projection_revision: 41,
                    authority_revision: 42,
                }),
            }),
            QueueMutationResult {
                mutation: Err("revision conflict".to_string()),
                authority: Err(QueueAuthorityLoadError::RevisionsKeptChanging {
                    projection_revision: 41,
                    authority_revision: 42,
                }),
            }
        );
    }

    #[test]
    fn stop_request_attempt_preserves_correlation_phase_and_error() {
        let correlation = StopRequestCorrelation::new(7, Some(TurnSubmissionCorrelation::new(3)));

        assert_eq!(
            stop_request_attempt_completion(
                correlation,
                StopRequestAttempt::AfterTurnStarted,
                Err(anyhow::anyhow!("runtime unavailable")),
            ),
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt: StopRequestAttempt::AfterTurnStarted,
                result: Err("runtime unavailable".to_string()),
            }
        );
    }

    #[test]
    fn conversation_snapshot_success_maps_to_core_completion() {
        let conversation = crate::domain::conversation::ConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "Core runtime".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: vec![ConversationMessage::new(
                ConversationMessageKind::User,
                "hello",
                None,
                None,
            )],
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        };
        let mut thread_review = ReviewCenterThreadProjection::new(
            "thread-1",
            "review-1",
            "Manual review",
            "pending",
            "Need operator follow-up",
            "2026-07-06T10:00:00Z",
            "2026-07-06T11:00:00Z",
        );
        thread_review.handoff_target = Some("operator".to_string());
        thread_review.handoff_note = Some("resume in inbox".to_string());

        assert_eq!(
            conversation_snapshot_completion(
                conversation_correlation("thread-1"),
                Ok(LoadedConversationThreadSnapshot {
                    conversation: conversation.clone(),
                    thread_review: vec![thread_review],
                }),
            ),
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_correlation("thread-1"),
                result: Ok(Box::new(ConversationReadySnapshot::from_parts(
                    conversation,
                    vec![ConversationThreadReviewSnapshot {
                        thread_id: "thread-1".to_string(),
                        review_id: "review-1".to_string(),
                        review_label: "Manual review".to_string(),
                        review_state: "pending".to_string(),
                        review_summary: "Need operator follow-up".to_string(),
                        requested_at: "2026-07-06T10:00:00Z".to_string(),
                        updated_at: "2026-07-06T11:00:00Z".to_string(),
                        handoff_target: Some("operator".to_string()),
                        handoff_note: Some("resume in inbox".to_string()),
                    }],
                ))),
            }
        );
    }

    #[test]
    fn conversation_snapshot_error_maps_to_core_completion() {
        assert_eq!(
            conversation_snapshot_completion(
                conversation_correlation("thread-1"),
                Err(anyhow::anyhow!("thread unavailable")),
            ),
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_correlation("thread-1"),
                result: Err("thread unavailable".to_string()),
            }
        );
    }

    #[test]
    fn conversation_completion_rejects_provider_thread_mismatch() {
        let snapshot = LoadedConversationThreadSnapshot {
            conversation: crate::domain::conversation::ConversationSnapshot {
                thread_id: "thread-other".to_string(),
                title: "Wrong thread".to_string(),
                cwd: "/tmp/workspace".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            },
            thread_review: Vec::new(),
        };

        let CoreEffectCompletion::ConversationLoaded { result, .. } =
            conversation_snapshot_completion(conversation_correlation("thread-1"), Ok(snapshot))
        else {
            panic!("general conversation load should use its correlated completion variant");
        };
        assert_eq!(
            result,
            Err("conversation provider returned a different thread".to_string())
        );
    }

    #[test]
    fn parallel_peek_completion_keeps_correlation_and_validates_thread() {
        let conversation = crate::domain::conversation::ConversationSnapshot {
            thread_id: "thread-peek".to_string(),
            title: "Peek thread".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        };

        assert_eq!(
            parallel_peek_conversation_completion(
                ParallelPeekLoadCorrelation::new(7, "thread-peek"),
                Ok(conversation.clone()),
            ),
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation: ParallelPeekLoadCorrelation::new(7, "thread-peek"),
                result: Ok(Box::new(ConversationReadySnapshot::from(conversation))),
            }
        );

        let mismatched = crate::domain::conversation::ConversationSnapshot {
            thread_id: "wrong-thread".to_string(),
            title: "Wrong thread".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        };
        let CoreEffectCompletion::ParallelPeekConversationLoaded { result, .. } =
            parallel_peek_conversation_completion(
                ParallelPeekLoadCorrelation::new(8, "thread-peek"),
                Ok(mismatched),
            )
        else {
            panic!("parallel peek completion must keep its dedicated variant");
        };
        assert_eq!(
            result,
            Err("conversation provider returned a different thread".to_string())
        );
    }
}
