use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};
use crate::application::service::conversation_service::{
    ConversationService, LoadedConversationThreadSnapshot,
};
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::manual_prompt_preparation::ManualPromptPreparationService;
use crate::application::service::parallel_mode::parallel_mode_integration_branch_for_repo;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::{
    DirectionsMaintenanceSummary as ApplicationDirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus as ApplicationDirectionsSupportingFileStatus,
    PlanningDraftEditorFile as ApplicationPlanningDraftEditorFile,
    PlanningDraftEditorSession as ApplicationPlanningDraftEditorSession,
    PlanningDraftPromoteResult as ApplicationPlanningDraftPromoteResult,
    PlanningInitStageResult as ApplicationPlanningInitStageResult,
    PlanningQueueAuthorityProjection, PlanningQueueAuthorityRefreshError,
    PlanningQueueCancellationRequest, PlanningQueueCancellationTarget,
    PlanningQueueCancellationTransactionResult, PlanningQueueUseCases,
    PlanningResetTarget as ApplicationPlanningResetTarget, PlanningRuntimeProjection,
    PlanningRuntimeUseCases, PlanningServices, PlanningTaskMutationCommitResult,
    PlanningWorkspaceUseCases,
};
use crate::application::service::post_turn_evaluation::{
    POST_TURN_EVALUATION_TIMEOUT, PostTurnEvaluationService, post_turn_evaluation_failure_execution,
};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::core_effect_worker::{
    spawn_effect_completion_worker, spawn_effect_completion_worker_with_recovery,
};
use crate::composition::core_turn_submission;
use crate::composition::production;
use crate::core::app::github_review_polling_target_is_valid;
use crate::core::app::{
    ApprovalDecisionCorrelation, ApprovalReviewPersistenceCorrelation, ConversationLoadCorrelation,
    ConversationReadySnapshot, ConversationThreadReviewSnapshot,
    DirectionsMaintenanceDirectionSnapshot, DirectionsMaintenanceLoadCorrelation,
    DirectionsMaintenanceSummarySnapshot,
    DirectionsSupportingFileStatus as CoreDirectionsSupportingFileStatus,
    GithubReviewPollCorrelation, GithubReviewPollingSetupCorrelation, GithubReviewPollingSetupMode,
    GithubReviewPollingSetupRequest, GithubReviewPollingSetupResult, ParallelPeekLoadCorrelation,
    PlanningEditorFileSnapshot, PlanningEditorMutationAction, PlanningEditorMutationRequest,
    PlanningEditorMutationResult, PlanningEditorSessionSnapshot, PlanningEditorStageSnapshot,
    PlanningEditorStageTarget, PlanningRuntimeRefreshCorrelation, PlanningRuntimeRefreshSnapshot,
    PlanningSimpleDraftPromotionSnapshot, PlanningSimpleDraftStageSnapshot,
    PlanningWorkspaceOperationCorrelation, PlanningWorkspaceOperationKind,
    PlanningWorkspaceResetSnapshot, PlanningWorkspaceResetTarget, QueueAuthorityLoadCorrelation,
    QueueAuthorityLoadError, QueueAuthoritySnapshot, QueueMutationCommitSnapshot,
    QueueMutationCorrelation, QueueMutationIntent, QueueMutationResult,
    ReviewCenterHistoryEntrySnapshot, ReviewCenterInboxItemSnapshot, ReviewCenterLoadCorrelation,
    ReviewCenterSnapshot, SessionCatalogLoadCorrelation, SessionCatalogReadySnapshot,
    SessionRenameCorrelation, StartupCheckCorrelation, StopRequestAttempt, StopRequestCorrelation,
};
use crate::core::app::{CoreEffect, CoreEffectCompletion, CoreInput, StartupReadySnapshot};
use crate::core::runtime::CoreEffectExecutor;
use crate::core::runtime::CoreInputSender;
use crate::domain::github_review::GithubPullRequestTarget;
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::panic_observation::catch_redacted_worker_unwind;

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
    github_review_polling_setup_loader: Arc<GithubReviewPollingSetupLoader>,
    github_review_polling_services: Arc<GithubReviewPollingServiceRegistry>,
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

type GithubReviewPollingSetupLoader = dyn Fn(
        &GithubReviewPollingSetupRequest,
    ) -> Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>>
    + Send
    + Sync;

struct GithubReviewPollingServiceEntry {
    correlation: GithubReviewPollingSetupCorrelation,
    service: Option<GithubReviewPollerService>,
}

#[derive(Default)]
struct GithubReviewPollingServiceRegistry {
    entry: Mutex<Option<GithubReviewPollingServiceEntry>>,
}

impl GithubReviewPollingServiceRegistry {
    fn begin(&self, correlation: GithubReviewPollingSetupCorrelation) {
        *self
            .entry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(GithubReviewPollingServiceEntry {
                correlation,
                service: None,
            });
    }

    fn complete(
        &self,
        correlation: &GithubReviewPollingSetupCorrelation,
        service: Option<GithubReviewPollerService>,
    ) {
        let mut entry = self
            .entry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(current) = entry.as_mut() else {
            return;
        };
        if current.correlation == *correlation {
            current.service = service;
        }
    }

    fn service_for(
        &self,
        correlation: &GithubReviewPollingSetupCorrelation,
    ) -> Option<GithubReviewPollerService> {
        let entry = self
            .entry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entry
            .as_ref()
            .filter(|entry| entry.correlation == *correlation)
            .and_then(|entry| entry.service.clone())
    }
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
            github_review_polling_setup_loader: Arc::new(load_github_review_polling_setup),
            github_review_polling_services: Arc::new(GithubReviewPollingServiceRegistry::default()),
            manual_prompt_workers: Arc::new(EffectExecutionRegistry::default()),
            stop_request_workers: Arc::new(EffectExecutionRegistry::default()),
            input_sender,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_github_review_polling_setup_loader(
        mut self,
        loader: impl Fn(
            &GithubReviewPollingSetupRequest,
        ) -> Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.github_review_polling_setup_loader = Arc::new(loader);
        self
    }

    fn spawn_startup_checks(&self, correlation: StartupCheckCorrelation) {
        let startup_service = self.startup_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = startup_checks_completion(
            correlation.clone(),
            Err(anyhow::anyhow!("startup checks worker panicked")),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            guarded_startup_checks_completion(correlation, |workspace_directory| {
                startup_service.run_checks(workspace_directory)
            })
        });
    }

    pub fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
        match effect {
            CoreEffect::RunStartupChecks { correlation } => {
                self.spawn_startup_checks(correlation);
                None
            }
            CoreEffect::LoadSessionCatalog { correlation } => {
                self.spawn_session_catalog_load(correlation);
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
            CoreEffect::ResetPlanningWorkspace { correlation } => {
                self.spawn_planning_workspace_reset(correlation);
                None
            }
            CoreEffect::StageSimplePlanningDraft { correlation } => {
                self.spawn_simple_planning_draft_stage(correlation);
                None
            }
            CoreEffect::StagePlanningEditor { correlation } => {
                self.spawn_planning_editor_stage(correlation);
                None
            }
            CoreEffect::MutatePlanningEditor {
                correlation,
                request,
            } => {
                self.spawn_planning_editor_mutation(correlation, request);
                None
            }
            CoreEffect::LoadSimplePlanningEditor { correlation } => {
                self.spawn_simple_planning_editor_load(correlation);
                None
            }
            CoreEffect::PromoteSimplePlanningDraft { correlation } => {
                self.spawn_simple_planning_draft_promotion(correlation);
                None
            }
            CoreEffect::ExecuteQueueMutation { correlation } => {
                self.spawn_queue_mutation(correlation);
                None
            }
            CoreEffect::SetupGithubReviewPolling {
                correlation,
                request,
            } => {
                self.github_review_polling_services
                    .begin(correlation.clone());
                self.spawn_github_review_polling_setup(correlation, request);
                None
            }
            CoreEffect::PollGithubReview {
                setup_correlation,
                correlation,
                previous_state,
            } => {
                let Some(service) = self
                    .github_review_polling_services
                    .service_for(&setup_correlation)
                else {
                    return Some(CoreInput::EffectCompleted(github_review_poll_completion(
                        correlation,
                        Err(anyhow::anyhow!(
                            "github review poller is not configured for the active setup"
                        )),
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
                self.spawn_turn_submission(correlation, *request);
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
            CoreEffect::EvaluatePostTurn {
                correlation,
                request,
            } => {
                self.spawn_post_turn_evaluation(correlation, *request);
                None
            }
        }
    }

    fn spawn_session_catalog_load(&self, correlation: SessionCatalogLoadCorrelation) {
        let session_service = self.session_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = session_catalog_completion(
            correlation.clone(),
            Err(anyhow::anyhow!("session catalog worker panicked")),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let request = SessionCatalogRequest::for_workspace(
                correlation.limit,
                correlation.workspace_directory.clone(),
            );
            session_catalog_completion(correlation, session_service.load_session_catalog(request))
        });
    }

    fn spawn_session_rename(&self, correlation: SessionRenameCorrelation) {
        let session_service = self.session_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = session_rename_completion(
            correlation.clone(),
            Err(anyhow::anyhow!("session rename worker panicked")),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = session_service.rename_session(correlation.request.clone());
            session_rename_completion(correlation, result)
        });
    }

    fn spawn_conversation_load(
        &self,
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
    ) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = conversation_snapshot_completion(
            correlation.clone(),
            Err(anyhow::anyhow!("conversation load worker panicked")),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = conversation_service.load_thread_snapshot(
                correlation.requested_thread_id.as_str(),
                fallback_workspace_directory.as_str(),
            );
            conversation_snapshot_completion(correlation, result)
        });
    }

    fn spawn_parallel_peek_conversation_load(&self, correlation: ParallelPeekLoadCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = parallel_peek_conversation_completion(
            correlation.clone(),
            Err(anyhow::anyhow!(
                "parallel peek conversation worker panicked"
            )),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result =
                conversation_service.load_snapshot(correlation.requested_thread_id.as_str());
            parallel_peek_conversation_completion(correlation, result)
        });
    }

    fn spawn_review_center_load(&self, correlation: ReviewCenterLoadCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::ReviewCenterLoaded {
            correlation: correlation.clone(),
            snapshot: failed_review_center_snapshot("review center worker panicked"),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let snapshot = load_review_center_snapshot(&conversation_service, &correlation);
            CoreEffectCompletion::ReviewCenterLoaded {
                correlation,
                snapshot,
            }
        });
    }

    fn spawn_queue_authority_load(&self, correlation: QueueAuthorityLoadCorrelation) {
        let planning_queue = self.planning_queue.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::QueueAuthorityLoaded {
            correlation: correlation.clone(),
            result: Err(QueueAuthorityLoadError::AuthorityUnavailable(
                "queue authority worker panicked".to_string(),
            )),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = queue_authority_result(
                planning_queue.load_coherent_authority(&correlation.workspace_directory),
            );
            CoreEffectCompletion::QueueAuthorityLoaded {
                correlation,
                result: result.map(Box::new),
            }
        });
    }

    fn spawn_directions_maintenance_load(&self, correlation: DirectionsMaintenanceLoadCorrelation) {
        let planning_workspace = self.planning_workspace.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::DirectionsMaintenanceLoaded {
            correlation: correlation.clone(),
            result: Err("directions maintenance worker panicked".to_string()),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = directions_maintenance_result(
                planning_workspace.load_summary(&correlation.workspace_directory),
            );
            CoreEffectCompletion::DirectionsMaintenanceLoaded {
                correlation,
                result,
            }
        });
    }

    fn spawn_planning_runtime_projection_load(
        &self,
        correlation: PlanningRuntimeRefreshCorrelation,
    ) {
        let planning_runtime = self.planning_runtime.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::PlanningRuntimeLoaded {
            correlation: correlation.clone(),
            result: Err("planning runtime worker panicked".to_string()),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = planning_runtime
                .inspect_runtime_projection(&correlation.workspace_directory)
                .map(PlanningRuntimeRefreshSnapshot::new)
                .map(Box::new)
                .map_err(|error| error.to_string());
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation,
                result,
            }
        });
    }

    fn spawn_planning_workspace_reset(&self, correlation: PlanningWorkspaceOperationCorrelation) {
        let planning_workspace = self.planning_workspace.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::PlanningWorkspaceResetCompleted {
            correlation: correlation.clone(),
            result: Err("planning workspace reset worker panicked".to_string()),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = correlation
                .reset_target()
                .ok_or_else(|| anyhow::anyhow!("planning workspace reset operation mismatch"))
                .and_then(|requested_target| {
                    catch_redacted_worker_unwind(|| {
                        planning_workspace.reset_workspace(
                            &correlation.workspace_directory,
                            application_planning_reset_target(requested_target),
                        )
                    })
                    .map_err(|_| anyhow::anyhow!("planning workspace reset worker panicked"))
                    .and_then(|result| result)
                    .map(planning_workspace_reset_snapshot)
                    .and_then(|snapshot| {
                        if snapshot.target == requested_target {
                            Ok(snapshot)
                        } else {
                            Err(anyhow::anyhow!(
                                "planning workspace reset provider returned a different target"
                            ))
                        }
                    })
                    .map(Box::new)
                })
                .map_err(|error| error.to_string());
            CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation,
                result,
            }
        });
    }

    fn spawn_simple_planning_draft_stage(
        &self,
        correlation: PlanningWorkspaceOperationCorrelation,
    ) {
        let planning_workspace = self.planning_workspace.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::PlanningSimpleDraftStaged {
            correlation: correlation.clone(),
            result: Err("planning simple draft stage worker panicked".to_string()),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = if matches!(
                &correlation.operation,
                PlanningWorkspaceOperationKind::StageSimpleDraft
            ) {
                catch_redacted_worker_unwind(|| {
                    planning_workspace.stage_simple_mode_draft(&correlation.workspace_directory)
                })
                .map_err(|_| anyhow::anyhow!("planning simple draft stage worker panicked"))
                .and_then(|result| result)
                .map(|result| simple_draft_stage_snapshot(&correlation, result))
                .map(Box::new)
            } else {
                Err(anyhow::anyhow!(
                    "planning simple draft stage operation mismatch"
                ))
            }
            .map_err(|error| error.to_string());
            CoreEffectCompletion::PlanningSimpleDraftStaged {
                correlation,
                result,
            }
        });
    }

    fn spawn_simple_planning_editor_load(
        &self,
        correlation: PlanningWorkspaceOperationCorrelation,
    ) {
        let planning_workspace = self.planning_workspace.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::PlanningSimpleEditorLoaded {
            correlation: correlation.clone(),
            result: Err("planning simple editor load worker panicked".to_string()),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            simple_planning_editor_load_completion(&planning_workspace, correlation)
        });
    }

    fn spawn_planning_editor_stage(&self, correlation: PlanningWorkspaceOperationCorrelation) {
        let planning_workspace = self.planning_workspace.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::PlanningEditorStaged {
            correlation: correlation.clone(),
            result: Err("planning editor stage worker panicked".to_string()),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            planning_editor_stage_completion(&planning_workspace, correlation)
        });
    }

    fn spawn_planning_editor_mutation(
        &self,
        correlation: PlanningWorkspaceOperationCorrelation,
        request: Box<PlanningEditorMutationRequest>,
    ) {
        let planning_workspace = self.planning_workspace.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::PlanningEditorMutationCompleted {
            correlation: correlation.clone(),
            result: Err("planning editor mutation worker panicked".to_string()),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            planning_editor_mutation_completion(&planning_workspace, correlation, *request)
        });
    }

    fn spawn_simple_planning_draft_promotion(
        &self,
        correlation: PlanningWorkspaceOperationCorrelation,
    ) {
        let planning_workspace = self.planning_workspace.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = CoreEffectCompletion::PlanningSimpleDraftPromoted {
            correlation: correlation.clone(),
            result: Err("planning simple draft promotion worker panicked".to_string()),
        };
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            simple_planning_draft_promotion_completion(&planning_workspace, correlation)
        });
    }

    fn spawn_queue_mutation(&self, correlation: QueueMutationCorrelation) {
        let planning_queue = self.planning_queue.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = queue_mutation_panic_completion(correlation.clone());
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let request = planning_queue_cancellation_request(&correlation.intent);
            queue_mutation_completion(
                correlation,
                planning_queue.execute_cancellation_transaction(request),
            )
        });
    }

    fn spawn_github_review_poll(
        &self,
        service: GithubReviewPollerService,
        correlation: GithubReviewPollCorrelation,
        previous_state: Option<crate::domain::github_review::GithubPullRequestPollState>,
    ) {
        let input_sender = self.input_sender.clone();
        let panic_completion = github_review_poll_completion(
            correlation.clone(),
            Err(anyhow::anyhow!("GitHub review polling worker panicked")),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = service.poll(&correlation.target, previous_state.as_ref());
            github_review_poll_completion(correlation, result)
        });
    }

    fn spawn_github_review_polling_setup(
        &self,
        correlation: GithubReviewPollingSetupCorrelation,
        request: GithubReviewPollingSetupRequest,
    ) {
        let loader = self.github_review_polling_setup_loader.clone();
        let services = self.github_review_polling_services.clone();
        let input_sender = self.input_sender.clone();
        let panic_correlation = correlation.clone();
        let panic_services = services.clone();
        let panic_completion = CoreEffectCompletion::GithubReviewPollingSetupCompleted {
            correlation: panic_correlation.clone(),
            result: Err("GitHub review polling setup worker panicked".to_string()),
        };
        spawn_effect_completion_worker_with_recovery(
            input_sender,
            panic_completion,
            move || {
                panic_services.complete(&panic_correlation, None);
            },
            move || {
                let loaded = loader(&request);
                let result = match loaded {
                    Ok(Some((target, _service)))
                        if !github_review_polling_target_is_valid(&target) =>
                    {
                        services.complete(&correlation, None);
                        Err("GitHub review polling setup returned an invalid target".to_string())
                    }
                    Ok(Some((target, _service)))
                        if request
                            .mode
                            .explicit_target()
                            .is_some_and(|expected| expected != &target) =>
                    {
                        services.complete(&correlation, None);
                        Err("GitHub review polling setup returned a different target".to_string())
                    }
                    Ok(Some((target, service))) => {
                        services.complete(&correlation, Some(service));
                        Ok(GithubReviewPollingSetupResult::Active { target })
                    }
                    Ok(None) => {
                        services.complete(&correlation, None);
                        Ok(GithubReviewPollingSetupResult::Disabled)
                    }
                    Err(error) => {
                        services.complete(&correlation, None);
                        Err(error.to_string())
                    }
                };
                CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                    correlation,
                    result,
                }
            },
        );
    }

    fn spawn_manual_prompt_preparation(
        &self,
        request: crate::domain::planning::ManualPromptRequest,
        permit: EffectExecutionPermit,
    ) {
        let service = self.manual_prompt_preparation_service.clone();
        let input_sender = self.input_sender.clone();
        let workers = self.manual_prompt_workers.clone();
        let generation = request.correlation.generation;
        let panic_correlation = request.correlation.clone();
        let panic_transcript = request.raw_prompt.trim().to_string();
        let panic_workers = workers.clone();
        let panic_permit = permit.clone();
        let panic_completion = CoreEffectCompletion::ManualPromptPrepared(Box::new(
            crate::domain::planning::ManualPromptOutcome::Rejected {
                correlation: panic_correlation,
                transcript_text: panic_transcript,
                runtime_projection: Box::new(PlanningRuntimeProjection::invalid(
                    "manual prompt preparation worker panicked",
                )),
                reason: "manual prompt preparation worker panicked".to_string(),
            },
        ));
        spawn_effect_completion_worker_with_recovery(
            input_sender,
            panic_completion,
            move || {
                panic_workers.finish(generation, &panic_permit);
            },
            move || {
                let panic_correlation = request.correlation.clone();
                let panic_transcript = request.raw_prompt.trim().to_string();
                let result = catch_redacted_worker_unwind(|| {
                    service.prepare_guarded(request, &|| permit.is_active())
                })
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
                CoreEffectCompletion::ManualPromptPrepared(Box::new(result))
            },
        );
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
        let panic_correlation = correlation;
        let panic_attempt = attempt;
        let panic_workers = workers.clone();
        let panic_permit = permit.clone();
        let panic_completion = stop_request_attempt_completion(
            panic_correlation,
            panic_attempt,
            Err(anyhow::anyhow!("stop request worker panicked")),
        );
        spawn_effect_completion_worker_with_recovery(
            input_sender,
            panic_completion,
            move || {
                panic_workers.finish(panic_correlation.generation, &panic_permit);
            },
            move || {
                let result = catch_redacted_worker_unwind(|| {
                    if !permit.is_active() {
                        return Err(anyhow::anyhow!(
                            "stop request was superseded before provider execution"
                        ));
                    }
                    conversation_service.request_stop_all_sessions()
                })
                .map_err(|_| anyhow::anyhow!("stop request worker panicked"))
                .and_then(|result| result);
                workers.finish(correlation.generation, &permit);
                stop_request_attempt_completion(correlation, attempt, result)
            },
        );
    }

    fn spawn_approval_decision_submission(&self, correlation: ApprovalDecisionCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = approval_decision_completion(
            correlation.clone(),
            Err(anyhow::anyhow!("approval decision worker panicked")),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = conversation_service.resolve_approval_request(
                &correlation.request_identity.approval_id,
                correlation.decision,
            );
            approval_decision_completion(correlation, result)
        });
    }

    fn spawn_approval_review_persistence(&self, correlation: ApprovalReviewPersistenceCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = approval_review_persistence_completion(
            correlation.clone(),
            Err(anyhow::anyhow!(
                "approval review persistence worker panicked"
            )),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            let result = conversation_service.persist_review_center_approval_review_for_workspace(
                &correlation.workspace_directory,
                &correlation.thread_id,
                &correlation.review,
            );
            approval_review_persistence_completion(correlation, result)
        });
    }

    fn spawn_turn_submission(
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

    fn spawn_turn_steer(
        &self,
        correlation: crate::core::app::TurnSteerCorrelation,
        request: crate::domain::conversation::ConversationTurnSteerRequest,
    ) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_completion = turn_steer_completion(
            correlation,
            Err(anyhow::anyhow!("turn steer worker panicked")),
        );
        spawn_effect_completion_worker(input_sender, panic_completion, move || {
            turn_steer_completion(correlation, conversation_service.steer_turn(request))
        });
    }

    fn spawn_post_turn_evaluation(
        &self,
        correlation: crate::core::app::PostTurnEvaluationCorrelation,
        request: crate::domain::planning::PostTurnRequest,
    ) {
        let service = self.post_turn_evaluation_service.clone();
        let input_sender = self.input_sender.clone();
        let panic_request = request.clone();
        let fallback_request = request.clone();
        let panic_execution = post_turn_evaluation_failure_execution(
            &panic_request.context,
            &panic_request,
            "post-turn evaluation worker panicked".to_string(),
        );
        let panic_completion = CoreEffectCompletion::PostTurnEvaluationCompleted {
            correlation: correlation.clone(),
            execution: Box::new(panic_execution),
        };
        spawn_effect_completion_worker_with_recovery(
            input_sender,
            panic_completion,
            move || {
                panic_request.continuation_permit.invalidate_if_current();
            },
            move || {
                let execution =
                    service.evaluate_with_timeout(request, POST_TURN_EVALUATION_TIMEOUT);
                post_turn_evaluation_completion(correlation, &fallback_request, execution)
            },
        );
    }
}

fn post_turn_evaluation_completion(
    correlation: crate::core::app::PostTurnEvaluationCorrelation,
    request: &crate::domain::planning::PostTurnRequest,
    execution: crate::domain::planning::PostTurnExecution,
) -> CoreEffectCompletion {
    let execution = if correlation.matches_execution(&execution) {
        execution
    } else {
        request.continuation_permit.invalidate_if_current();
        post_turn_evaluation_failure_execution(
            &request.context,
            request,
            "post-turn evaluation worker returned a mismatched target".to_string(),
        )
    };
    CoreEffectCompletion::PostTurnEvaluationCompleted {
        correlation,
        execution: Box::new(execution),
    }
}

impl CoreEffectExecutor for CoreEffectRunner {
    fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
        CoreEffectRunner::run_effect(self, effect)
    }
}

fn load_github_review_polling_setup(
    request: &GithubReviewPollingSetupRequest,
) -> Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>> {
    let workspace = Path::new(&request.workspace_directory);
    match &request.mode {
        GithubReviewPollingSetupMode::Explicit { target } => {
            let service = production::build_github_review_poller_service(workspace)?;
            Ok(Some((target.clone(), service)))
        }
        GithubReviewPollingSetupMode::Discover => {
            let integration_branch =
                parallel_mode_integration_branch_for_repo(&request.workspace_directory)
                    .map_err(anyhow::Error::msg)?;
            production::discover_github_review_poller_service_for_current_branch(
                workspace,
                &integration_branch,
            )
        }
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

fn application_planning_reset_target(
    target: PlanningWorkspaceResetTarget,
) -> ApplicationPlanningResetTarget {
    match target {
        PlanningWorkspaceResetTarget::Queue => ApplicationPlanningResetTarget::Queue,
        PlanningWorkspaceResetTarget::Directions => ApplicationPlanningResetTarget::Directions,
        PlanningWorkspaceResetTarget::All => ApplicationPlanningResetTarget::All,
    }
}

fn core_planning_reset_target(
    target: ApplicationPlanningResetTarget,
) -> PlanningWorkspaceResetTarget {
    match target {
        ApplicationPlanningResetTarget::Queue => PlanningWorkspaceResetTarget::Queue,
        ApplicationPlanningResetTarget::Directions => PlanningWorkspaceResetTarget::Directions,
        ApplicationPlanningResetTarget::All => PlanningWorkspaceResetTarget::All,
    }
}

fn planning_workspace_reset_snapshot(
    result: crate::application::service::planning::PlanningWorkspaceResetResult,
) -> PlanningWorkspaceResetSnapshot {
    PlanningWorkspaceResetSnapshot {
        target: core_planning_reset_target(result.target),
        rewritten_paths: result.rewritten_paths,
        removed_paths: result.removed_paths,
    }
}

fn guarded_startup_checks_completion(
    correlation: StartupCheckCorrelation,
    run_checks: impl FnOnce(&str) -> Result<StartupDiagnostics>,
) -> CoreEffectCompletion {
    let workspace_directory = correlation.workspace_directory.clone();
    let result = catch_redacted_worker_unwind(|| run_checks(&workspace_directory))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("startup checks panicked")));
    startup_checks_completion(correlation, result)
}

fn simple_draft_stage_snapshot(
    correlation: &PlanningWorkspaceOperationCorrelation,
    result: ApplicationPlanningInitStageResult,
) -> PlanningSimpleDraftStageSnapshot {
    PlanningSimpleDraftStageSnapshot {
        session_identity: correlation.editor_session_identity(result.draft_name),
        staged_file_count: result.staged_file_count,
        validation_report: result.validation_report,
    }
}

fn planning_editor_session_snapshot(
    correlation: &PlanningWorkspaceOperationCorrelation,
    result: ApplicationPlanningDraftEditorSession,
) -> PlanningEditorSessionSnapshot {
    PlanningEditorSessionSnapshot {
        session_identity: correlation.editor_session_identity(result.draft_name),
        draft_directory: result.draft_directory,
        editable_files: result
            .editable_files
            .into_iter()
            .map(|file| PlanningEditorFileSnapshot {
                active_path: file.active_path,
                staged_path: file.staged_path,
                body: file.body,
            })
            .collect(),
        validation_report: result.validation_report,
        source_planning_revision: result.source_planning_revision,
    }
}

fn planning_editor_stage_completion(
    planning_workspace: &PlanningWorkspaceUseCases,
    correlation: PlanningWorkspaceOperationCorrelation,
) -> CoreEffectCompletion {
    let result = match correlation.editor_stage_target().cloned() {
        Some(target) => {
            let provider_result = catch_redacted_worker_unwind(|| match &target {
                PlanningEditorStageTarget::PlanningManual => {
                    planning_workspace.stage_manual_editor_session(&correlation.workspace_directory)
                }
                PlanningEditorStageTarget::DirectionDetail { direction_id } => planning_workspace
                    .stage_detail_doc_editor_session(
                        &correlation.workspace_directory,
                        direction_id,
                    ),
                PlanningEditorStageTarget::QueueIdlePrompt => planning_workspace
                    .stage_queue_idle_prompt_editor_session(&correlation.workspace_directory),
            })
            .map_err(|_| anyhow::anyhow!("planning editor stage worker panicked"))
            .and_then(|result| result);

            provider_result.and_then(|session| {
                let snapshot = PlanningEditorStageSnapshot {
                    target: target.clone(),
                    session: planning_editor_session_snapshot(&correlation, session),
                };
                validate_planning_editor_stage_snapshot(&correlation, &target, snapshot)
            })
        }
        None => Err(anyhow::anyhow!("planning editor stage operation mismatch")),
    }
    .map(Box::new)
    .map_err(|error| error.to_string());
    CoreEffectCompletion::PlanningEditorStaged {
        correlation,
        result,
    }
}

fn validate_planning_editor_stage_snapshot(
    correlation: &PlanningWorkspaceOperationCorrelation,
    target: &PlanningEditorStageTarget,
    snapshot: PlanningEditorStageSnapshot,
) -> Result<PlanningEditorStageSnapshot> {
    let draft_name = snapshot.session.session_identity.draft_name.as_str();
    let expected_session = correlation.editor_session_identity(draft_name.to_string());
    if draft_name.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "planning editor stage provider returned an empty draft"
        ));
    }
    if &snapshot.target != target || snapshot.session.session_identity != expected_session {
        return Err(anyhow::anyhow!(
            "planning editor stage provider identity mismatch"
        ));
    }
    Ok(snapshot)
}

fn planning_editor_mutation_completion(
    planning_workspace: &PlanningWorkspaceUseCases,
    correlation: PlanningWorkspaceOperationCorrelation,
    request: PlanningEditorMutationRequest,
) -> CoreEffectCompletion {
    let result = validate_planning_editor_mutation_request(&correlation, &request).and_then(|()| {
        let identity = request.identity;
        let editable_files = request
            .editable_files
            .into_iter()
            .map(|file| ApplicationPlanningDraftEditorFile {
                active_path: file.active_path,
                staged_path: file.staged_path,
                body: file.body,
            })
            .collect::<Vec<_>>();
        let workspace_directory = correlation.workspace_directory.as_str();
        let draft_name = identity.draft_name.as_str();
        match identity.action {
            PlanningEditorMutationAction::Save => catch_redacted_worker_unwind(|| {
                planning_workspace.save_draft_editor_files(
                    workspace_directory,
                    draft_name,
                    &editable_files,
                )
            })
            .map_err(|_| anyhow::anyhow!("planning editor mutation worker panicked"))
            .and_then(|result| result)
            .map(|result| PlanningEditorMutationResult::Saved {
                identity,
                draft_name: result.draft_name,
                validation_report: result.validation_report,
            }),
            PlanningEditorMutationAction::Promote => catch_redacted_worker_unwind(|| {
                if let Some(source_planning_revision) = identity.source_planning_revision {
                    planning_workspace.promote_draft_editor_files_at_revision(
                        workspace_directory,
                        draft_name,
                        &editable_files,
                        source_planning_revision,
                    )
                } else {
                    planning_workspace.promote_draft_editor_files(
                        workspace_directory,
                        draft_name,
                        &editable_files,
                    )
                }
            })
            .map_err(|_| anyhow::anyhow!("planning editor mutation worker panicked"))
            .and_then(|result| result)
            .map(|result| PlanningEditorMutationResult::Promoted {
                identity,
                draft_name: result.draft_name,
                promoted_file_count: result.promoted_file_count,
                validation_report: result.validation_report,
                committed_planning_revision: result.committed_planning_revision,
            }),
        }
    });
    CoreEffectCompletion::PlanningEditorMutationCompleted {
        correlation,
        result: result.map(Box::new).map_err(|error| error.to_string()),
    }
}

fn validate_planning_editor_mutation_request(
    correlation: &PlanningWorkspaceOperationCorrelation,
    request: &PlanningEditorMutationRequest,
) -> Result<()> {
    let Some(expected) = correlation.editor_mutation_identity() else {
        anyhow::bail!("planning editor mutation operation mismatch");
    };
    if expected != &request.identity
        || expected.draft_name.trim().is_empty()
        || expected.source_session.workspace_directory != correlation.workspace_directory
        || expected.source_session.draft_name != expected.draft_name
    {
        anyhow::bail!("planning editor mutation request identity mismatch");
    }
    Ok(())
}

fn simple_draft_promotion_snapshot(
    result: ApplicationPlanningDraftPromoteResult,
) -> PlanningSimpleDraftPromotionSnapshot {
    PlanningSimpleDraftPromotionSnapshot {
        draft_name: result.draft_name,
        promoted_file_count: result.promoted_file_count,
        validation_report: result.validation_report,
    }
}

fn simple_planning_editor_load_completion(
    planning_workspace: &PlanningWorkspaceUseCases,
    correlation: PlanningWorkspaceOperationCorrelation,
) -> CoreEffectCompletion {
    let result = match &correlation.operation {
        PlanningWorkspaceOperationKind::LoadSimpleEditor {
            draft_name,
            source_session,
        } if source_session.workspace_directory.as_str()
            == correlation.workspace_directory.as_str()
            && source_session.draft_name.as_str() == draft_name.as_str() =>
        {
            let draft_name = draft_name.clone();
            catch_redacted_worker_unwind(|| {
                planning_workspace
                    .load_manual_editor_session(&correlation.workspace_directory, &draft_name)
            })
            .map_err(|_| anyhow::anyhow!("planning simple editor load worker panicked"))
            .and_then(|result| result)
            .map(|result| planning_editor_session_snapshot(&correlation, result))
        }
        PlanningWorkspaceOperationKind::LoadSimpleEditor { .. } => Err(anyhow::anyhow!(
            "planning simple editor load source session mismatch"
        )),
        _ => Err(anyhow::anyhow!(
            "planning simple editor load operation mismatch"
        )),
    }
    .map(Box::new)
    .map_err(|error| error.to_string());
    CoreEffectCompletion::PlanningSimpleEditorLoaded {
        correlation,
        result,
    }
}

fn simple_planning_draft_promotion_completion(
    planning_workspace: &PlanningWorkspaceUseCases,
    correlation: PlanningWorkspaceOperationCorrelation,
) -> CoreEffectCompletion {
    let result = match &correlation.operation {
        PlanningWorkspaceOperationKind::PromoteSimpleDraft {
            draft_name,
            source_session,
        } if source_session.workspace_directory.as_str()
            == correlation.workspace_directory.as_str()
            && source_session.draft_name.as_str() == draft_name.as_str() =>
        {
            let draft_name = draft_name.clone();
            catch_redacted_worker_unwind(|| {
                planning_workspace
                    .promote_staged_draft(&correlation.workspace_directory, &draft_name)
            })
            .map_err(|_| anyhow::anyhow!("planning simple draft promotion worker panicked"))
            .and_then(|result| result)
            .map(simple_draft_promotion_snapshot)
        }
        PlanningWorkspaceOperationKind::PromoteSimpleDraft { .. } => Err(anyhow::anyhow!(
            "planning simple draft promotion source session mismatch"
        )),
        _ => Err(anyhow::anyhow!(
            "planning simple draft promotion operation mismatch"
        )),
    }
    .map(Box::new)
    .map_err(|error| error.to_string());
    CoreEffectCompletion::PlanningSimpleDraftPromoted {
        correlation,
        result,
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

fn failed_review_center_snapshot(message: &str) -> ReviewCenterSnapshot {
    ReviewCenterSnapshot {
        current_thread_reviews: Err(message.to_string()),
        pending_inbox: Err(message.to_string()),
        recent_history: Err(message.to_string()),
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

fn queue_mutation_panic_completion(correlation: QueueMutationCorrelation) -> CoreEffectCompletion {
    let message = "queue mutation worker panicked".to_string();
    CoreEffectCompletion::QueueMutationCompleted {
        correlation,
        result: Box::new(QueueMutationResult {
            mutation: Err(message.clone()),
            authority: Err(QueueAuthorityLoadError::AuthorityUnavailable(message)),
        }),
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
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::adapter::outbound::github::GithubAutomationAdapter;
    use crate::application::port::outbound::app_server_prompt_log_port::{
        AppServerPromptLogMaintenanceMode, AppServerPromptLogMaintenancePort,
    };
    use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
        PlanningDraftStageRecord, PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
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
        ManualPromptPreparationAdmission, ManualPromptPreparationIntent,
        PlanningEditorMutationIdentity, PlanningEditorMutationTarget,
        PlanningWorkspaceOperationAdmission, PlanningWorkspaceOperationCorrelation,
        PlanningWorkspaceResetIntent, PlanningWorkspaceResetTarget, QueueMutationKind,
        QueueMutationTarget, SessionCatalogLoadIntent, SessionCatalogSnapshot, StartupSnapshot,
        StopRequestAdmission, TurnStreamEvent, TurnSubmissionAdmission, TurnSubmissionCorrelation,
        TurnSubmissionRequest,
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
    const SENSITIVE_MAINTENANCE_PANIC_PAYLOAD: &str = "raw maintenance provider prompt payload";
    const SENSITIVE_SIMPLE_AUTHORING_PANIC_PAYLOAD: &str =
        "raw simple authoring provider draft payload";
    const SENSITIVE_EDITOR_STAGE_PANIC_PAYLOAD: &str =
        "raw planning editor stage provider draft payload";
    const SENSITIVE_EDITOR_MUTATION_PANIC_PAYLOAD: &str =
        "raw planning editor mutation provider body payload";
    const SENSITIVE_STARTUP_PANIC_PAYLOAD: &str = "raw startup provider prompt payload";
    const STARTUP_PANIC_CHILD_ENV: &str = "AKRA_STARTUP_PANIC_OBSERVATION_CHILD";
    const STARTUP_PANIC_TEST_NAME: &str = "composition::core_effect_runner::tests::startup_provider_panic_stderr_is_redacted_in_isolated_process";
    const NORMAL_PANIC_PAYLOAD: &str = "ordinary panic remains observable";
    const STARTUP_COMPLETION_MARKER: &str = "AKRA_STARTUP_COMPLETION_ONCE";

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
        simple_authoring_gate: Option<Arc<OneShotGate>>,
        stage_call_count: Arc<AtomicUsize>,
        promote_call_count: Arc<AtomicUsize>,
        panic_load_once: AtomicBool,
        panic_draft_load_once: AtomicBool,
    }

    impl PlanningWorkspacePort for GatedPlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            self.stage_call_count.fetch_add(1, Ordering::SeqCst);
            if self.panic_draft_load_once.swap(false, Ordering::SeqCst) {
                panic!("{SENSITIVE_EDITOR_STAGE_PANIC_PAYLOAD}");
            }
            if let Some(gate) = &self.simple_authoring_gate {
                gate.wait_once();
            }
            anyhow::bail!("synthetic stage should not complete")
        }

        fn load_planning_draft_files(
            &self,
            workspace_dir: &str,
            draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            self.promote_call_count.fetch_add(1, Ordering::SeqCst);
            if self.panic_draft_load_once.swap(false, Ordering::SeqCst) {
                panic!("{SENSITIVE_SIMPLE_AUTHORING_PANIC_PAYLOAD}");
            }
            if let Some(gate) = &self.simple_authoring_gate {
                gate.wait_once();
            }
            if draft_name.starts_with("mutation-valid-") {
                return Ok(PlanningDraftLoadRecord {
                    draft_name: draft_name.to_string(),
                    draft_directory: format!("{workspace_dir}/drafts/{draft_name}"),
                    staged_files: vec![PlanningDraftLoadFileRecord {
                        active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
                        staged_path: format!(
                            "{workspace_dir}/drafts/{draft_name}/result-output.md"
                        ),
                        body: "# Result Output\n\n- Preserve the accepted task summary.\n"
                            .to_string(),
                    }],
                });
            }
            anyhow::bail!("synthetic promote should not complete")
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            active_path: &str,
            _body: &str,
        ) -> Result<String> {
            self.stage_call_count.fetch_add(1, Ordering::SeqCst);
            if self.panic_draft_load_once.swap(false, Ordering::SeqCst) {
                panic!("{SENSITIVE_EDITOR_MUTATION_PANIC_PAYLOAD}");
            }
            if let Some(gate) = &self.simple_authoring_gate {
                gate.wait_once();
            }
            Ok(format!("draft/{active_path}"))
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
            Ok(None)
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
            self.promote_call_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
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
        panic_session_catalog_once: AtomicBool,
        session_catalog_requests: Mutex<Vec<SessionCatalogRequest>>,
    }

    impl GatedRuntimePort {
        fn with_stop_gate(stop_gate: Arc<OneShotGate>) -> Self {
            Self {
                stop_gate: Some(stop_gate),
                stop_call_count: AtomicUsize::new(0),
                panic_stop_once: AtomicBool::new(false),
                panic_session_catalog_once: AtomicBool::new(false),
                session_catalog_requests: Mutex::new(Vec::new()),
            }
        }

        fn panicking_stop_once() -> Self {
            Self {
                stop_gate: None,
                stop_call_count: AtomicUsize::new(0),
                panic_stop_once: AtomicBool::new(true),
                panic_session_catalog_once: AtomicBool::new(false),
                session_catalog_requests: Mutex::new(Vec::new()),
            }
        }

        fn panicking_session_catalog_once() -> Self {
            Self {
                stop_gate: None,
                stop_call_count: AtomicUsize::new(0),
                panic_stop_once: AtomicBool::new(false),
                panic_session_catalog_once: AtomicBool::new(true),
                session_catalog_requests: Mutex::new(Vec::new()),
            }
        }
    }

    struct PanickingStartupProbePort;

    impl StartupProbePort for PanickingStartupProbePort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            panic!("{SENSITIVE_STARTUP_PANIC_PAYLOAD}");
        }
    }

    struct PanickingPromptLogMaintenancePort;

    impl AppServerPromptLogMaintenancePort for PanickingPromptLogMaintenancePort {
        fn maintain_app_server_prompt_logs(
            &self,
            _workspace_dir: &str,
            _mode: AppServerPromptLogMaintenanceMode,
        ) -> Result<()> {
            panic!("{SENSITIVE_MAINTENANCE_PANIC_PAYLOAD}");
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
        fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
            self.session_catalog_requests
                .lock()
                .expect("session catalog request log should lock")
                .push(request);
            if self
                .panic_session_catalog_once
                .swap(false, Ordering::SeqCst)
            {
                panic!("SENSITIVE-SESSION-CATALOG-PANIC");
            }
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
        test_effect_runner_with_startup_service(
            StartupService::new(runtime_port.clone()),
            planning_workspace,
            runtime_port,
            input_sender,
        )
    }

    fn test_effect_runner_with_startup_service(
        startup_service: StartupService,
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
            startup_service,
            SessionService::new(runtime_port.clone()),
            ConversationService::new(runtime_port),
            planning.clone(),
            parallel_turns.clone(),
            PostTurnEvaluationService::new(planning, parallel_turns),
            input_sender,
        )
    }

    fn test_core_runtime(runtime_port: Arc<GatedRuntimePort>) -> CoreRuntime<CoreEffectRunner> {
        let (planning_gate, _entered, release) = one_shot_gate();
        release
            .send(())
            .expect("unused planning gate should start open");
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: planning_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        CoreRuntime::new(runner, input_receiver)
    }

    struct LabeledGithubReviewPollerPort {
        title: String,
        second_load_gate: Option<Arc<OneShotGate>>,
        load_count: AtomicUsize,
    }

    impl GithubReviewPollerPort for LabeledGithubReviewPollerPort {
        fn load_pull_request_activity(
            &self,
            target: &GithubPullRequestTarget,
        ) -> Result<crate::domain::github_review::GithubPullRequestActivitySnapshot> {
            let load_number = self.load_count.fetch_add(1, Ordering::SeqCst) + 1;
            if load_number == 2
                && let Some(gate) = &self.second_load_gate
            {
                gate.wait_once();
            }
            Ok(
                crate::domain::github_review::GithubPullRequestActivitySnapshot {
                    target: target.clone(),
                    title: self.title.clone(),
                    url: "https://example.invalid/pull/42".to_string(),
                    head_branch: "feature".to_string(),
                    base_branch: "prerelease".to_string(),
                    events: vec![crate::domain::github_review::GithubPullRequestActivityEvent {
                        id: 1,
                        kind: crate::domain::github_review::GithubPullRequestActivityKind::IssueComment,
                        submitted_at: "2026-07-19T10:00:00Z".to_string(),
                        author_login: "reviewer".to_string(),
                        body: "review note".to_string(),
                        state: None,
                        url: "https://example.invalid/pull/42#issuecomment-1".to_string(),
                        path: None,
                    }],
                },
            )
        }
    }

    fn github_review_service(label: &str) -> GithubReviewPollerService {
        GithubReviewPollerService::new(Arc::new(LabeledGithubReviewPollerPort {
            title: label.to_string(),
            second_load_gate: None,
            load_count: AtomicUsize::new(0),
        }))
    }

    fn github_review_service_with_second_load_gate(
        label: &str,
        gate: Arc<OneShotGate>,
    ) -> GithubReviewPollerService {
        GithubReviewPollerService::new(Arc::new(LabeledGithubReviewPollerPort {
            title: label.to_string(),
            second_load_gate: Some(gate),
            load_count: AtomicUsize::new(0),
        }))
    }

    fn github_review_test_runtime(
        loader: impl Fn(
            &GithubReviewPollingSetupRequest,
        ) -> Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>>
        + Send
        + Sync
        + 'static,
    ) -> CoreRuntime<CoreEffectRunner> {
        let (planning_gate, _entered, release) = one_shot_gate();
        release
            .send(())
            .expect("unused planning gate should start open");
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: planning_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender)
            .with_github_review_polling_setup_loader(loader);
        CoreRuntime::new(runner, input_receiver)
    }

    fn explicit_github_review_setup(
        workspace_directory: &str,
        target: GithubPullRequestTarget,
    ) -> AppCommand {
        AppCommand::SetupGithubReviewPolling(GithubReviewPollingSetupRequest::new(
            workspace_directory,
            GithubReviewPollingSetupMode::Explicit { target },
        ))
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
        StartupCheckCorrelation::new(7, "/tmp/workspace")
    }

    fn session_catalog_correlation() -> SessionCatalogLoadCorrelation {
        SessionCatalogLoadCorrelation::new(8, 10, "/tmp/session-catalog")
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
            crate::domain::conversation::ConversationApprovalRequestIdentity {
                approval_id: "approval-1".to_string(),
                server_request_id: "server-1".to_string(),
            },
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

    fn planning_reset_intent(workspace_directory: &str) -> PlanningWorkspaceResetIntent {
        PlanningWorkspaceResetIntent::new(workspace_directory, PlanningWorkspaceResetTarget::Queue)
    }

    fn planning_reset_correlation(
        generation: u64,
        workspace_directory: &str,
    ) -> PlanningWorkspaceOperationCorrelation {
        PlanningWorkspaceOperationCorrelation {
            generation,
            workspace_directory: workspace_directory.to_string(),
            operation: PlanningWorkspaceOperationKind::Reset {
                target: PlanningWorkspaceResetTarget::Queue,
            },
        }
    }

    fn planning_editor_session(
        generation: u64,
        workspace_directory: &str,
        draft_name: &str,
    ) -> crate::core::app::PlanningEditorSessionIdentity {
        crate::core::app::PlanningEditorSessionIdentity::new(
            generation,
            workspace_directory,
            draft_name,
        )
    }

    fn planning_editor_stage_correlation(
        generation: u64,
        workspace_directory: &str,
        target: PlanningEditorStageTarget,
    ) -> PlanningWorkspaceOperationCorrelation {
        PlanningWorkspaceOperationCorrelation {
            generation,
            workspace_directory: workspace_directory.to_string(),
            operation: PlanningWorkspaceOperationKind::StageEditor { target },
        }
    }

    fn planning_editor_stage_snapshot(
        correlation: &PlanningWorkspaceOperationCorrelation,
        target: PlanningEditorStageTarget,
        identity: crate::core::app::PlanningEditorSessionIdentity,
    ) -> PlanningEditorStageSnapshot {
        PlanningEditorStageSnapshot {
            target,
            session: PlanningEditorSessionSnapshot {
                session_identity: identity,
                draft_directory: format!("{}/drafts/draft-a", correlation.workspace_directory),
                editable_files: Vec::new(),
                validation_report: Default::default(),
                source_planning_revision: None,
            },
        }
    }

    fn planning_editor_mutation_identity(
        action: PlanningEditorMutationAction,
        target: PlanningEditorMutationTarget,
        draft_name: &str,
        source_generation: u64,
        workspace_directory: &str,
        buffer_revision: u64,
    ) -> PlanningEditorMutationIdentity {
        PlanningEditorMutationIdentity::new(
            action,
            target,
            draft_name,
            planning_editor_session(source_generation, workspace_directory, draft_name),
            buffer_revision,
        )
    }

    fn planning_editor_mutation_correlation(
        generation: u64,
        workspace_directory: &str,
        identity: PlanningEditorMutationIdentity,
    ) -> PlanningWorkspaceOperationCorrelation {
        PlanningWorkspaceOperationCorrelation {
            generation,
            workspace_directory: workspace_directory.to_string(),
            operation: PlanningWorkspaceOperationKind::MutateEditor { identity },
        }
    }

    fn planning_editor_mutation_request(
        identity: PlanningEditorMutationIdentity,
        body: &str,
    ) -> PlanningEditorMutationRequest {
        PlanningEditorMutationRequest {
            identity,
            editable_files: vec![PlanningEditorFileSnapshot {
                active_path: "planning/result-output.md".to_string(),
                staged_path: "draft/planning/result-output.md".to_string(),
                body: body.to_string(),
            }],
        }
    }

    fn gated_editor_mutation_workspace(
        gate: Option<Arc<OneShotGate>>,
        panic_once: bool,
    ) -> (
        Arc<GatedPlanningWorkspacePort>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
    ) {
        let (unused_load_gate, _unused_entered, _unused_release) = one_shot_gate();
        let replace_call_count = Arc::new(AtomicUsize::new(0));
        let load_call_count = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(GatedPlanningWorkspacePort {
                load_gate: unused_load_gate,
                simple_authoring_gate: gate,
                stage_call_count: replace_call_count.clone(),
                promote_call_count: load_call_count.clone(),
                panic_load_once: AtomicBool::new(false),
                panic_draft_load_once: AtomicBool::new(panic_once),
            }),
            replace_call_count,
            load_call_count,
        )
    }

    #[test]
    fn planning_editor_mutation_rejects_every_identity_mismatch_before_provider_io() {
        let workspace_directory = "/workspace";
        let draft_name = "mutation-valid-identity";
        let identity = planning_editor_mutation_identity(
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationTarget::Planning,
            draft_name,
            17,
            workspace_directory,
            4,
        );
        let correlation =
            planning_editor_mutation_correlation(31, workspace_directory, identity.clone());
        let request = planning_editor_mutation_request(identity.clone(), "body");
        let (planning_workspace, replace_call_count, load_call_count) =
            gated_editor_mutation_workspace(None, false);
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, _input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);

        let mut wrong_action = request.clone();
        wrong_action.identity.action = PlanningEditorMutationAction::Promote;
        let mut wrong_target = request.clone();
        wrong_target.identity.target = PlanningEditorMutationTarget::Directions;
        let mut wrong_draft = request.clone();
        wrong_draft.identity.draft_name = "mutation-valid-other".to_string();
        let mut wrong_source_generation = request.clone();
        wrong_source_generation.identity.source_session.generation += 1;
        let mut wrong_source_workspace = request.clone();
        wrong_source_workspace
            .identity
            .source_session
            .workspace_directory = "/other".to_string();
        let mut wrong_source_draft = request.clone();
        wrong_source_draft.identity.source_session.draft_name = "mutation-valid-other".to_string();
        let mut wrong_buffer_revision = request.clone();
        wrong_buffer_revision.identity.buffer_revision += 1;
        let mut wrong_workspace_correlation = correlation.clone();
        wrong_workspace_correlation.workspace_directory = "/other".to_string();
        let malformed_source_workspace = planning_editor_mutation_identity(
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationTarget::Planning,
            draft_name,
            17,
            "/other",
            4,
        );
        let malformed_source_draft = PlanningEditorMutationIdentity::new(
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationTarget::Planning,
            draft_name,
            planning_editor_session(17, workspace_directory, "mutation-valid-other"),
            4,
        );

        let invalid = vec![
            (
                planning_reset_correlation(31, workspace_directory),
                request.clone(),
                "planning editor mutation operation mismatch",
            ),
            (
                correlation.clone(),
                wrong_action,
                "planning editor mutation request identity mismatch",
            ),
            (
                correlation.clone(),
                wrong_target,
                "planning editor mutation request identity mismatch",
            ),
            (
                correlation.clone(),
                wrong_draft,
                "planning editor mutation request identity mismatch",
            ),
            (
                correlation.clone(),
                wrong_source_generation,
                "planning editor mutation request identity mismatch",
            ),
            (
                correlation.clone(),
                wrong_source_workspace,
                "planning editor mutation request identity mismatch",
            ),
            (
                correlation.clone(),
                wrong_source_draft,
                "planning editor mutation request identity mismatch",
            ),
            (
                correlation.clone(),
                wrong_buffer_revision,
                "planning editor mutation request identity mismatch",
            ),
            (
                wrong_workspace_correlation,
                request,
                "planning editor mutation request identity mismatch",
            ),
            (
                planning_editor_mutation_correlation(
                    31,
                    workspace_directory,
                    malformed_source_workspace.clone(),
                ),
                planning_editor_mutation_request(malformed_source_workspace, "body"),
                "planning editor mutation request identity mismatch",
            ),
            (
                planning_editor_mutation_correlation(
                    31,
                    workspace_directory,
                    malformed_source_draft.clone(),
                ),
                planning_editor_mutation_request(malformed_source_draft, "body"),
                "planning editor mutation request identity mismatch",
            ),
        ];

        for (correlation, request, expected_error) in invalid {
            let CoreEffectCompletion::PlanningEditorMutationCompleted { result, .. } =
                planning_editor_mutation_completion(
                    &runner.planning_workspace,
                    correlation,
                    request,
                )
            else {
                panic!("mutation validation should return its typed completion");
            };
            assert_eq!(
                result.expect_err("malformed mutation must fail"),
                expected_error
            );
            assert_eq!(replace_call_count.load(Ordering::SeqCst), 0);
            assert_eq!(load_call_count.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn planning_editor_mutation_runs_exactly_one_save_or_promote_use_case() {
        for (action, target) in [
            (
                PlanningEditorMutationAction::Save,
                PlanningEditorMutationTarget::Planning,
            ),
            (
                PlanningEditorMutationAction::Promote,
                PlanningEditorMutationTarget::Directions,
            ),
        ] {
            let workspace_directory = "/workspace";
            let draft_name = match action {
                PlanningEditorMutationAction::Save => "mutation-valid-save",
                PlanningEditorMutationAction::Promote => "mutation-valid-promote",
            };
            let identity = planning_editor_mutation_identity(
                action,
                target,
                draft_name,
                5,
                workspace_directory,
                9,
            );
            let correlation =
                planning_editor_mutation_correlation(13, workspace_directory, identity.clone());
            let (planning_workspace, replace_call_count, load_call_count) =
                gated_editor_mutation_workspace(None, false);
            let runtime_port = Arc::new(GatedRuntimePort::default());
            let (input_sender, _input_receiver) = core_input_channel();
            let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);

            let CoreEffectCompletion::PlanningEditorMutationCompleted {
                correlation: completed_correlation,
                result,
            } = planning_editor_mutation_completion(
                &runner.planning_workspace,
                correlation.clone(),
                planning_editor_mutation_request(identity.clone(), "edited body"),
            )
            else {
                panic!("mutation should return its typed completion");
            };
            assert_eq!(completed_correlation, correlation);
            match (
                action,
                result.expect("valid mutation should complete").as_ref(),
            ) {
                (
                    PlanningEditorMutationAction::Save,
                    PlanningEditorMutationResult::Saved {
                        identity: completed_identity,
                        draft_name: completed_draft,
                        ..
                    },
                ) => {
                    assert_eq!(completed_identity, &identity);
                    assert_eq!(completed_draft, draft_name);
                }
                (
                    PlanningEditorMutationAction::Promote,
                    PlanningEditorMutationResult::Promoted {
                        identity: completed_identity,
                        draft_name: completed_draft,
                        promoted_file_count,
                        ..
                    },
                ) => {
                    assert_eq!(completed_identity, &identity);
                    assert_eq!(completed_draft, draft_name);
                    assert_eq!(*promoted_file_count, 1);
                }
                (_, result) => panic!("wrong mutation result variant: {result:?}"),
            }
            assert_eq!(replace_call_count.load(Ordering::SeqCst), 1);
            assert_eq!(
                load_call_count.load(Ordering::SeqCst),
                match action {
                    PlanningEditorMutationAction::Save => 1,
                    PlanningEditorMutationAction::Promote => 2,
                },
                "only promotion may perform the active workspace write"
            );
        }
    }

    #[test]
    fn planning_editor_mutation_dispatch_is_non_blocking_and_coordinates_duplicates() {
        let workspace_directory = "/tmp/gated-editor-mutation";
        let draft_name = "mutation-valid-gated";
        let (mutation_gate, gate_entered, gate_release) = one_shot_gate();
        let (planning_workspace, replace_call_count, load_call_count) =
            gated_editor_mutation_workspace(Some(mutation_gate), false);
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let runtime = CoreRuntime::new(runner, input_receiver);
        let identity = planning_editor_mutation_identity(
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationTarget::Planning,
            draft_name,
            3,
            workspace_directory,
            7,
        );
        let command = AppCommand::MutatePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            request: Box::new(planning_editor_mutation_request(
                identity.clone(),
                "SENSITIVE-GATED-EDITOR-BODY",
            )),
        };
        assert!(!format!("{command:?}").contains("SENSITIVE-GATED-EDITOR-BODY"));
        let (dispatch_tx, dispatch_rx) = mpsc::sync_channel(1);
        let dispatcher = thread::spawn({
            let command = command.clone();
            move || {
                let mut runtime = runtime;
                let outcome = runtime.dispatch_command(command);
                dispatch_tx
                    .send((runtime, outcome))
                    .expect("mutation dispatch should return to the test");
            }
        });

        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("mutation provider should reach the gate");
        thread::sleep(Duration::from_millis(650));
        let (mut runtime, started) = dispatch_rx
            .recv_timeout(Duration::from_millis(300))
            .expect("mutation dispatch must return while provider I/O remains blocked");
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("mutation should be admitted");
        };
        let correlation = correlation.clone();
        assert_eq!(replace_call_count.load(Ordering::SeqCst), 1);
        assert_eq!(load_call_count.load(Ordering::SeqCst), 0);

        let duplicate = runtime.dispatch_command(command.clone());
        assert_eq!(
            duplicate.events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: correlation.clone(),
                },
            )]
        );
        let mut busy_identity = identity;
        busy_identity.buffer_revision += 1;
        let busy = runtime.dispatch_command(AppCommand::MutatePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            request: Box::new(planning_editor_mutation_request(
                busy_identity,
                "newer body",
            )),
        });
        assert!(matches!(
            busy.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation,
                    ..
                }
            )] if active_correlation == &correlation
        ));
        assert_eq!(replace_call_count.load(Ordering::SeqCst), 1);

        gate_release
            .send(())
            .expect("mutation provider should be released");
        let completion = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningEditorMutationCompleted {
                    correlation: completed,
                    result: Ok(result),
                }] if completed == &correlation
                    && matches!(result.as_ref(), PlanningEditorMutationResult::Saved { .. })
            )
        });
        assert_eq!(completion.events.len(), 1);
        assert_eq!(replace_call_count.load(Ordering::SeqCst), 1);
        assert_eq!(load_call_count.load(Ordering::SeqCst), 1);
        dispatcher
            .join()
            .expect("mutation dispatch thread should not panic");
    }

    #[test]
    fn planning_editor_save_and_promote_panics_are_redacted_once_and_reopen_admission() {
        for action in [
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationAction::Promote,
        ] {
            let workspace_directory = "/tmp/panicking-editor-mutation";
            let draft_name = match action {
                PlanningEditorMutationAction::Save => "mutation-valid-panic-save",
                PlanningEditorMutationAction::Promote => "mutation-valid-panic-promote",
            };
            let identity = planning_editor_mutation_identity(
                action,
                PlanningEditorMutationTarget::Planning,
                draft_name,
                8,
                workspace_directory,
                2,
            );
            let command = AppCommand::MutatePlanningEditor {
                workspace_directory: workspace_directory.to_string(),
                request: Box::new(planning_editor_mutation_request(
                    identity,
                    SENSITIVE_EDITOR_MUTATION_PANIC_PAYLOAD,
                )),
            };
            let (planning_workspace, replace_call_count, load_call_count) =
                gated_editor_mutation_workspace(None, true);
            let runtime_port = Arc::new(GatedRuntimePort::default());
            let (input_sender, input_receiver) = core_input_channel();
            let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
            let mut runtime = CoreRuntime::new(runner, input_receiver);

            let started = runtime.dispatch_command(command.clone());
            let [
                AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                    PlanningWorkspaceOperationAdmission::Started { correlation },
                ),
            ] = started.events.as_slice()
            else {
                panic!("panicking mutation should start");
            };
            let first = correlation.clone();
            let panicked = poll_until(&mut runtime, |outcome| {
                matches!(
                    outcome.events.as_slice(),
                    [AppEvent::PlanningEditorMutationCompleted {
                        correlation,
                        result: Err(error),
                    }] if correlation == &first
                        && error == "planning editor mutation worker panicked"
                )
            });
            assert_eq!(panicked.events.len(), 1);
            assert!(runtime.poll_pending_input().is_none());
            assert_eq!(replace_call_count.load(Ordering::SeqCst), 1);
            assert_eq!(load_call_count.load(Ordering::SeqCst), 0);

            let retried = runtime.dispatch_command(command);
            assert!(matches!(
                retried.events.as_slice(),
                [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                    PlanningWorkspaceOperationAdmission::Started { correlation }
                )] if correlation.generation == first.generation + 1
            ));
            let completed = poll_until(&mut runtime, |outcome| {
                matches!(
                    outcome.events.as_slice(),
                    [AppEvent::PlanningEditorMutationCompleted { result: Ok(_), .. }]
                )
            });
            assert_eq!(completed.events.len(), 1);
            assert!(runtime.poll_pending_input().is_none());
            assert_eq!(replace_call_count.load(Ordering::SeqCst), 2);
            assert_eq!(
                load_call_count.load(Ordering::SeqCst),
                match action {
                    PlanningEditorMutationAction::Save => 1,
                    PlanningEditorMutationAction::Promote => 2,
                }
            );
        }
    }

    #[test]
    fn planning_editor_stage_dispatch_is_non_blocking_and_exact_duplicates_coalesce() {
        let workspace_directory = "/tmp/gated-editor-stage";
        let (unused_load_gate, _unused_entered, _unused_release) = one_shot_gate();
        let (stage_gate, gate_entered, gate_release) = one_shot_gate();
        let stage_call_count = Arc::new(AtomicUsize::new(0));
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_load_gate,
            simple_authoring_gate: Some(stage_gate),
            stage_call_count: stage_call_count.clone(),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let runtime = CoreRuntime::new(runner, input_receiver);
        let command = AppCommand::StagePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            target: PlanningEditorStageTarget::PlanningManual,
        };
        let (dispatch_tx, dispatch_rx) = mpsc::sync_channel(1);
        let dispatcher = thread::spawn({
            let command = command.clone();
            move || {
                let mut runtime = runtime;
                let outcome = runtime.dispatch_command(command);
                dispatch_tx
                    .send((runtime, outcome))
                    .expect("editor-stage dispatch should return to the test");
            }
        });

        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("editor-stage provider should reach the gate");
        thread::sleep(Duration::from_millis(650));
        let (mut runtime, started) = dispatch_rx
            .recv_timeout(Duration::from_millis(300))
            .expect("editor-stage dispatch must return while provider I/O remains blocked");
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("editor stage should be admitted");
        };
        let correlation = correlation.clone();

        let duplicate = runtime.dispatch_command(command.clone());
        assert_eq!(
            duplicate.events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: correlation.clone(),
                },
            )]
        );
        assert!(duplicate.effects.is_empty());
        let busy = runtime.dispatch_command(AppCommand::StagePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            target: PlanningEditorStageTarget::QueueIdlePrompt,
        });
        assert!(matches!(
            busy.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation,
                    ..
                }
            )] if active_correlation == &correlation
        ));
        assert_eq!(stage_call_count.load(Ordering::SeqCst), 1);

        gate_release
            .send(())
            .expect("editor-stage provider gate should release");
        dispatcher
            .join()
            .expect("editor-stage dispatch thread should not panic");
        let completed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningEditorStaged {
                    correlation: completed,
                    result: Err(error),
                }] if completed == &correlation
                    && error == "synthetic stage should not complete"
            )
        });
        assert_eq!(completed.events.len(), 1);
        assert!(matches!(
            runtime.dispatch_command(command).events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation: next }
            )] if next.generation == correlation.generation + 1
        ));
    }

    #[test]
    fn planning_editor_stage_runner_rejects_wrong_operation_target_session_and_draft() {
        let target = PlanningEditorStageTarget::PlanningManual;
        let correlation = planning_editor_stage_correlation(7, "/workspace", target.clone());
        let valid = planning_editor_stage_snapshot(
            &correlation,
            target.clone(),
            correlation.editor_session_identity("draft-a"),
        );
        assert_eq!(
            validate_planning_editor_stage_snapshot(&correlation, &target, valid.clone())
                .expect("exact editor-stage snapshot should pass"),
            valid
        );

        for malformed in [
            planning_editor_stage_snapshot(
                &correlation,
                PlanningEditorStageTarget::QueueIdlePrompt,
                correlation.editor_session_identity("draft-a"),
            ),
            planning_editor_stage_snapshot(
                &correlation,
                target.clone(),
                crate::core::app::PlanningEditorSessionIdentity::new(
                    correlation.generation + 1,
                    "/workspace",
                    "draft-a",
                ),
            ),
            planning_editor_stage_snapshot(
                &correlation,
                target.clone(),
                crate::core::app::PlanningEditorSessionIdentity::new(
                    correlation.generation,
                    "/other",
                    "draft-a",
                ),
            ),
            planning_editor_stage_snapshot(
                &correlation,
                target.clone(),
                correlation.editor_session_identity(""),
            ),
        ] {
            assert!(
                validate_planning_editor_stage_snapshot(&correlation, &target, malformed).is_err()
            );
        }

        let (unused_load_gate, _unused_entered, _unused_release) = one_shot_gate();
        let stage_call_count = Arc::new(AtomicUsize::new(0));
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_load_gate,
            simple_authoring_gate: None,
            stage_call_count: stage_call_count.clone(),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, _input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let wrong_operation = planning_reset_correlation(8, "/workspace");
        let CoreEffectCompletion::PlanningEditorStaged { result, .. } =
            planning_editor_stage_completion(&runner.planning_workspace, wrong_operation)
        else {
            panic!("wrong operation should still return a typed editor-stage completion");
        };
        assert_eq!(
            result.expect_err("wrong operation should fail"),
            "planning editor stage operation mismatch"
        );
        assert_eq!(stage_call_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn planning_editor_session_snapshot_preserves_source_planning_revision() {
        let correlation = planning_editor_stage_correlation(
            9,
            "/workspace",
            PlanningEditorStageTarget::DirectionDetail {
                direction_id: "general".to_string(),
            },
        );
        let snapshot = planning_editor_session_snapshot(
            &correlation,
            ApplicationPlanningDraftEditorSession {
                draft_name: "draft-a".to_string(),
                draft_directory: "/workspace/drafts/draft-a".to_string(),
                editable_files: Vec::new(),
                validation_report: Default::default(),
                source_planning_revision: Some(17),
            },
        );

        assert_eq!(snapshot.source_planning_revision, Some(17));
    }

    #[test]
    fn planning_editor_stage_worker_panic_is_redacted_and_reopens_admission() {
        let workspace_directory = "/tmp/panicking-editor-stage";
        let (unused_load_gate, _unused_entered, _unused_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_load_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(true),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);
        let command = AppCommand::StagePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            target: PlanningEditorStageTarget::PlanningManual,
        };
        let started = runtime.dispatch_command(command.clone());
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("editor stage should start");
        };
        let first = correlation.clone();
        let panicked = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningEditorStaged {
                    correlation,
                    result: Err(error),
                }] if correlation == &first
                    && error == "planning editor stage worker panicked"
            )
        });
        assert_eq!(panicked.events.len(), 1);
        assert!(matches!(
            runtime.dispatch_command(command).events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation == first.generation + 1
        ));
    }

    #[test]
    fn simple_draft_stage_dispatch_returns_within_300ms_while_provider_stays_gated() {
        let workspace_directory = "/tmp/gated-simple-stage";
        let (unused_load_gate, _unused_entered, _unused_release) = one_shot_gate();
        let (authoring_gate, gate_entered, gate_release) = one_shot_gate();
        let stage_call_count = Arc::new(AtomicUsize::new(0));
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_load_gate,
            simple_authoring_gate: Some(authoring_gate),
            stage_call_count: stage_call_count.clone(),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let runtime = CoreRuntime::new(runner, input_receiver);
        let (dispatch_tx, dispatch_rx) = mpsc::sync_channel(1);
        let dispatcher = thread::spawn(move || {
            let mut runtime = runtime;
            let outcome = runtime.dispatch_command(AppCommand::StageSimplePlanningDraft {
                workspace_directory: workspace_directory.to_string(),
            });
            dispatch_tx
                .send((runtime, outcome))
                .expect("simple stage dispatch should return to the test");
        });

        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("simple stage provider should reach the gate");
        thread::sleep(Duration::from_millis(650));
        let (mut runtime, accepted) = dispatch_rx
            .recv_timeout(Duration::from_millis(300))
            .expect("simple stage dispatch must return within 300ms while provider I/O is blocked");
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = accepted.events.as_slice()
        else {
            panic!("simple stage should be admitted");
        };
        let correlation = correlation.clone();
        assert!(matches!(
            &correlation.operation,
            PlanningWorkspaceOperationKind::StageSimpleDraft
        ));

        let duplicate = runtime.dispatch_command(AppCommand::StageSimplePlanningDraft {
            workspace_directory: workspace_directory.to_string(),
        });
        assert_eq!(
            duplicate.events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: correlation.clone(),
                },
            )]
        );
        assert!(duplicate.effects.is_empty());
        assert_eq!(stage_call_count.load(Ordering::SeqCst), 1);

        gate_release
            .send(())
            .expect("simple stage provider gate should release");
        dispatcher
            .join()
            .expect("simple stage dispatch thread should not panic");
        let completed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningSimpleDraftStaged {
                    correlation: completed,
                    result: Err(error),
                }] if completed == &correlation
                    && error == "synthetic stage should not complete"
            )
        });
        assert!(matches!(
            completed.events.as_slice(),
            [AppEvent::PlanningSimpleDraftStaged { correlation: completed, .. }]
                if completed == &correlation
        ));
    }

    #[test]
    fn mismatched_simple_authoring_source_session_never_calls_the_provider() {
        let workspace_directory = "/tmp/simple-source-workspace-a";
        let draft_name = "draft-a";
        let (unused_load_gate, _unused_entered, _unused_release) = one_shot_gate();
        let provider_call_count = Arc::new(AtomicUsize::new(0));
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_load_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: provider_call_count.clone(),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, _input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);

        let load_correlation = PlanningWorkspaceOperationCorrelation {
            generation: 1,
            workspace_directory: workspace_directory.to_string(),
            operation: PlanningWorkspaceOperationKind::LoadSimpleEditor {
                draft_name: draft_name.to_string(),
                source_session: planning_editor_session(
                    8,
                    "/tmp/simple-source-workspace-b",
                    draft_name,
                ),
            },
        };
        let CoreEffectCompletion::PlanningSimpleEditorLoaded {
            correlation,
            result,
        } = simple_planning_editor_load_completion(
            &runner.planning_workspace,
            load_correlation.clone(),
        )
        else {
            panic!("malformed load should produce a typed load completion");
        };
        assert_eq!(correlation, load_correlation);
        assert_eq!(
            result.expect_err("malformed load should fail"),
            "planning simple editor load source session mismatch"
        );
        assert_eq!(provider_call_count.load(Ordering::SeqCst), 0);

        let promotion_correlation = PlanningWorkspaceOperationCorrelation {
            generation: 2,
            workspace_directory: workspace_directory.to_string(),
            operation: PlanningWorkspaceOperationKind::PromoteSimpleDraft {
                draft_name: draft_name.to_string(),
                source_session: planning_editor_session(9, workspace_directory, "different-draft"),
            },
        };
        let CoreEffectCompletion::PlanningSimpleDraftPromoted {
            correlation,
            result,
        } = simple_planning_draft_promotion_completion(
            &runner.planning_workspace,
            promotion_correlation.clone(),
        )
        else {
            panic!("malformed promotion should produce a typed promotion completion");
        };
        assert_eq!(correlation, promotion_correlation);
        assert_eq!(
            result.expect_err("malformed promotion should fail"),
            "planning simple draft promotion source session mismatch"
        );
        assert_eq!(provider_call_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn simple_editor_worker_panic_returns_one_redacted_completion_and_reopens_admission() {
        let workspace_directory = "/tmp/panicking-simple-editor";
        let draft_name = "draft-a";
        let (unused_load_gate, _unused_entered, _unused_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_load_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(true),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);
        let command = AppCommand::LoadSimplePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            draft_name: draft_name.to_string(),
            source_session: planning_editor_session(8, workspace_directory, draft_name),
        };
        let started = runtime.dispatch_command(command.clone());
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("simple editor load should start");
        };
        let first = correlation.clone();

        let panicked = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningSimpleEditorLoaded {
                    correlation,
                    result: Err(error),
                }] if correlation == &first
                    && error == "planning simple editor load worker panicked"
            )
        });
        assert!(matches!(
            panicked.events.as_slice(),
            [AppEvent::PlanningSimpleEditorLoaded {
                correlation,
                result: Err(error),
            }] if correlation == &first
                && error == "planning simple editor load worker panicked"
        ));

        let reopened = runtime.dispatch_command(command);
        assert!(matches!(
            reopened.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation == first.generation + 1
        ));
    }

    #[test]
    fn planning_reset_dispatch_returns_while_provider_remains_gated_for_600ms() {
        let workspace_directory = "/tmp/gated-planning-reset";
        let (planning_gate, gate_entered, gate_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: planning_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let runtime = CoreRuntime::new(runner, input_receiver);
        let (dispatch_tx, dispatch_rx) = mpsc::sync_channel(1);
        let dispatcher = thread::spawn(move || {
            let mut runtime = runtime;
            let outcome = runtime.dispatch_command(AppCommand::ResetPlanningWorkspace(
                planning_reset_intent(workspace_directory),
            ));
            dispatch_tx
                .send((runtime, outcome))
                .expect("reset dispatch result should return to the test");
        });

        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("reset provider should reach the gate");
        let gated_at = Instant::now();
        thread::sleep(Duration::from_millis(650));
        let (mut runtime, accepted) = dispatch_rx
            .recv_timeout(NONBLOCKING_DISPATCH_TIMEOUT)
            .expect("reset dispatch must return while provider I/O remains blocked");
        let correlation = planning_reset_correlation(1, workspace_directory);
        assert_eq!(
            accepted.events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started {
                    correlation: correlation.clone(),
                },
            )]
        );

        let duplicate = runtime.dispatch_command(AppCommand::ResetPlanningWorkspace(
            planning_reset_intent(workspace_directory),
        ));
        assert_eq!(
            duplicate.events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: correlation.clone(),
                },
            )]
        );
        assert!(duplicate.effects.is_empty());
        assert!(gated_at.elapsed() >= Duration::from_millis(600));

        gate_release
            .send(())
            .expect("reset provider gate should release");
        dispatcher
            .join()
            .expect("reset dispatch thread should not panic");
        let completed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningWorkspaceResetCompleted {
                    correlation: completed,
                    result: Err(error),
                }] if completed == &correlation
                    && error.contains("manual preparation authority unavailable after gate release")
            )
        });
        assert!(matches!(
            completed.events.as_slice(),
            [AppEvent::PlanningWorkspaceResetCompleted {
                correlation: completed,
                result: Err(_),
            }] if completed == &correlation
        ));
    }

    #[test]
    fn planning_reset_worker_panic_returns_exact_error_and_reopens_admission() {
        let workspace_directory = "/tmp/panicking-planning-reset";
        let (planning_gate, gate_entered, gate_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: planning_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(true),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);
        let first = planning_reset_correlation(1, workspace_directory);

        runtime.dispatch_command(AppCommand::ResetPlanningWorkspace(planning_reset_intent(
            workspace_directory,
        )));
        let panicked = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningWorkspaceResetCompleted {
                    correlation,
                    result: Err(error),
                }] if correlation == &first
                    && error == "planning workspace reset worker panicked"
            )
        });
        assert!(matches!(
            panicked.events.as_slice(),
            [AppEvent::PlanningWorkspaceResetCompleted {
                correlation,
                result: Err(error),
            }] if correlation == &first
                && error == "planning workspace reset worker panicked"
        ));

        let reopened = runtime.dispatch_command(AppCommand::ResetPlanningWorkspace(
            planning_reset_intent(workspace_directory),
        ));
        assert!(matches!(
            reopened.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation == &planning_reset_correlation(2, workspace_directory)
        ));
        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("second reset should reach the provider");
        gate_release
            .send(())
            .expect("second reset provider gate should release");
        let _ = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningWorkspaceResetCompleted { correlation, .. }]
                    if correlation == &planning_reset_correlation(2, workspace_directory)
            )
        });
    }

    #[test]
    fn manual_prompt_preparation_dispatch_returns_before_blocked_authority_work() {
        let workspace_directory = "/tmp/gated-manual-preparation";
        let correlation = manual_correlation(1, workspace_directory);
        let (planning_gate, gate_entered, gate_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: planning_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
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
            simple_authoring_gate: None,
            stage_call_count: stage_call_count.clone(),
            promote_call_count: promote_call_count.clone(),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
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
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
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
        assert!(matches!(
            accepted.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(authority),
                AppEvent::StopRequestAdmissionResolved(StopRequestAdmission::Accepted {
                    correlation: admitted,
                }),
            ] if *admitted == correlation
                && authority.active_turn.as_ref().is_some_and(|active| {
                    active.correlation == turn_submission
                        && active.phase == crate::core::app::ActiveTurnPhase::Submitting
                        && active.turn_id.is_none()
                })
                && authority.auto_follow.continuation_paused
                && accepted.snapshot.conversation_runtime == **authority
        ));

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
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port.clone(), input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);
        let turn_request = TurnSubmissionRequest {
            workspace_directory: "/tmp/new-workspace".to_string(),
            thread_id: Some("thread-new".to_string()),
            prompt: "new work".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        };

        let stop = runtime.dispatch_command(AppCommand::RequestStopAllSessions);
        assert!(matches!(
            stop.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(authority),
                AppEvent::StopRequestAdmissionResolved(StopRequestAdmission::Accepted {
                    correlation,
                }),
            ] if *correlation == StopRequestCorrelation::new(1, None)
                && authority.active_turn.is_none()
                && authority.auto_follow.continuation_paused
                && stop.snapshot.conversation_runtime == **authority
        ));
        gate_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("stop provider should reach its gate");

        let blocked_turn =
            runtime.dispatch_command(AppCommand::SubmitTurn(Box::new(turn_request.clone())));
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
        let admitted_turn =
            runtime.dispatch_command(AppCommand::SubmitTurn(Box::new(turn_request)));
        assert!(matches!(
            admitted_turn.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(authority),
                AppEvent::TurnSubmissionAdmissionResolved(
                    TurnSubmissionAdmission::Accepted { correlation },
                ),
            ] if authority.active_turn.as_ref().is_some_and(|active| {
                active.correlation == *correlation
                    && active.phase == crate::core::app::ActiveTurnPhase::Submitting
                    && active.workspace_directory == "/tmp/new-workspace"
                    && active.turn_id.is_none()
            })
                && admitted_turn.snapshot.conversation_runtime == **authority
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
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(true),
            panic_draft_load_once: AtomicBool::new(false),
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
    fn startup_provider_panic_maps_to_one_exact_redacted_completion() {
        let correlation = StartupCheckCorrelation::new(9, "/tmp/workspace-a");
        let mut observed_workspace = String::new();

        let completion =
            guarded_startup_checks_completion(correlation.clone(), |workspace_directory| {
                observed_workspace = workspace_directory.to_string();
                std::panic::panic_any(());
            });

        assert_eq!(observed_workspace, "/tmp/workspace-a");
        assert_eq!(
            completion,
            CoreEffectCompletion::StartupChecksLoaded {
                correlation,
                result: Err("startup checks panicked".to_string()),
            }
        );
    }

    #[test]
    fn startup_provider_panic_stderr_is_redacted_in_isolated_process() {
        match std::env::var(STARTUP_PANIC_CHILD_ENV).as_deref() {
            Ok(mode @ ("maintenance" | "startup")) => {
                run_panicking_startup_worker_child(mode);
                return;
            }
            Ok("simple") => {
                run_panicking_simple_authoring_worker_child();
                return;
            }
            Ok("editor-stage") => {
                run_panicking_editor_stage_worker_child();
                return;
            }
            Ok(mode @ ("editor-mutation-save" | "editor-mutation-promote")) => {
                run_panicking_editor_mutation_worker_child(mode);
                return;
            }
            Ok("normal") => {
                assert!(catch_redacted_worker_unwind(|| ()).is_ok());
                panic!("{NORMAL_PANIC_PAYLOAD}");
            }
            _ => {}
        }

        let executable = std::env::current_exe().expect("test executable should resolve");
        for mode in [
            "maintenance",
            "startup",
            "simple",
            "editor-stage",
            "editor-mutation-save",
            "editor-mutation-promote",
        ] {
            let redacted_output = Command::new(&executable)
                .args(["--exact", STARTUP_PANIC_TEST_NAME, "--nocapture"])
                .env(STARTUP_PANIC_CHILD_ENV, mode)
                .output()
                .expect("redacted startup panic child should run");
            assert!(
                redacted_output.status.success(),
                "redacted startup panic child should complete successfully"
            );
            let redacted_stdout =
                String::from_utf8(redacted_output.stdout).expect("child stdout should be UTF-8");
            let redacted_stderr =
                String::from_utf8(redacted_output.stderr).expect("child stderr should be UTF-8");
            assert_eq!(
                redacted_stdout.matches(STARTUP_COMPLETION_MARKER).count(),
                1
            );
            for sensitive_payload in [
                SENSITIVE_MAINTENANCE_PANIC_PAYLOAD,
                SENSITIVE_SIMPLE_AUTHORING_PANIC_PAYLOAD,
                SENSITIVE_EDITOR_STAGE_PANIC_PAYLOAD,
                SENSITIVE_EDITOR_MUTATION_PANIC_PAYLOAD,
                SENSITIVE_STARTUP_PANIC_PAYLOAD,
            ] {
                assert!(!redacted_stdout.contains(sensitive_payload));
                assert!(!redacted_stderr.contains(sensitive_payload));
            }
            assert_eq!(
                redacted_stderr
                    .matches(crate::panic_observation::REDACTED_WORKER_PANIC_MESSAGE)
                    .count(),
                1
            );
        }

        let normal_output = Command::new(executable)
            .args(["--exact", STARTUP_PANIC_TEST_NAME, "--nocapture"])
            .env(STARTUP_PANIC_CHILD_ENV, "normal")
            .output()
            .expect("ordinary panic child should run");
        assert!(
            !normal_output.status.success(),
            "ordinary panic child should fail through the delegated hook"
        );
        let normal_stderr =
            String::from_utf8(normal_output.stderr).expect("child stderr should be UTF-8");
        assert!(normal_stderr.contains(NORMAL_PANIC_PAYLOAD));
        assert!(!normal_stderr.contains(crate::panic_observation::REDACTED_WORKER_PANIC_MESSAGE));
    }

    fn run_panicking_startup_worker_child(mode: &str) {
        let workspace_directory = "/tmp/workspace-redacted";
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let startup_service = match mode {
            "maintenance" => StartupService::new(runtime_port.clone()).with_prompt_log_maintenance(
                Arc::new(PanickingPromptLogMaintenancePort),
                AppServerPromptLogMaintenanceMode::ClearAll,
            ),
            "startup" => StartupService::new(Arc::new(PanickingStartupProbePort)),
            _ => unreachable!("isolated child mode should be validated by the parent test"),
        }
        .with_test_local_startup_prerequisites(workspace_directory);
        let (unused_gate, _unused_entered, _unused_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(false),
        });
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner_with_startup_service(
            startup_service,
            planning_workspace,
            runtime_port,
            input_sender,
        );
        let mut runtime = CoreRuntime::new(runner, input_receiver);

        let admission = runtime.dispatch_command(AppCommand::RunStartupChecks {
            workspace_directory: workspace_directory.to_string(),
        });
        assert!(matches!(
            admission.events.as_slice(),
            [AppEvent::StartupChanged {
                snapshot: StartupSnapshot::Loading,
                ..
            }]
        ));

        let deadline = Instant::now() + WORKER_COMPLETION_TIMEOUT;
        let completion = loop {
            if let Some(outcome) = runtime.poll_pending_input() {
                break outcome;
            }
            assert!(
                Instant::now() < deadline,
                "startup worker completion should return before the deadline"
            );
            thread::sleep(Duration::from_millis(5));
        };
        match mode {
            "maintenance" => assert!(matches!(
                completion.events.as_slice(),
                [AppEvent::StartupChanged {
                    snapshot: StartupSnapshot::Ready(ready),
                    ..
                }] if ready.warnings == [
                    "app-server prompt-log privacy maintenance panicked; startup continued"
                ]
            )),
            "startup" => assert!(matches!(
                completion.events.as_slice(),
                [AppEvent::StartupChanged {
                    snapshot: StartupSnapshot::Failed { message },
                    ..
                }] if message == "startup checks panicked"
            )),
            _ => unreachable!("isolated child mode should be validated by the parent test"),
        }

        let quiet_deadline = Instant::now() + Duration::from_millis(100);
        let mut received_completion_inputs = 1;
        while Instant::now() < quiet_deadline {
            if runtime.poll_pending_input().is_some() {
                received_completion_inputs += 1;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(received_completion_inputs, 1);
        println!("{STARTUP_COMPLETION_MARKER}");
    }

    fn run_panicking_simple_authoring_worker_child() {
        let workspace_directory = "/tmp/simple-authoring-redacted";
        let draft_name = "draft-redacted";
        let (unused_gate, _unused_entered, _unused_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(true),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);

        let admission = runtime.dispatch_command(AppCommand::LoadSimplePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            draft_name: draft_name.to_string(),
            source_session: planning_editor_session(1, workspace_directory, draft_name),
        });
        assert!(matches!(
            admission.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { .. }
            )]
        ));
        let completion = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningSimpleEditorLoaded {
                    result: Err(error),
                    ..
                }] if error == "planning simple editor load worker panicked"
            )
        });
        assert!(matches!(
            completion.events.as_slice(),
            [AppEvent::PlanningSimpleEditorLoaded {
                result: Err(error),
                ..
            }] if error == "planning simple editor load worker panicked"
        ));
        assert!(
            runtime.poll_pending_input().is_none(),
            "one simple-authoring worker must emit exactly one completion"
        );
        println!("{STARTUP_COMPLETION_MARKER}");
    }

    fn run_panicking_editor_stage_worker_child() {
        let workspace_directory = "/tmp/editor-stage-redacted";
        let (unused_gate, _unused_entered, _unused_release) = one_shot_gate();
        let planning_workspace = Arc::new(GatedPlanningWorkspacePort {
            load_gate: unused_gate,
            simple_authoring_gate: None,
            stage_call_count: Arc::new(AtomicUsize::new(0)),
            promote_call_count: Arc::new(AtomicUsize::new(0)),
            panic_load_once: AtomicBool::new(false),
            panic_draft_load_once: AtomicBool::new(true),
        });
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);

        let admission = runtime.dispatch_command(AppCommand::StagePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            target: PlanningEditorStageTarget::PlanningManual,
        });
        assert!(matches!(
            admission.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { .. }
            )]
        ));
        let completion = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningEditorStaged {
                    result: Err(error),
                    ..
                }] if error == "planning editor stage worker panicked"
            )
        });
        assert!(matches!(
            completion.events.as_slice(),
            [AppEvent::PlanningEditorStaged {
                result: Err(error),
                ..
            }] if error == "planning editor stage worker panicked"
        ));
        assert!(
            runtime.poll_pending_input().is_none(),
            "one editor-stage worker must emit exactly one completion"
        );
        println!("{STARTUP_COMPLETION_MARKER}");
    }

    fn run_panicking_editor_mutation_worker_child(mode: &str) {
        let workspace_directory = "/tmp/editor-mutation-redacted";
        let action = match mode {
            "editor-mutation-save" => PlanningEditorMutationAction::Save,
            "editor-mutation-promote" => PlanningEditorMutationAction::Promote,
            _ => unreachable!("isolated child mode should be validated by the parent test"),
        };
        let draft_name = match action {
            PlanningEditorMutationAction::Save => "mutation-valid-redacted-save",
            PlanningEditorMutationAction::Promote => "mutation-valid-redacted-promote",
        };
        let identity = planning_editor_mutation_identity(
            action,
            PlanningEditorMutationTarget::Planning,
            draft_name,
            1,
            workspace_directory,
            0,
        );
        let command = AppCommand::MutatePlanningEditor {
            workspace_directory: workspace_directory.to_string(),
            request: Box::new(planning_editor_mutation_request(
                identity,
                SENSITIVE_EDITOR_MUTATION_PANIC_PAYLOAD,
            )),
        };
        let (planning_workspace, _replace_call_count, _load_call_count) =
            gated_editor_mutation_workspace(None, true);
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let (input_sender, input_receiver) = core_input_channel();
        let runner = test_effect_runner(planning_workspace, runtime_port, input_sender);
        let mut runtime = CoreRuntime::new(runner, input_receiver);

        assert!(matches!(
            runtime.dispatch_command(command).events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { .. }
            )]
        ));
        let completion = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::PlanningEditorMutationCompleted {
                    result: Err(error),
                    ..
                }] if error == "planning editor mutation worker panicked"
            )
        });
        assert_eq!(completion.events.len(), 1);
        assert!(
            runtime.poll_pending_input().is_none(),
            "one editor-mutation worker must emit exactly one completion"
        );
        println!("{STARTUP_COMPLETION_MARKER}");
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
    fn session_catalog_worker_panic_returns_one_failure_and_reopens_loading_gate() {
        let runtime_port = Arc::new(GatedRuntimePort::panicking_session_catalog_once());
        let mut runtime = test_core_runtime(runtime_port);
        let command = AppCommand::LoadSessionCatalog(SessionCatalogLoadIntent::refresh(
            10,
            "/tmp/session-catalog-panic",
        ));

        assert!(matches!(
            runtime.dispatch_command(command.clone()).events.as_slice(),
            [AppEvent::SessionCatalogChanged(
                SessionCatalogSnapshot::Loading
            )]
        ));
        let failed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::SessionCatalogChanged(
                    SessionCatalogSnapshot::Failed { message }
                )] if message == "session catalog worker panicked"
            )
        });
        assert_eq!(failed.events.len(), 1);
        assert!(
            runtime.poll_pending_input().is_none(),
            "one panicking catalog worker must emit exactly one completion"
        );

        assert!(matches!(
            runtime.dispatch_command(command).events.as_slice(),
            [AppEvent::SessionCatalogChanged(
                SessionCatalogSnapshot::Loading
            )]
        ));
        let recovered = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::SessionCatalogChanged(
                    SessionCatalogSnapshot::Ready(_)
                )]
            )
        });
        assert_eq!(recovered.events.len(), 1);
    }

    #[test]
    fn session_catalog_effect_derives_provider_request_from_full_correlation() {
        let runtime_port = Arc::new(GatedRuntimePort::default());
        let mut runtime = test_core_runtime(runtime_port.clone());
        let limit = 23;
        let workspace_directory = "/tmp/session-catalog-correlation";
        let expected_request = SessionCatalogRequest::for_workspace(limit, workspace_directory);

        assert!(matches!(
            runtime
                .dispatch_command(AppCommand::LoadSessionCatalog(
                    SessionCatalogLoadIntent::refresh(limit, workspace_directory),
                ))
                .events
                .as_slice(),
            [AppEvent::SessionCatalogChanged(
                SessionCatalogSnapshot::Loading
            )]
        ));
        let completed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::SessionCatalogChanged(
                    SessionCatalogSnapshot::Ready(_)
                )]
            )
        });
        assert_eq!(completed.events.len(), 1);
        assert_eq!(
            *runtime_port
                .session_catalog_requests
                .lock()
                .expect("session catalog request log should lock"),
            vec![expected_request],
            "composition must derive the provider request from the exact Core correlation"
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
    fn github_review_setup_worker_panic_returns_one_exact_failure() {
        let mut runtime = github_review_test_runtime(|_| {
            panic!("synthetic setup panic");
        });
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let correlation = GithubReviewPollingSetupCorrelation::new(1, "/workspace");

        runtime.dispatch_command(explicit_github_review_setup("/workspace", target));
        let completed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::GithubReviewPollingSetupCompleted {
                    correlation: completed,
                    result: Err(message),
                }] if completed == &correlation
                    && message == "GitHub review polling setup worker panicked"
            )
        });
        assert!(matches!(
            completed.events.as_slice(),
            [AppEvent::GithubReviewPollingSetupCompleted {
                correlation: completed,
                result: Err(message),
            }] if completed == &correlation
                && message == "GitHub review polling setup worker panicked"
        ));
        assert!(
            runtime.poll_pending_input().is_none(),
            "one setup worker must emit exactly one completion"
        );
        let poll = runtime.dispatch_command(AppCommand::PollGithubReview);
        assert!(
            poll.effects.is_empty(),
            "panic must leave setup fail-closed"
        );
    }

    #[test]
    fn newer_github_review_setup_replaces_active_service_cursor_and_in_flight_poll() {
        let (a_poll_gate, a_poll_entered, a_poll_release) = one_shot_gate();
        let mut runtime = github_review_test_runtime(move |request| {
            let target = request
                .mode
                .explicit_target()
                .expect("test setup should be explicit")
                .clone();
            if request.workspace_directory == "/workspace-a" {
                return Ok(Some((
                    target,
                    github_review_service_with_second_load_gate("service-a", a_poll_gate.clone()),
                )));
            }
            Ok(Some((target, github_review_service("service-b"))))
        });
        let target_a = GithubPullRequestTarget::new("acme/widgets", 41);
        let target_b = GithubPullRequestTarget::new("acme/widgets", 42);

        runtime.dispatch_command(explicit_github_review_setup(
            "/workspace-a",
            target_a.clone(),
        ));
        let setup_a = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::GithubReviewPollingSetupCompleted {
                    correlation: GithubReviewPollingSetupCorrelation {
                        generation: 1,
                        workspace_directory,
                    },
                    result: Ok(GithubReviewPollingSetupResult::Active { target }),
                }] if workspace_directory == "/workspace-a" && target == &target_a
            )
        });
        assert!(setup_a.effects.is_empty());

        let first_a = runtime.dispatch_command(AppCommand::PollGithubReview);
        assert!(matches!(
            first_a.effects.as_slice(),
            [CoreEffect::PollGithubReview {
                setup_correlation: GithubReviewPollingSetupCorrelation {
                    generation: 1,
                    workspace_directory,
                },
                correlation: GithubReviewPollCorrelation {
                    generation: 1,
                    target,
                },
                previous_state: None,
            }] if workspace_directory == "/workspace-a" && target == &target_a
        ));
        let first_a_completed = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::GithubReviewPollCompleted {
                    correlation: GithubReviewPollCorrelation {
                        generation: 1,
                        target,
                    },
                    result: Ok(result),
                }] if target == &target_a
                    && result.snapshot.title == "service-a"
                    && result.next_state.latest_submitted_at.as_deref()
                        == Some("2026-07-19T10:00:00Z")
            )
        });
        assert!(first_a_completed.effects.is_empty());

        let second_a = runtime.dispatch_command(AppCommand::PollGithubReview);
        assert!(matches!(
            second_a.effects.as_slice(),
            [CoreEffect::PollGithubReview {
                setup_correlation: GithubReviewPollingSetupCorrelation {
                    generation: 1,
                    workspace_directory,
                },
                correlation: GithubReviewPollCorrelation {
                    generation: 2,
                    target,
                },
                previous_state: Some(previous_state),
            }] if workspace_directory == "/workspace-a"
                && target == &target_a
                && previous_state.latest_submitted_at.as_deref()
                    == Some("2026-07-19T10:00:00Z")
        ));
        a_poll_entered
            .recv_timeout(WORKER_COMPLETION_TIMEOUT)
            .expect("the second poll for setup A should reach its gate");

        runtime.dispatch_command(explicit_github_review_setup(
            "/workspace-b",
            target_b.clone(),
        ));
        let setup_b = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::GithubReviewPollingSetupCompleted {
                    correlation: GithubReviewPollingSetupCorrelation {
                        generation: 2,
                        workspace_directory,
                    },
                    result: Ok(GithubReviewPollingSetupResult::Active { target }),
                }] if workspace_directory == "/workspace-b" && target == &target_b
            )
        });
        assert!(setup_b.effects.is_empty());

        a_poll_release
            .send(())
            .expect("the in-flight poll for setup A should be released");
        let stale_a_poll = poll_next(&mut runtime);
        assert!(
            stale_a_poll.events.is_empty(),
            "late poll completion from setup A must be ignored by Core"
        );

        let started = runtime.dispatch_command(AppCommand::PollGithubReview);
        assert!(matches!(
            started.effects.as_slice(),
            [CoreEffect::PollGithubReview {
                setup_correlation: GithubReviewPollingSetupCorrelation {
                    generation: 2,
                    workspace_directory,
                },
                correlation: GithubReviewPollCorrelation { target, .. },
                previous_state: None,
            }] if workspace_directory == "/workspace-b" && target == &target_b
        ));
        let polled = poll_until(&mut runtime, |outcome| {
            matches!(
                outcome.events.as_slice(),
                [AppEvent::GithubReviewPollCompleted {
                    result: Ok(result),
                    ..
                }] if result.snapshot.title == "service-b"
            )
        });
        assert!(matches!(
            polled.events.as_slice(),
            [AppEvent::GithubReviewPollCompleted {
                result: Ok(result),
                ..
            }] if result.snapshot.target == target_b
                && result.snapshot.title == "service-b"
        ));
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
    fn mismatched_post_turn_execution_becomes_one_correlated_safe_failure_with_local_revoke() {
        let continuation_gate = crate::domain::planning::PostTurnContinuationGate::default();
        let context = crate::domain::planning::PostTurnContext {
            thread_id: "thread-1".to_string(),
            planning_workspace_directory: "/tmp/planning".to_string(),
            latest_user_message: None,
            latest_main_reply: None,
            previous_handoff_task: None,
            current_runtime_projection: RuntimeProjection::invalid("refresh required"),
            parallel_mode_enabled: false,
            parallel_automation_epoch_id: None,
            planning_settlement_paused: false,
            continuation_paused: false,
            can_queue_next: false,
            stop_keyword: ":stop".to_string(),
            stop_keyword_matched: false,
            no_file_changes_stop_matched: false,
            mode_label: "test".to_string(),
        };
        let request = crate::domain::planning::PostTurnRequest {
            context,
            workspace_directory: "/tmp/turn".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
            planning_worker_panel_state: Default::default(),
            continuation_permit: continuation_gate.capture(),
        };
        let captured_permit = request.continuation_permit.clone();
        let newer_permit = continuation_gate.capture();
        let correlation = crate::core::app::PostTurnEvaluationCorrelation::new(
            17,
            "thread-1",
            "turn-1",
            "/tmp/turn",
            "/tmp/planning",
        );
        let mut execution = post_turn_evaluation_failure_execution(
            &request.context,
            &request,
            "synthetic source result".to_string(),
        );
        execution.evaluation.provenance.completed_turn_id = "forged-turn".to_string();

        let completion = post_turn_evaluation_completion(correlation.clone(), &request, execution);

        assert!(
            !captured_permit.is_current(),
            "malformed output must revoke only the originating request"
        );
        assert!(
            newer_permit.is_current(),
            "a stale malformed worker must not invalidate another request's shared gate generation"
        );
        assert!(matches!(
            completion,
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: completed_correlation,
                execution,
            } if completed_correlation == correlation
                && correlation.matches_execution(execution.as_ref())
                && execution.evaluation.runtime_notices
                    == vec!["post-turn evaluation worker returned a mismatched target"]
                && execution.planning_worker_panel_state.status
                    == crate::domain::planning::PlanningWorkerStatus::RefreshFailed
        ));
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
