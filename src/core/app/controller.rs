mod state;

use self::state::AppState;
use super::conversation_preferences::ConversationPreferenceFeatureReducer;
use super::conversation_turn_reducer::{
    ConversationLoadAdmission, ConversationTurnFeatureReducer, LoadedConversationStreamIdentity,
    StopEffectIntent,
};
use super::github_review_reducer::{GithubReviewFeatureReducer, GithubReviewPollingSetupAdmission};
use super::planning_reducer::{ManualPromptCompletionDisposition, PlanningFeatureReducer};
use super::read_model_reducer::ReadModelFeatureReducer;
use super::session_reducer::{SessionCatalogLoadReduction, SessionFeatureReducer};
use super::startup_reducer::StartupFeatureReducer;
use super::{
    AppCommand, AppEvent, AppSnapshot, ApprovalDecisionAdmission, ConversationLoadCorrelation,
    CoreEffect, CoreEffectCompletion, CoreInput, ParallelModeProjection,
    PlanningEditorMutationRequest, PlanningWorkspaceOperationAdmission,
    PlanningWorkspaceOperationIntent, PlanningWorkspaceOperationKind,
    RevisionedPlanningParallelProjection, SessionCatalogLoadIntent, SessionRenameAcceptedSnapshot,
    SessionRenameAdmission, StartupCheckCorrelation, StopRequestAdmission, StopRequestAttempt,
    TurnSteerAdmission, TurnStreamEvent, TurnSubmissionAdmission, TurnSubmissionCorrelation,
};
#[cfg(test)]
use super::{
    ApprovalDecisionCorrelation, PostTurnEvaluationCorrelation, SessionRenameCorrelation,
    StopRequestCorrelation, TurnSteerCorrelation,
};
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleProjection;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDispatchOutcome {
    pub events: Vec<AppEvent>,
    pub effects: Vec<CoreEffect>,
    pub snapshot: Arc<AppSnapshot>,
}

#[derive(Debug, Clone)]
pub(in crate::core) struct CoreController {
    state: AppState,
    startup: StartupFeatureReducer,
    session_feature: SessionFeatureReducer,
    conversation_turn: ConversationTurnFeatureReducer,
    read_models: ReadModelFeatureReducer,
    planning: PlanningFeatureReducer,
    github_review: GithubReviewFeatureReducer,
    conversation_preferences: ConversationPreferenceFeatureReducer,
}

impl CoreController {
    pub(in crate::core) fn new() -> Self {
        Self {
            state: AppState::new(),
            startup: StartupFeatureReducer::new(),
            session_feature: SessionFeatureReducer::new(),
            conversation_turn: ConversationTurnFeatureReducer::new(),
            read_models: ReadModelFeatureReducer::new(),
            planning: PlanningFeatureReducer::new(),
            github_review: GithubReviewFeatureReducer::new(),
            conversation_preferences: ConversationPreferenceFeatureReducer::new(),
        }
    }

    pub(in crate::core) fn snapshot(&self) -> AppSnapshot {
        self.state.snapshot()
    }

    fn shared_snapshot(&self) -> Arc<AppSnapshot> {
        self.state.shared_snapshot()
    }

    pub(in crate::core) fn revisioned_planning_parallel_projection(
        &self,
    ) -> RevisionedPlanningParallelProjection {
        self.state.revisioned_planning_parallel_projection()
    }

    pub(in crate::core) fn parallel_mode_projection(&self) -> ParallelModeProjection {
        self.state.parallel_mode_projection()
    }

    pub(in crate::core) fn handle_input(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        let outcome = self.handle_input_inner(input);
        self.with_conversation_runtime_authority(outcome)
    }

    fn handle_input_inner(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        match input {
            CoreInput::Command(AppCommand::Noop) => CoreDispatchOutcome {
                events: Vec::new(),
                effects: Vec::new(),
                snapshot: self.shared_snapshot(),
            },
            CoreInput::Command(AppCommand::RunStartupChecks {
                workspace_directory,
            }) => {
                let correlation = self.startup.begin(workspace_directory);
                self.state.mark_startup_loading();
                self.startup_changed_outcome(
                    correlation.clone(),
                    vec![CoreEffect::RunStartupChecks { correlation }],
                )
            }
            CoreInput::Command(AppCommand::LoadSessionCatalog(intent)) => {
                self.admit_session_catalog_load(intent)
            }
            CoreInput::Command(AppCommand::RenameSession(request)) => {
                let conversation_load_blocker = self
                    .conversation_turn
                    .active_conversation_load_for_thread(&request.thread_id);
                let admission = self
                    .session_feature
                    .reduce_rename(request, conversation_load_blocker);
                let effects = match &admission {
                    SessionRenameAdmission::Accepted { correlation } => {
                        vec![CoreEffect::RenameSession {
                            correlation: correlation.clone(),
                        }]
                    }
                    SessionRenameAdmission::RejectedActive { .. }
                    | SessionRenameAdmission::RejectedCatalogLoading { .. }
                    | SessionRenameAdmission::RejectedConversationLoading { .. } => Vec::new(),
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::SessionRenameAdmissionResolved(admission)],
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::LoadConversation {
                thread_id,
                fallback_workspace_directory,
            }) => self.start_conversation_load(thread_id, fallback_workspace_directory),
            CoreInput::Command(AppCommand::InvalidateConversationLoad) => {
                self.planning.reset_worker_panel_history();
                let cancelled_refresh = self.planning.cancel_runtime_refresh();
                let reduction = self.conversation_turn.reduce_conversation_invalidation();
                self.state.reset_conversation();
                let mut outcome = self.conversation_changed_outcome(
                    None,
                    stop_effects_from_intents(reduction.stop_effects),
                );
                if let Some(correlation) = cancelled_refresh {
                    outcome
                        .events
                        .insert(0, AppEvent::PlanningRuntimeRefreshCancelled { correlation });
                }
                outcome
            }
            CoreInput::Command(AppCommand::LoadParallelPeekConversation { thread_id }) => {
                let correlation = self.read_models.begin_parallel_peek_load(thread_id);
                CoreDispatchOutcome {
                    events: Vec::new(),
                    effects: vec![CoreEffect::LoadParallelPeekConversation { correlation }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::LoadReviewCenter {
                workspace_directory,
                active_thread_id,
            }) => {
                let correlation = self
                    .read_models
                    .begin_review_center_load(workspace_directory, active_thread_id);
                CoreDispatchOutcome {
                    events: vec![AppEvent::ReviewCenterLoadStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::LoadReviewCenter { correlation }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::LoadQueueAuthority {
                workspace_directory,
                active_thread_id,
            }) => {
                let correlation = self
                    .read_models
                    .begin_queue_authority_load(workspace_directory, active_thread_id);
                CoreDispatchOutcome {
                    events: vec![AppEvent::QueueAuthorityLoadStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::LoadQueueAuthority { correlation }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::LoadDirectionsMaintenance {
                workspace_directory,
            }) => {
                let correlation = self
                    .read_models
                    .begin_directions_maintenance_load(workspace_directory);
                CoreDispatchOutcome {
                    events: vec![AppEvent::DirectionsMaintenanceLoadStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::LoadDirectionsMaintenance { correlation }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::RefreshPlanningRuntime {
                workspace_directory,
            }) => {
                let (correlation, superseded) =
                    self.planning.begin_runtime_refresh(workspace_directory);
                let mut events = vec![AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: correlation.clone(),
                }];
                if let Some(correlation) = superseded {
                    events.push(AppEvent::PlanningRuntimeRefreshCancelled { correlation });
                }
                CoreDispatchOutcome {
                    events,
                    effects: vec![CoreEffect::LoadPlanningRuntime { correlation }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::ResetPlanningWorkspace(intent)) => self
                .begin_planning_workspace_operation(PlanningWorkspaceOperationIntent::reset(
                    intent,
                )),
            CoreInput::Command(AppCommand::StageSimplePlanningDraft {
                workspace_directory,
            }) => self.begin_planning_workspace_operation(
                PlanningWorkspaceOperationIntent::stage_simple_draft(workspace_directory),
            ),
            CoreInput::Command(AppCommand::StagePlanningEditor {
                workspace_directory,
                target,
            }) => self.begin_planning_workspace_operation(
                PlanningWorkspaceOperationIntent::stage_editor(workspace_directory, target),
            ),
            CoreInput::Command(AppCommand::MutatePlanningEditor {
                workspace_directory,
                request,
            }) => self.begin_planning_editor_mutation(workspace_directory, request),
            CoreInput::Command(AppCommand::LoadSimplePlanningEditor {
                workspace_directory,
                draft_name,
                source_session,
            }) => self.begin_planning_workspace_operation(
                PlanningWorkspaceOperationIntent::load_simple_editor(
                    workspace_directory,
                    draft_name,
                    source_session,
                ),
            ),
            CoreInput::Command(AppCommand::PromoteSimplePlanningDraft {
                workspace_directory,
                draft_name,
                source_session,
            }) => self.begin_planning_workspace_operation(
                PlanningWorkspaceOperationIntent::promote_simple_draft(
                    workspace_directory,
                    draft_name,
                    source_session,
                ),
            ),
            CoreInput::Command(AppCommand::SubmitQueueMutation(intent)) => {
                let Some(correlation) = self.planning.begin_queue_mutation(*intent) else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::QueueMutationStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::ExecuteQueueMutation { correlation }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::PrepareManualPrompt(intent)) => {
                let reduction = self.planning.begin_manual_prompt_preparation(*intent);
                let effects = reduction
                    .request
                    .into_iter()
                    .map(CoreEffect::PrepareManualPrompt)
                    .collect();
                CoreDispatchOutcome {
                    events: vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                        reduction.admission,
                    )],
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::CancelManualPromptPreparation) => {
                let Some(correlation) = self.planning.cancel_manual_prompt_preparation() else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: Vec::new(),
                    effects: vec![CoreEffect::CancelManualPromptPreparation { correlation }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::SubmitTurn(request)) => {
                let admission = self.conversation_turn.admit_turn_submission(&request);
                let effects = match &admission {
                    TurnSubmissionAdmission::Accepted { correlation } => {
                        vec![CoreEffect::SubmitTurn {
                            correlation: *correlation,
                            request,
                        }]
                    }
                    TurnSubmissionAdmission::RejectedActive { .. }
                    | TurnSubmissionAdmission::RejectedStopPending { .. }
                    | TurnSubmissionAdmission::RejectedUnavailable => Vec::new(),
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnSubmissionAdmissionResolved(admission)],
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::RequestStopAllSessions) => {
                self.conversation_turn.pause_post_turn_continuation();
                let admission = self.conversation_turn.admit_stop_request();
                let effects = match admission {
                    StopRequestAdmission::Accepted { correlation } => {
                        vec![CoreEffect::RequestStopAllSessions {
                            correlation,
                            attempt: StopRequestAttempt::Initial,
                        }]
                    }
                    StopRequestAdmission::RejectedActive { .. } => Vec::new(),
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::StopRequestAdmissionResolved(admission)],
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::SteerTurn(request)) => {
                let admission = self.conversation_turn.admit_turn_steer(&request);
                let effects = match admission {
                    TurnSteerAdmission::Accepted { correlation } => {
                        vec![CoreEffect::SteerTurn {
                            correlation,
                            request,
                        }]
                    }
                    TurnSteerAdmission::RejectedActive { .. }
                    | TurnSteerAdmission::RejectedUnavailable => Vec::new(),
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnSteerAdmissionResolved(admission)],
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::SubmitApprovalDecision {
                request_identity,
                decision,
            }) => {
                let admission = self
                    .conversation_turn
                    .admit_approval_decision(request_identity, decision);
                let effects = match &admission {
                    ApprovalDecisionAdmission::Accepted { correlation } => {
                        vec![CoreEffect::SubmitApprovalDecision {
                            correlation: correlation.clone(),
                        }]
                    }
                    ApprovalDecisionAdmission::RejectedActive { .. }
                    | ApprovalDecisionAdmission::RejectedUnavailable => Vec::new(),
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::ApprovalDecisionAdmissionResolved(admission)],
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::PersistConversationPreferences(request)) => {
                let effects = self
                    .conversation_preferences
                    .enqueue(*request)
                    .into_iter()
                    .map(|correlation| CoreEffect::PersistConversationPreferences { correlation })
                    .collect();
                CoreDispatchOutcome {
                    events: Vec::new(),
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::SetAutoFollowMaxTurns { value }) => {
                self.conversation_turn.set_auto_follow_max_turns(value);
                self.unchanged_outcome()
            }
            CoreInput::Command(AppCommand::PausePostTurnContinuation) => {
                self.conversation_turn.pause_post_turn_continuation();
                self.unchanged_outcome()
            }
            CoreInput::Command(AppCommand::SetParallelPostTurnRearm { rearmed }) => {
                self.conversation_turn.set_parallel_post_turn_rearm(rearmed);
                self.unchanged_outcome()
            }
            CoreInput::Command(AppCommand::ResolvePostTurnContinuation {
                correlation,
                resolution,
            }) => {
                let Some((execution, route_resolution)) = self
                    .conversation_turn
                    .resolve_post_turn_route(&correlation, resolution)
                else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::PostTurnEvaluationCompleted {
                        correlation,
                        execution,
                        route_resolution,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::SetupGithubReviewPolling(request)) => {
                let GithubReviewPollingSetupAdmission::Started { correlation } =
                    self.github_review.begin_setup(request.clone())
                else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::GithubReviewPollingSetupStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::SetupGithubReviewPolling {
                        correlation,
                        request,
                    }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::PollGithubReview) => {
                let Some(admission) = self.github_review.begin_poll() else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::GithubReviewPollStarted {
                        correlation: admission.correlation.clone(),
                    }],
                    effects: vec![CoreEffect::PollGithubReview {
                        setup_correlation: admission.setup_correlation,
                        correlation: admission.correlation,
                        previous_state: admission.previous_state,
                    }],
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::Command(AppCommand::EvaluatePostTurn(mut request)) => {
                match self
                    .conversation_turn
                    .admit_post_turn_evaluation(request.as_mut())
                {
                    Some(correlation) => {
                        let planning_worker_panel_state =
                            self.planning.begin_post_turn_worker_panel(request.as_ref());
                        request.planning_worker_panel_state = planning_worker_panel_state.clone();
                        CoreDispatchOutcome {
                            events: vec![AppEvent::PostTurnEvaluationStarted(
                                planning_worker_panel_state,
                            )],
                            effects: vec![CoreEffect::EvaluatePostTurn {
                                correlation,
                                request,
                            }],
                            snapshot: self.shared_snapshot(),
                        }
                    }
                    None => self.unchanged_outcome(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::StartupChecksLoaded {
                correlation,
                result,
            }) => {
                if !self.startup.accept(&correlation) {
                    return self.unchanged_outcome();
                }
                self.state.apply_startup_result(result);
                self.startup_changed_outcome(correlation, Vec::new())
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::SessionCatalogLoaded {
                correlation,
                result,
            }) => {
                if !self.session_feature.accept_catalog_completion(&correlation) {
                    return self.unchanged_outcome();
                }
                self.state.apply_session_catalog_result(result);
                self.session_catalog_changed_outcome(Vec::new())
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::SessionRenamed {
                correlation,
                result,
            }) => {
                if !self.session_feature.accept_rename_completion(&correlation) {
                    return self.unchanged_outcome();
                }
                let result = result.map(|()| {
                    self.state.apply_session_rename(&correlation.request);
                    SessionRenameAcceptedSnapshot {
                        session_catalog: self.shared_snapshot().session_catalog.clone(),
                        turn_stream: self
                            .conversation_turn
                            .reduce_session_rename_projection(&correlation)
                            .map(Box::new),
                    }
                });
                let mut events = vec![AppEvent::SessionRenameCompleted {
                    correlation,
                    result,
                }];
                let mut effects = Vec::new();
                self.start_deferred_session_reads(&mut events, &mut effects);
                CoreDispatchOutcome {
                    events,
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ConversationLoaded {
                correlation,
                mut result,
            }) => {
                if result.as_ref().is_ok_and(|ready| {
                    ready.conversation.thread_id != correlation.requested_thread_id
                }) {
                    result = Err("conversation provider returned a different thread".to_string());
                }
                let lifecycle_hydration = result.as_ref().ok().map(|ready| {
                    ConversationItemLifecycleProjection::from_snapshot_for_thread(
                        &ready.thread_id,
                        ready.conversation.item_lifecycle.clone(),
                    )
                });
                if lifecycle_hydration.as_ref().is_some_and(Result::is_err) {
                    result = Err(
                        "conversation provider returned an invalid item lifecycle projection"
                            .to_string(),
                    );
                }
                let loaded_stream_identity = match (result.as_ref().ok(), lifecycle_hydration) {
                    (Some(ready), Some(Ok(item_lifecycle))) => {
                        Some(LoadedConversationStreamIdentity {
                            thread_id: ready.thread_id.clone(),
                            title: ready.title.clone(),
                            workspace_directory: ready.workspace_directory.clone(),
                            item_lifecycle,
                        })
                    }
                    _ => None,
                };
                let Some(reduction) = self
                    .conversation_turn
                    .complete_conversation_load(&correlation, loaded_stream_identity)
                else {
                    return self.unchanged_outcome();
                };
                self.state.apply_conversation_result(result);
                self.conversation_changed_outcome(
                    Some(correlation),
                    stop_effects_from_intents(reduction.stop_effects),
                )
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation,
                result,
            }) => {
                if !self.read_models.accept_parallel_peek_load(&correlation) {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::ParallelPeekConversationLoaded {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ReviewCenterLoaded {
                correlation,
                snapshot,
            }) => {
                if !self.read_models.accept_review_center_load(&correlation) {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::ReviewCenterLoaded {
                        correlation,
                        snapshot,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::QueueAuthorityLoaded {
                correlation,
                result,
            }) => {
                if !self.read_models.accept_queue_authority_load(&correlation) {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::QueueAuthorityLoaded {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::DirectionsMaintenanceLoaded {
                correlation,
                result,
            }) => {
                if !self
                    .read_models
                    .accept_directions_maintenance_load(&correlation)
                {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::DirectionsMaintenanceLoaded {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation,
                result,
            }) => {
                if !self.planning.accept_runtime_refresh(&correlation) {
                    return self.unchanged_outcome();
                }
                let result = result.map(|snapshot| {
                    let snapshot = *snapshot;
                    self.state.apply_planning_runtime_projection(
                        correlation.workspace_directory.clone(),
                        snapshot.runtime_projection,
                    );
                    snapshot.doctor
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::PlanningRuntimeRefreshed {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation,
                result,
            }) => {
                if !matches!(
                    &correlation.operation,
                    PlanningWorkspaceOperationKind::Reset { .. }
                ) || !self.planning.accept_workspace_operation(&correlation)
                {
                    return self.unchanged_outcome();
                }
                let result = result.and_then(|snapshot| {
                    if correlation.reset_target() == Some(snapshot.target) {
                        Ok(snapshot)
                    } else {
                        Err(format!(
                            "planning workspace reset completion target mismatch: expected {}, received {}",
                            correlation
                                .reset_target()
                                .map(|target| target.label())
                                .unwrap_or("non-reset operation"),
                            snapshot.target.label(),
                        ))
                    }
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::PlanningWorkspaceResetCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PlanningSimpleDraftStaged {
                correlation,
                result,
            }) => {
                if !matches!(
                    &correlation.operation,
                    PlanningWorkspaceOperationKind::StageSimpleDraft
                ) || !self.planning.accept_workspace_operation(&correlation)
                {
                    return self.unchanged_outcome();
                }
                let result = result.and_then(|snapshot| {
                    let expected = correlation
                        .editor_session_identity(snapshot.session_identity.draft_name.clone());
                    if matches!(
                        &correlation.operation,
                        PlanningWorkspaceOperationKind::StageSimpleDraft
                    ) && !snapshot.session_identity.draft_name.is_empty()
                        && snapshot.session_identity == expected
                    {
                        Ok(snapshot)
                    } else {
                        Err("planning simple draft stage completion identity mismatch".to_string())
                    }
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::PlanningSimpleDraftStaged {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PlanningEditorStaged {
                correlation,
                result,
            }) => {
                let Some(expected_target) = correlation.editor_stage_target() else {
                    return self.unchanged_outcome();
                };
                if let Ok(snapshot) = &result {
                    let draft_name = snapshot.session.session_identity.draft_name.as_str();
                    let expected_session =
                        correlation.editor_session_identity(draft_name.to_string());
                    if draft_name.trim().is_empty()
                        || &snapshot.target != expected_target
                        || snapshot.session.session_identity != expected_session
                    {
                        return self.unchanged_outcome();
                    }
                }
                if !self.planning.accept_workspace_operation(&correlation) {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::PlanningEditorStaged {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PlanningEditorMutationCompleted {
                correlation,
                result,
            }) => {
                let Some(expected_identity) = correlation.editor_mutation_identity().cloned()
                else {
                    return self.unchanged_outcome();
                };
                if !self.planning.accept_workspace_operation(&correlation) {
                    return self.unchanged_outcome();
                }
                let result = result.and_then(|result| {
                    if result.identity() == &expected_identity
                        && result.action() == expected_identity.action
                        && !result.draft_name().trim().is_empty()
                        && result.draft_name() == expected_identity.draft_name
                    {
                        Ok(result)
                    } else {
                        Err("planning editor mutation completion identity mismatch".to_string())
                    }
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::PlanningEditorMutationCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PlanningSimpleEditorLoaded {
                correlation,
                result,
            }) => {
                if !matches!(
                    &correlation.operation,
                    PlanningWorkspaceOperationKind::LoadSimpleEditor { .. }
                ) || !self.planning.accept_workspace_operation(&correlation)
                {
                    return self.unchanged_outcome();
                }
                let result = result.and_then(|snapshot| {
                    let expected_draft = correlation.draft_name().unwrap_or_default();
                    let expected = correlation.editor_session_identity(expected_draft.to_string());
                    let source_matches = correlation.source_session().is_some_and(|source| {
                        source.workspace_directory == correlation.workspace_directory
                            && source.draft_name == expected_draft
                    });
                    if matches!(
                        &correlation.operation,
                        PlanningWorkspaceOperationKind::LoadSimpleEditor { .. }
                    ) && source_matches
                        && snapshot.session_identity == expected
                    {
                        Ok(snapshot)
                    } else {
                        Err("planning simple editor completion identity mismatch".to_string())
                    }
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::PlanningSimpleEditorLoaded {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PlanningSimpleDraftPromoted {
                correlation,
                result,
            }) => {
                if !matches!(
                    &correlation.operation,
                    PlanningWorkspaceOperationKind::PromoteSimpleDraft { .. }
                ) || !self.planning.accept_workspace_operation(&correlation)
                {
                    return self.unchanged_outcome();
                }
                let result = result.and_then(|snapshot| {
                    let expected_draft = correlation.draft_name().unwrap_or_default();
                    let source_matches = correlation.source_session().is_some_and(|source| {
                        source.workspace_directory == correlation.workspace_directory
                            && source.draft_name == expected_draft
                    });
                    if matches!(
                        &correlation.operation,
                        PlanningWorkspaceOperationKind::PromoteSimpleDraft { .. }
                    ) && source_matches
                        && snapshot.draft_name == expected_draft
                    {
                        Ok(snapshot)
                    } else {
                        Err(
                            "planning simple draft promotion completion identity mismatch"
                                .to_string(),
                        )
                    }
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::PlanningSimpleDraftPromoted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::QueueMutationCompleted {
                correlation,
                result,
            }) => {
                if !self.planning.complete_queue_mutation(&correlation) {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::QueueMutationCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt,
                result,
            }) => {
                let Some(reduction) = self.conversation_turn.complete_stop_request(
                    correlation,
                    attempt,
                    result.is_err(),
                ) else {
                    return self.unchanged_outcome();
                };
                let mut effects = stop_effects_from_intents(reduction.stop_effects);
                let mut events = reduction
                    .publish_completion
                    .then_some(AppEvent::StopRequestAttemptCompleted {
                        correlation,
                        attempt,
                        result,
                    })
                    .into_iter()
                    .collect::<Vec<_>>();
                if reduction.settlement_finished {
                    self.start_deferred_session_reads(&mut events, &mut effects);
                }
                CoreDispatchOutcome {
                    events,
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::TurnSteered {
                correlation,
                result,
            }) => {
                let Some(result) = self
                    .conversation_turn
                    .complete_turn_steer(correlation, result)
                else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnSteerCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation,
                result,
            }) => {
                if !self
                    .conversation_turn
                    .complete_approval_decision(&correlation, result.is_ok())
                {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::ApprovalDecisionSubmissionCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ApprovalReviewPersisted {
                correlation,
                result,
            }) => {
                let Some(settlement) = self
                    .conversation_turn
                    .complete_approval_review_persistence(&correlation)
                else {
                    return self.unchanged_outcome();
                };
                let mut events = Vec::new();
                if let Err(error) = result
                    && settlement.completion_matches_current_turn
                    && self.conversation_turn.matches_conversation(
                        &correlation.workspace_directory,
                        &correlation.thread_id,
                    )
                {
                    events.push(AppEvent::turn_stream_snapshot_changed(
                        self.conversation_turn.apply_runtime_notice(format!(
                            "review-center persistence failed: {error}"
                        )),
                    ));
                }
                let effects = settlement
                    .next
                    .into_iter()
                    .map(|correlation| CoreEffect::PersistApprovalReview { correlation })
                    .collect();
                CoreDispatchOutcome {
                    events,
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(
                CoreEffectCompletion::ConversationPreferencesPersisted {
                    correlation,
                    result,
                },
            ) => {
                let Some(settlement) = self.conversation_preferences.complete(&correlation) else {
                    return self.unchanged_outcome();
                };
                let effects = settlement
                    .next
                    .into_iter()
                    .map(|correlation| CoreEffect::PersistConversationPreferences { correlation })
                    .collect();
                CoreDispatchOutcome {
                    events: vec![AppEvent::ConversationPreferencesPersisted {
                        correlation,
                        result,
                    }],
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(
                CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                    correlation,
                    result,
                },
            ) => {
                let Some(result) = self.github_review.complete_setup(&correlation, result) else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::GithubReviewPollingSetupCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::GithubReviewPollCompleted {
                correlation,
                result,
            }) => {
                let Some(result) = self.github_review.complete_poll(&correlation, result) else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::GithubReviewPollCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ManualPromptPrepared(result)) => {
                let Some(disposition) = self
                    .planning
                    .complete_manual_prompt_preparation(result.correlation())
                else {
                    return CoreDispatchOutcome {
                        events: Vec::new(),
                        effects: Vec::new(),
                        snapshot: self.shared_snapshot(),
                    };
                };
                let snapshot = self.shared_snapshot();
                CoreDispatchOutcome {
                    events: (disposition == ManualPromptCompletionDisposition::Publish)
                        .then_some(AppEvent::ManualPromptPrepared(result))
                        .into_iter()
                        .collect(),
                    effects: Vec::new(),
                    snapshot,
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation,
                execution,
            }) => {
                if !self
                    .conversation_turn
                    .complete_post_turn_evaluation(&correlation, execution.as_ref())
                {
                    return self.unchanged_outcome();
                }
                self.planning
                    .accept_post_turn_worker_panel(execution.as_ref());
                let workspace_directory = execution.runtime_projection_workspace_directory.clone();
                let refresh_matches_workspace = self
                    .planning
                    .runtime_refresh_matches_workspace(&workspace_directory);
                let should_apply_projection = if self.planning.runtime_refresh_has_active() {
                    refresh_matches_workspace
                } else {
                    self.state
                        .planning_runtime_workspace_directory()
                        .is_none_or(|current| current == workspace_directory)
                };
                if should_apply_projection {
                    self.state.apply_planning_runtime_projection(
                        workspace_directory.clone(),
                        Box::new(execution.evaluation.runtime_projection.clone()),
                    );
                }
                let (mut refresh_events, effects) =
                    self.restart_planning_runtime_refresh_after_writer(&workspace_directory);
                let mut events = vec![AppEvent::PostTurnContinuationRoutingRequested {
                    correlation,
                    execution,
                }];
                events.append(&mut refresh_events);
                CoreDispatchOutcome {
                    events,
                    effects,
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::ConversationStreamUpdated { correlation, event } => {
                self.reduce_correlated_turn_stream_event(correlation, event)
            }
            CoreInput::ConversationRuntimeNotice(notice) => {
                let stream_snapshot = self.conversation_turn.apply_runtime_notice(notice);
                CoreDispatchOutcome {
                    events: vec![AppEvent::turn_stream_snapshot_changed(stream_snapshot)],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::ConversationTurnRuntimeNotice {
                correlation,
                notice,
            } => {
                let Some(stream_snapshot) = self
                    .conversation_turn
                    .apply_correlated_runtime_notice(correlation, notice)
                else {
                    return self.unchanged_outcome();
                };
                CoreDispatchOutcome {
                    events: vec![AppEvent::turn_stream_snapshot_changed(stream_snapshot)],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::ConversationTurnWorkspaceChanged {
                correlation,
                workspace_directory,
            } => {
                if !self
                    .conversation_turn
                    .apply_turn_workspace_change(correlation, workspace_directory.clone())
                {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::ConversationTurnWorkspaceChanged {
                        workspace_directory,
                    }],
                    effects: Vec::new(),
                    snapshot: self.shared_snapshot(),
                }
            }
            CoreInput::ParallelModeSupervisorSnapshotInvalidated => CoreDispatchOutcome {
                events: vec![AppEvent::ParallelModeSupervisorSnapshotInvalidated],
                effects: Vec::new(),
                snapshot: self.shared_snapshot(),
            },
            CoreInput::RuntimeProjectionChanged {
                workspace_directory,
                projection,
            } => {
                let refresh_matches_workspace = self
                    .planning
                    .runtime_refresh_matches_workspace(&workspace_directory);
                if self.planning.runtime_refresh_has_active() && !refresh_matches_workspace {
                    return self.unchanged_outcome();
                }
                let changed = self
                    .state
                    .apply_planning_runtime_projection(workspace_directory.clone(), projection);
                let (refresh_events, refresh_effects) =
                    self.restart_planning_runtime_refresh_after_writer(&workspace_directory);
                let mut outcome = self.snapshot_changed_outcome(changed);
                outcome.events.extend(refresh_events);
                outcome.effects.extend(refresh_effects);
                outcome
            }
            CoreInput::ParallelModeReadinessProjectionChanged(snapshot) => {
                let changed = self.state.apply_parallel_readiness_projection(snapshot);
                self.snapshot_changed_outcome(changed)
            }
            CoreInput::ParallelModeSupervisorProjectionChanged(snapshot) => {
                let changed = self.state.apply_parallel_supervisor_projection(snapshot);
                self.snapshot_changed_outcome(changed)
            }
        }
    }

    fn restart_planning_runtime_refresh_after_writer(
        &mut self,
        workspace_directory: &str,
    ) -> (Vec<AppEvent>, Vec<CoreEffect>) {
        let Some((replacement, superseded)) = self
            .planning
            .restart_runtime_refresh_if_matches(workspace_directory)
        else {
            return (Vec::new(), Vec::new());
        };
        (
            vec![
                AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: replacement.clone(),
                },
                AppEvent::PlanningRuntimeRefreshCancelled {
                    correlation: superseded,
                },
            ],
            vec![CoreEffect::LoadPlanningRuntime {
                correlation: replacement,
            }],
        )
    }

    fn admit_session_catalog_load(
        &mut self,
        intent: SessionCatalogLoadIntent,
    ) -> CoreDispatchOutcome {
        let reduction = {
            let snapshot = self.shared_snapshot();
            self.session_feature
                .reduce_catalog_load(intent, &snapshot.session_catalog)
        };
        match reduction {
            SessionCatalogLoadReduction::Unchanged | SessionCatalogLoadReduction::Deferred => {
                self.unchanged_outcome()
            }
            SessionCatalogLoadReduction::Started { correlation } => {
                self.state.mark_session_catalog_loading();
                self.session_catalog_changed_outcome(vec![CoreEffect::LoadSessionCatalog {
                    correlation,
                }])
            }
        }
    }

    fn begin_planning_workspace_operation(
        &mut self,
        intent: PlanningWorkspaceOperationIntent,
    ) -> CoreDispatchOutcome {
        if intent
            .operation
            .source_session()
            .is_some_and(|source_session| {
                source_session.workspace_directory != intent.workspace_directory
                    || intent.operation.draft_name() != Some(source_session.draft_name.as_str())
            })
        {
            return self.unchanged_outcome();
        }
        let admission = self.planning.begin_workspace_operation(intent);
        let effects = match &admission {
            PlanningWorkspaceOperationAdmission::Started { correlation } => {
                let effect = match &correlation.operation {
                    PlanningWorkspaceOperationKind::Reset { .. } => {
                        CoreEffect::ResetPlanningWorkspace {
                            correlation: correlation.clone(),
                        }
                    }
                    PlanningWorkspaceOperationKind::StageSimpleDraft => {
                        CoreEffect::StageSimplePlanningDraft {
                            correlation: correlation.clone(),
                        }
                    }
                    PlanningWorkspaceOperationKind::StageEditor { .. } => {
                        CoreEffect::StagePlanningEditor {
                            correlation: correlation.clone(),
                        }
                    }
                    PlanningWorkspaceOperationKind::MutateEditor { .. } => {
                        unreachable!("editor mutations carry a redacted effect payload")
                    }
                    PlanningWorkspaceOperationKind::LoadSimpleEditor { .. } => {
                        CoreEffect::LoadSimplePlanningEditor {
                            correlation: correlation.clone(),
                        }
                    }
                    PlanningWorkspaceOperationKind::PromoteSimpleDraft { .. } => {
                        CoreEffect::PromoteSimplePlanningDraft {
                            correlation: correlation.clone(),
                        }
                    }
                };
                vec![effect]
            }
            PlanningWorkspaceOperationAdmission::Coalesced { .. }
            | PlanningWorkspaceOperationAdmission::Busy { .. } => Vec::new(),
        };
        CoreDispatchOutcome {
            events: vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                admission,
            )],
            effects,
            snapshot: self.shared_snapshot(),
        }
    }

    fn begin_planning_editor_mutation(
        &mut self,
        workspace_directory: String,
        request: Box<PlanningEditorMutationRequest>,
    ) -> CoreDispatchOutcome {
        let identity = &request.identity;
        if identity.draft_name.trim().is_empty()
            || identity.source_session.workspace_directory != workspace_directory
            || identity.source_session.draft_name != identity.draft_name
        {
            return self.unchanged_outcome();
        }
        let admission = self.planning.begin_workspace_operation(
            PlanningWorkspaceOperationIntent::mutate_editor(workspace_directory, identity.clone()),
        );
        let effects = match &admission {
            PlanningWorkspaceOperationAdmission::Started { correlation } => {
                vec![CoreEffect::MutatePlanningEditor {
                    correlation: correlation.clone(),
                    request,
                }]
            }
            PlanningWorkspaceOperationAdmission::Coalesced { .. }
            | PlanningWorkspaceOperationAdmission::Busy { .. } => Vec::new(),
        };
        CoreDispatchOutcome {
            events: vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                admission,
            )],
            effects,
            snapshot: self.shared_snapshot(),
        }
    }

    fn start_conversation_load(
        &mut self,
        thread_id: String,
        fallback_workspace_directory: String,
    ) -> CoreDispatchOutcome {
        self.planning.reset_worker_panel_history();
        let cancelled_refresh = self.planning.cancel_runtime_refresh();
        let blocked_by_session_rename = self
            .session_feature
            .active_rename_matches_thread(&thread_id);
        let admission = self.conversation_turn.admit_conversation_load(
            thread_id,
            fallback_workspace_directory,
            blocked_by_session_rename,
        );
        let mut outcome = match admission {
            ConversationLoadAdmission::Deferred { stop_effects } => {
                let mut outcome = self.unchanged_outcome();
                outcome.effects = stop_effects_from_intents(stop_effects);
                outcome
            }
            ConversationLoadAdmission::Started {
                correlation,
                fallback_workspace_directory,
                stop_effects,
            } => {
                self.state.mark_conversation_loading();
                let mut effects = stop_effects_from_intents(stop_effects);
                effects.push(CoreEffect::LoadConversation {
                    correlation: correlation.clone(),
                    fallback_workspace_directory,
                });
                self.conversation_changed_outcome(Some(correlation), effects)
            }
        };
        if let Some(correlation) = cancelled_refresh {
            outcome
                .events
                .insert(0, AppEvent::PlanningRuntimeRefreshCancelled { correlation });
        }
        outcome
    }

    fn start_deferred_session_reads(
        &mut self,
        events: &mut Vec<AppEvent>,
        effects: &mut Vec<CoreEffect>,
    ) {
        if let Some(intent) = self.session_feature.take_deferred_catalog_load() {
            let outcome = self.admit_session_catalog_load(intent);
            events.extend(outcome.events);
            effects.extend(outcome.effects);
        }
        if !self.session_feature.has_active_rename()
            && !self.conversation_turn.stop_request_settlement_pending()
            && let Some((thread_id, fallback_workspace_directory)) =
                self.conversation_turn.take_deferred_conversation_load()
        {
            let outcome = self.start_conversation_load(thread_id, fallback_workspace_directory);
            events.extend(outcome.events);
            effects.extend(outcome.effects);
        }
    }

    fn snapshot_changed_outcome(&self, changed: bool) -> CoreDispatchOutcome {
        let snapshot = self.shared_snapshot();
        CoreDispatchOutcome {
            events: if changed {
                vec![AppEvent::SnapshotChanged(snapshot.clone())]
            } else {
                Vec::new()
            },
            effects: Vec::new(),
            snapshot,
        }
    }

    #[cfg(test)]
    fn active_post_turn_evaluation_correlation(&self) -> Option<&PostTurnEvaluationCorrelation> {
        self.conversation_turn
            .active_post_turn_evaluation_correlation()
    }

    fn reduce_correlated_turn_stream_event(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        event: TurnStreamEvent,
    ) -> CoreDispatchOutcome {
        let Some(reduction) = self
            .conversation_turn
            .apply_correlated_turn_stream_event(correlation, event)
        else {
            return self.unchanged_outcome();
        };
        let events = reduction
            .snapshots
            .into_iter()
            .map(AppEvent::turn_stream_snapshot_changed)
            .collect();
        let mut effects = stop_effects_from_intents(reduction.stop_effects);
        if let Some(correlation) = reduction.approval_review_persistence {
            effects.push(CoreEffect::PersistApprovalReview { correlation });
        }
        CoreDispatchOutcome {
            events,
            effects,
            snapshot: self.shared_snapshot(),
        }
    }

    #[cfg(test)]
    pub(crate) fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        let correlation = self.conversation_turn.begin_test_turn_submission();
        self.state
            .apply_conversation_runtime_snapshot(self.conversation_turn.runtime_snapshot());
        correlation
    }

    #[cfg(test)]
    pub(crate) fn begin_test_post_turn_evaluation(
        &mut self,
        thread_id: &str,
        completed_turn_id: &str,
        turn_workspace_directory: &str,
        planning_workspace_directory: &str,
    ) -> PostTurnEvaluationCorrelation {
        let correlation = self.conversation_turn.begin_test_post_turn_evaluation(
            thread_id,
            completed_turn_id,
            turn_workspace_directory,
            planning_workspace_directory,
        );
        self.state
            .apply_conversation_runtime_snapshot(self.conversation_turn.runtime_snapshot());
        correlation
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn test_post_turn_evaluation_is_in_flight(
        &self,
        thread_id: &str,
        completed_turn_id: &str,
    ) -> bool {
        self.conversation_turn
            .active_post_turn_evaluation_correlation()
            .is_some_and(|active| {
                active.thread_id == thread_id && active.completed_turn_id == completed_turn_id
            })
    }

    fn unchanged_outcome(&self) -> CoreDispatchOutcome {
        CoreDispatchOutcome {
            events: Vec::new(),
            effects: Vec::new(),
            snapshot: self.shared_snapshot(),
        }
    }

    fn with_conversation_runtime_authority(
        &mut self,
        mut outcome: CoreDispatchOutcome,
    ) -> CoreDispatchOutcome {
        let runtime = self.conversation_turn.runtime_snapshot();
        if self
            .state
            .apply_conversation_runtime_snapshot(runtime.clone())
        {
            outcome.events.insert(
                0,
                AppEvent::ConversationRuntimeAuthorityChanged(Box::new(runtime)),
            );
            outcome.snapshot = self.shared_snapshot();
        }
        outcome
    }

    fn startup_changed_outcome(
        &self,
        correlation: StartupCheckCorrelation,
        effects: Vec<CoreEffect>,
    ) -> CoreDispatchOutcome {
        let snapshot = self.shared_snapshot();
        CoreDispatchOutcome {
            events: vec![AppEvent::StartupChanged {
                correlation,
                snapshot: snapshot.startup.clone(),
            }],
            effects,
            snapshot,
        }
    }

    fn session_catalog_changed_outcome(&self, effects: Vec<CoreEffect>) -> CoreDispatchOutcome {
        let snapshot = self.shared_snapshot();
        CoreDispatchOutcome {
            events: vec![AppEvent::SessionCatalogChanged(
                snapshot.session_catalog.clone(),
            )],
            effects,
            snapshot,
        }
    }

    fn conversation_changed_outcome(
        &self,
        correlation: Option<ConversationLoadCorrelation>,
        effects: Vec<CoreEffect>,
    ) -> CoreDispatchOutcome {
        let snapshot = self.shared_snapshot();
        CoreDispatchOutcome {
            events: vec![AppEvent::ConversationChanged {
                correlation,
                snapshot: snapshot.conversation.clone(),
            }],
            effects,
            snapshot,
        }
    }
}

fn stop_effects_from_intents(intents: Vec<StopEffectIntent>) -> Vec<CoreEffect> {
    intents
        .into_iter()
        .map(|intent| match intent {
            StopEffectIntent::Request {
                correlation,
                attempt,
            } => CoreEffect::RequestStopAllSessions {
                correlation,
                attempt,
            },
            StopEffectIntent::Invalidate { correlation } => {
                CoreEffect::InvalidateStopRequest { correlation }
            }
        })
        .collect()
}

impl Default for CoreController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::service::planning::PlanningRuntimeProjection;
    use crate::core::app::{
        ApprovalAuthorityPhase, ApprovalReviewPersistenceCorrelation, AutoFollowPhase,
        ConversationPreferencePersistenceRequest, ConversationPreferenceThreadTarget,
        ConversationReadySnapshot, ConversationSnapshot, CorePromptOrigin,
        DirectionsMaintenanceDirectionSnapshot, DirectionsMaintenanceLoadCorrelation,
        DirectionsMaintenanceSummarySnapshot, DirectionsSupportingFileStatus,
        GithubReviewPollCorrelation, GithubReviewPollingSetupCorrelation,
        GithubReviewPollingSetupMode, GithubReviewPollingSetupRequest,
        GithubReviewPollingSetupResult, ManualPromptPreparationAdmission,
        ManualPromptPreparationIntent, ParallelPeekLoadCorrelation, PlanningDoctorSnapshot,
        PlanningEditorFileSnapshot, PlanningEditorMutationAction, PlanningEditorMutationIdentity,
        PlanningEditorMutationRequest, PlanningEditorMutationResult, PlanningEditorMutationTarget,
        PlanningEditorSessionIdentity, PlanningEditorSessionSnapshot, PlanningEditorStageSnapshot,
        PlanningEditorStageTarget, PlanningRuntimeRefreshCorrelation,
        PlanningRuntimeRefreshSnapshot, PlanningSimpleDraftPromotionSnapshot,
        PlanningSimpleDraftStageSnapshot, PlanningWorkspaceOperationCorrelation,
        PlanningWorkspaceResetIntent, PlanningWorkspaceResetSnapshot, PlanningWorkspaceResetTarget,
        QueueAuthorityLoadCorrelation, QueueAuthorityLoadError, QueueAuthoritySnapshot,
        QueueMutationCommitSnapshot, QueueMutationCorrelation, QueueMutationIntent,
        QueueMutationKind, QueueMutationResult, QueueMutationTarget, ReviewCenterLoadCorrelation,
        ReviewCenterSnapshot, SessionCatalogLoadCorrelation, SessionCatalogReadySnapshot,
        SessionCatalogSnapshot, TurnSubmissionRequest,
    };
    use crate::core::app::{
        PostTurnAuthoritySnapshot, PostTurnRouteResolution, StartupAttachmentSnapshot,
        StartupDiagnosticSnapshot, StartupReadySnapshot, StartupSnapshot, TurnStreamEvent,
        TurnStreamSnapshot, TurnStreamTerminalSnapshot, TurnStreamUpdate,
    };
    use crate::domain::conversation::{
        ConversationApprovalDecision, ConversationApprovalRequest,
        ConversationApprovalRequestIdentity, ConversationApprovalRequestKind,
        ConversationApprovalResolution, ConversationApprovalReview,
        ConversationApprovalReviewStatus, ConversationMessage, ConversationMessageKind,
        ConversationReasoningEffort, ConversationSnapshot as DomainConversationSnapshot,
        ConversationTurnOptions, ConversationTurnSteerRequest,
    };
    use crate::domain::conversation_item_lifecycle::{
        ConversationItemKind, ConversationItemLifecycleConsistency,
        ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
        ConversationItemLifecycleSource, ConversationItemOutcome,
    };
    use crate::domain::github_review::{
        GithubPullRequestActivitySnapshot, GithubPullRequestPollResult, GithubPullRequestPollState,
        GithubPullRequestTarget,
    };
    use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeReadinessState};
    use crate::domain::planning::{
        ManualPromptCorrelation, ManualPromptOutcome, ManualPromptRequest,
        PlanningWorkerPanelState, PlanningWorkerStatus, PostTurnContext, PostTurnContinuationGate,
        PostTurnContinuationPermit, PostTurnRequest, QueueIdlePolicy, RESULT_OUTPUT_FILE_PATH,
        RuntimeProjection, TaskStatus, TurnSnapshotCapture,
    };
    use crate::domain::recent_sessions::{RecentSessions, SessionRenameRequest};
    use crate::domain::session_summary::SessionSummary;
    use crate::domain::turn_terminal::ConversationTurnError;

    fn manual_prompt_intent(
        workspace_directory: &str,
        raw_prompt: &str,
    ) -> ManualPromptPreparationIntent {
        ManualPromptPreparationIntent {
            workspace_directory: workspace_directory.to_string(),
            raw_prompt: raw_prompt.to_string(),
            parent_thread_id: None,
            parent_turn_id: None,
        }
    }

    fn manual_prompt_correlation(
        generation: u64,
        workspace_directory: &str,
    ) -> ManualPromptCorrelation {
        ManualPromptCorrelation {
            request_id: generation,
            generation,
            workspace_directory: workspace_directory.to_string(),
        }
    }

    fn startup_check_correlation(generation: u64) -> StartupCheckCorrelation {
        StartupCheckCorrelation::new(generation, "/tmp/workspace")
    }

    fn startup_check_correlation_for(
        generation: u64,
        workspace_directory: &str,
    ) -> StartupCheckCorrelation {
        StartupCheckCorrelation::new(generation, workspace_directory)
    }

    fn run_startup_checks_command(workspace_directory: &str) -> CoreInput {
        CoreInput::Command(AppCommand::RunStartupChecks {
            workspace_directory: workspace_directory.to_string(),
        })
    }

    fn ensure_session_catalog_command(limit: usize, workspace_directory: &str) -> CoreInput {
        CoreInput::Command(AppCommand::LoadSessionCatalog(
            SessionCatalogLoadIntent::ensure_loaded(limit, workspace_directory),
        ))
    }

    fn refresh_session_catalog_command(limit: usize, workspace_directory: &str) -> CoreInput {
        CoreInput::Command(AppCommand::LoadSessionCatalog(
            SessionCatalogLoadIntent::refresh(limit, workspace_directory),
        ))
    }

    fn session_catalog_correlation(
        generation: u64,
        limit: usize,
        workspace_directory: &str,
    ) -> SessionCatalogLoadCorrelation {
        SessionCatalogLoadCorrelation::new(generation, limit, workspace_directory)
    }

    fn conversation_load_correlation(
        generation: u64,
        thread_id: &str,
    ) -> ConversationLoadCorrelation {
        ConversationLoadCorrelation::new(generation, thread_id)
    }

    fn parallel_peek_load_correlation(
        generation: u64,
        thread_id: &str,
    ) -> ParallelPeekLoadCorrelation {
        ParallelPeekLoadCorrelation::new(generation, thread_id)
    }

    fn review_center_load_correlation(
        generation: u64,
        workspace_directory: &str,
        active_thread_id: Option<&str>,
    ) -> ReviewCenterLoadCorrelation {
        ReviewCenterLoadCorrelation::new(
            generation,
            workspace_directory,
            active_thread_id.map(str::to_string),
        )
    }

    fn empty_review_center_snapshot() -> ReviewCenterSnapshot {
        ReviewCenterSnapshot {
            current_thread_reviews: Ok(Vec::new()),
            pending_inbox: Ok(Vec::new()),
            recent_history: Ok(Vec::new()),
        }
    }

    fn queue_authority_load_correlation(
        generation: u64,
        workspace_directory: &str,
        active_thread_id: Option<&str>,
    ) -> QueueAuthorityLoadCorrelation {
        QueueAuthorityLoadCorrelation::new(
            generation,
            workspace_directory,
            active_thread_id.map(str::to_string),
        )
    }

    fn directions_maintenance_load_correlation(
        generation: u64,
        workspace_directory: &str,
    ) -> DirectionsMaintenanceLoadCorrelation {
        DirectionsMaintenanceLoadCorrelation::new(generation, workspace_directory)
    }

    fn directions_maintenance_summary() -> Box<DirectionsMaintenanceSummarySnapshot> {
        Box::new(DirectionsMaintenanceSummarySnapshot {
            directions: vec![DirectionsMaintenanceDirectionSnapshot {
                id: "direction-1".to_string(),
                title: "Direction 1".to_string(),
                detail_doc_path: Some("docs/direction-1.md".to_string()),
                detail_doc_status: DirectionsSupportingFileStatus::Ready,
            }],
            missing_detail_doc_count: 0,
            broken_detail_doc_count: 0,
            queue_idle_policy: QueueIdlePolicy::ReviewAndEnqueue,
            queue_idle_prompt_path: Some("prompts/review.md".to_string()),
            queue_idle_prompt_status: DirectionsSupportingFileStatus::Ready,
            parse_error: None,
        })
    }

    fn planning_runtime_refresh_correlation(
        generation: u64,
        workspace_directory: &str,
    ) -> PlanningRuntimeRefreshCorrelation {
        PlanningRuntimeRefreshCorrelation::new(generation, workspace_directory)
    }

    fn planning_runtime_refresh_snapshot(
        projection: PlanningRuntimeProjection,
    ) -> Box<PlanningRuntimeRefreshSnapshot> {
        Box::new(PlanningRuntimeRefreshSnapshot::new(projection))
    }

    fn planning_doctor_snapshot(projection: &PlanningRuntimeProjection) -> PlanningDoctorSnapshot {
        PlanningDoctorSnapshot::from_runtime_projection(projection)
    }

    fn runtime_projection_changed(
        workspace_directory: &str,
        projection: PlanningRuntimeProjection,
    ) -> CoreInput {
        CoreInput::RuntimeProjectionChanged {
            workspace_directory: workspace_directory.to_string(),
            projection: Box::new(projection),
        }
    }

    fn post_turn_completion(
        workspace_directory: &str,
        execution: Box<
            crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution,
        >,
    ) -> CoreEffectCompletion {
        let correlation = PostTurnEvaluationCorrelation::new(
            1,
            execution.thread_id.clone(),
            execution.completed_turn_id.clone(),
            workspace_directory,
            workspace_directory,
        );
        post_turn_completion_for(correlation, workspace_directory, execution)
    }

    fn post_turn_completion_for(
        correlation: PostTurnEvaluationCorrelation,
        workspace_directory: &str,
        mut execution: Box<
            crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution,
        >,
    ) -> CoreEffectCompletion {
        execution.runtime_projection_workspace_directory = workspace_directory.to_string();
        CoreEffectCompletion::PostTurnEvaluationCompleted {
            correlation,
            execution,
        }
    }

    fn empty_queue_authority_snapshot() -> QueueAuthoritySnapshot {
        QueueAuthoritySnapshot {
            runtime_projection: RuntimeProjection::uninitialized(),
            planning_revision: 0,
            tasks: Vec::new(),
        }
    }

    fn queue_mutation_intent(
        workspace_directory: &str,
        active_thread_id: Option<&str>,
        kind: QueueMutationKind,
    ) -> QueueMutationIntent {
        QueueMutationIntent {
            workspace_directory: workspace_directory.to_string(),
            active_thread_id: active_thread_id.map(str::to_string),
            kind,
            expected_planning_revision: 7,
            targets: vec![QueueMutationTarget {
                task_id: "task-1".to_string(),
                expected_status: TaskStatus::Ready,
                expected_updated_at: "2026-07-19T00:00:00Z".to_string(),
            }],
            receipt_at_start: None,
        }
    }

    fn successful_queue_mutation_result() -> Box<QueueMutationResult> {
        Box::new(QueueMutationResult {
            mutation: Ok(QueueMutationCommitSnapshot {
                committed_planning_revision: 8,
                committed_task_ids: vec!["task-1".to_string()],
            }),
            authority: Ok(empty_queue_authority_snapshot()),
        })
    }

    fn session_rename_correlation(
        generation: u64,
        thread_id: &str,
        name: &str,
    ) -> SessionRenameCorrelation {
        SessionRenameCorrelation::new(generation, SessionRenameRequest::new(thread_id, name))
    }

    fn assert_wrong_planning_completion_kind_keeps_active_lease(
        command: AppCommand,
        wrong_completion: impl FnOnce(PlanningWorkspaceOperationCorrelation) -> CoreEffectCompletion,
        exact_completion: impl FnOnce(PlanningWorkspaceOperationCorrelation) -> CoreEffectCompletion,
    ) {
        let mut controller = CoreController::new();
        let started = controller.handle_input(CoreInput::Command(command));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("planning workspace operation should start");
        };
        let correlation = correlation.clone();

        let rejected = controller.handle_input(CoreInput::EffectCompleted(wrong_completion(
            correlation.clone(),
        )));
        assert!(rejected.events.is_empty());
        assert!(rejected.effects.is_empty());

        let busy = controller.handle_input(CoreInput::Command(AppCommand::ResetPlanningWorkspace(
            PlanningWorkspaceResetIntent::new("/workspace", PlanningWorkspaceResetTarget::Queue),
        )));
        assert!(matches!(
            busy.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation,
                    ..
                }
            )] if active_correlation == &correlation
        ));
        assert!(busy.effects.is_empty());

        let settled =
            controller.handle_input(CoreInput::EffectCompleted(exact_completion(correlation)));
        assert_eq!(settled.events.len(), 1);
        assert!(settled.effects.is_empty());

        let restarted = controller.handle_input(CoreInput::Command(
            AppCommand::ResetPlanningWorkspace(PlanningWorkspaceResetIntent::new(
                "/workspace",
                PlanningWorkspaceResetTarget::Queue,
            )),
        ));
        assert!(matches!(
            restarted.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation == 2
        ));
    }

    #[test]
    fn new_controller_exposes_initial_snapshot() {
        let controller = CoreController::new();
        let default_controller = CoreController::default();

        assert_eq!(controller.snapshot(), AppSnapshot::initial());
        assert_eq!(default_controller.snapshot(), AppSnapshot::initial());
    }

    #[test]
    fn noop_command_keeps_initial_state_without_events() {
        let mut controller = CoreController::new();

        let first = controller.handle_input(CoreInput::Command(AppCommand::Noop));
        let second = controller.handle_input(CoreInput::Command(AppCommand::Noop));

        assert!(first.events.is_empty());
        assert!(first.effects.is_empty());
        assert!(second.events.is_empty());
        assert!(second.effects.is_empty());
        assert!(Arc::ptr_eq(&first.snapshot, &second.snapshot));
        assert_eq!(*second.snapshot, AppSnapshot::initial());
        assert_eq!(controller.snapshot(), AppSnapshot::initial());
    }

    #[test]
    fn planning_workspace_reset_core_coalesces_busy_and_rejects_stale_aba_completions() {
        let mut controller = CoreController::new();
        let intent =
            PlanningWorkspaceResetIntent::new("/workspace", PlanningWorkspaceResetTarget::Queue);
        let started = controller.handle_input(CoreInput::Command(
            AppCommand::ResetPlanningWorkspace(intent.clone()),
        ));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("first reset should start");
        };
        let first = correlation.clone();
        assert_eq!(first.generation, 1);
        assert_eq!(
            started.effects,
            vec![CoreEffect::ResetPlanningWorkspace {
                correlation: first.clone(),
            }]
        );

        let duplicate = controller.handle_input(CoreInput::Command(
            AppCommand::ResetPlanningWorkspace(intent.clone()),
        ));
        assert_eq!(
            duplicate.events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: first.clone(),
                },
            )]
        );
        assert!(duplicate.effects.is_empty());

        let requested_reset =
            PlanningWorkspaceResetIntent::new("/workspace", PlanningWorkspaceResetTarget::All);
        let busy = controller.handle_input(CoreInput::Command(AppCommand::ResetPlanningWorkspace(
            requested_reset.clone(),
        )));
        let requested = PlanningWorkspaceOperationIntent::reset(requested_reset);
        assert_eq!(
            busy.events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation: first.clone(),
                    requested,
                },
            )]
        );
        assert!(busy.effects.is_empty());

        let mut stale = first.clone();
        stale.generation = 99;
        let stale_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation: stale,
                result: Err("stale".to_string()),
            },
        ));
        assert!(stale_completion.events.is_empty());

        let result = Box::new(PlanningWorkspaceResetSnapshot {
            target: PlanningWorkspaceResetTarget::Queue,
            rewritten_paths: vec!["planning/task-authority.json".to_string()],
            removed_paths: Vec::new(),
        });
        let completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation: first.clone(),
                result: Ok(result.clone()),
            },
        ));
        assert_eq!(
            completed.events,
            vec![AppEvent::PlanningWorkspaceResetCompleted {
                correlation: first.clone(),
                result: Ok(result),
            }]
        );

        let duplicate_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation: first.clone(),
                result: Err("duplicate".to_string()),
            },
        ));
        assert!(duplicate_completion.events.is_empty());

        let restarted = controller.handle_input(CoreInput::Command(
            AppCommand::ResetPlanningWorkspace(intent),
        ));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started {
                    correlation: second,
                },
            ),
        ] = restarted.events.as_slice()
        else {
            panic!("reset should restart after exact settlement");
        };
        assert_eq!(second.generation, 2);
        let aba_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation: first,
                result: Err("aba".to_string()),
            },
        ));
        assert!(aba_completion.events.is_empty());
    }

    #[test]
    fn planning_workspace_reset_core_rejects_a_mismatched_success_snapshot() {
        let mut controller = CoreController::new();
        let started = controller.handle_input(CoreInput::Command(
            AppCommand::ResetPlanningWorkspace(PlanningWorkspaceResetIntent::new(
                "/workspace",
                PlanningWorkspaceResetTarget::Queue,
            )),
        ));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("reset should start");
        };
        let correlation = correlation.clone();

        let completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation: correlation.clone(),
                result: Ok(Box::new(PlanningWorkspaceResetSnapshot {
                    target: PlanningWorkspaceResetTarget::All,
                    rewritten_paths: Vec::new(),
                    removed_paths: Vec::new(),
                })),
            },
        ));

        assert_eq!(
            completion.events,
            vec![AppEvent::PlanningWorkspaceResetCompleted {
                correlation,
                result: Err(
                    "planning workspace reset completion target mismatch: expected queue, received all"
                        .to_string(),
                ),
            }]
        );
        assert!(matches!(
            controller
                .handle_input(CoreInput::Command(AppCommand::ResetPlanningWorkspace(
                    PlanningWorkspaceResetIntent::new(
                        "/workspace",
                        PlanningWorkspaceResetTarget::Queue,
                    ),
                )))
                .events
                .as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation == 2
        ));
    }

    #[test]
    fn simple_stage_coalesces_and_stale_aba_panic_or_wrong_identity_fail_closed() {
        let mut controller = CoreController::new();
        let command = AppCommand::StageSimplePlanningDraft {
            workspace_directory: "/workspace".to_string(),
        };
        let started = controller.handle_input(CoreInput::Command(command.clone()));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("simple stage should start");
        };
        let first = correlation.clone();
        assert_eq!(
            started.effects,
            vec![CoreEffect::StageSimplePlanningDraft {
                correlation: first.clone(),
            }]
        );

        let duplicate = controller.handle_input(CoreInput::Command(command.clone()));
        assert_eq!(
            duplicate.events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: first.clone(),
                },
            )]
        );
        assert!(duplicate.effects.is_empty());

        let source = PlanningEditorSessionIdentity::new(9, "/workspace", "draft-a");
        let busy =
            controller.handle_input(CoreInput::Command(AppCommand::PromoteSimplePlanningDraft {
                workspace_directory: "/workspace".to_string(),
                draft_name: "draft-a".to_string(),
                source_session: source,
            }));
        assert!(matches!(
            busy.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation,
                    requested,
                }
            )] if active_correlation == &first
                && matches!(
                    &requested.operation,
                    PlanningWorkspaceOperationKind::PromoteSimpleDraft { .. }
                )
        ));

        let mut stale = first.clone();
        stale.generation = 99;
        assert!(
            controller
                .handle_input(CoreInput::EffectCompleted(
                    CoreEffectCompletion::PlanningSimpleDraftStaged {
                        correlation: stale,
                        result: Err("stale".to_string()),
                    },
                ))
                .events
                .is_empty()
        );

        let wrong = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningSimpleDraftStaged {
                correlation: first.clone(),
                result: Ok(Box::new(PlanningSimpleDraftStageSnapshot {
                    session_identity: PlanningEditorSessionIdentity::new(
                        77,
                        "/workspace",
                        "draft-a",
                    ),
                    staged_file_count: 3,
                    validation_report: Default::default(),
                })),
            },
        ));
        assert!(matches!(
            wrong.events.as_slice(),
            [AppEvent::PlanningSimpleDraftStaged {
                correlation,
                result: Err(error),
            }] if correlation == &first
                && error == "planning simple draft stage completion identity mismatch"
        ));

        let restarted = controller.handle_input(CoreInput::Command(command.clone()));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = restarted.events.as_slice()
        else {
            panic!("stage should reopen after a mismatched completion");
        };
        let second = correlation.clone();
        assert_eq!(second.generation, 2);
        assert!(
            controller
                .handle_input(CoreInput::EffectCompleted(
                    CoreEffectCompletion::PlanningSimpleDraftStaged {
                        correlation: first,
                        result: Err("aba".to_string()),
                    },
                ))
                .events
                .is_empty()
        );

        let panic_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningSimpleDraftStaged {
                correlation: second.clone(),
                result: Err("planning simple draft stage worker panicked".to_string()),
            },
        ));
        assert!(matches!(
            panic_completion.events.as_slice(),
            [AppEvent::PlanningSimpleDraftStaged {
                correlation,
                result: Err(error),
            }] if correlation == &second
                && error == "planning simple draft stage worker panicked"
        ));
        assert!(matches!(
            controller
                .handle_input(CoreInput::Command(command))
                .events
                .as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation == 3
        ));
    }

    #[test]
    fn editor_stage_requires_exact_target_workspace_generation_draft_and_completion_kind() {
        let mut controller = CoreController::new();
        let target = PlanningEditorStageTarget::PlanningManual;
        let command = AppCommand::StagePlanningEditor {
            workspace_directory: "/workspace".to_string(),
            target: target.clone(),
        };
        let started = controller.handle_input(CoreInput::Command(command.clone()));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("planning editor stage should start");
        };
        let first = correlation.clone();
        assert_eq!(
            started.effects,
            vec![CoreEffect::StagePlanningEditor {
                correlation: first.clone(),
            }]
        );
        assert_eq!(
            controller
                .handle_input(CoreInput::Command(command.clone()))
                .events,
            vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: first.clone(),
                },
            )]
        );
        assert!(matches!(
            controller
                .handle_input(CoreInput::Command(AppCommand::StagePlanningEditor {
                    workspace_directory: "/workspace".to_string(),
                    target: PlanningEditorStageTarget::QueueIdlePrompt,
                }))
                .events
                .as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation,
                    ..
                }
            )] if active_correlation == &first
        ));

        let snapshot = |target: PlanningEditorStageTarget,
                        identity: PlanningEditorSessionIdentity| {
            Box::new(PlanningEditorStageSnapshot {
                target,
                session: PlanningEditorSessionSnapshot {
                    session_identity: identity,
                    draft_directory: "/workspace/drafts/draft-a".to_string(),
                    editable_files: Vec::new(),
                    validation_report: Default::default(),
                    source_planning_revision: None,
                },
            })
        };
        for malformed in [
            snapshot(
                PlanningEditorStageTarget::QueueIdlePrompt,
                first.editor_session_identity("draft-a"),
            ),
            snapshot(
                target.clone(),
                PlanningEditorSessionIdentity::new(first.generation + 1, "/workspace", "draft-a"),
            ),
            snapshot(
                target.clone(),
                PlanningEditorSessionIdentity::new(first.generation, "/other", "draft-a"),
            ),
            snapshot(target.clone(), first.editor_session_identity("")),
        ] {
            let rejected = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::PlanningEditorStaged {
                    correlation: first.clone(),
                    result: Ok(malformed),
                },
            ));
            assert!(rejected.events.is_empty());
            assert_eq!(
                controller
                    .handle_input(CoreInput::Command(command.clone()))
                    .events,
                vec![AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                    PlanningWorkspaceOperationAdmission::Coalesced {
                        correlation: first.clone(),
                    },
                )],
                "malformed success must not release the active editor-stage lease"
            );
        }

        let exact = snapshot(target.clone(), first.editor_session_identity("draft-a"));
        assert_eq!(
            controller
                .handle_input(CoreInput::EffectCompleted(
                    CoreEffectCompletion::PlanningEditorStaged {
                        correlation: first.clone(),
                        result: Ok(exact.clone()),
                    },
                ))
                .events,
            vec![AppEvent::PlanningEditorStaged {
                correlation: first.clone(),
                result: Ok(exact),
            }]
        );
        assert!(
            controller
                .handle_input(CoreInput::EffectCompleted(
                    CoreEffectCompletion::PlanningEditorStaged {
                        correlation: first.clone(),
                        result: Err("duplicate".to_string()),
                    },
                ))
                .events
                .is_empty()
        );

        let restarted = controller.handle_input(CoreInput::Command(command.clone()));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = restarted.events.as_slice()
        else {
            panic!("editor stage should restart after exact settlement");
        };
        let second = correlation.clone();
        assert_eq!(second.generation, first.generation + 1);
        assert!(
            controller
                .handle_input(CoreInput::EffectCompleted(
                    CoreEffectCompletion::PlanningEditorStaged {
                        correlation: first,
                        result: Err("aba".to_string()),
                    },
                ))
                .events
                .is_empty()
        );
        assert!(matches!(
            controller
                .handle_input(CoreInput::EffectCompleted(
                    CoreEffectCompletion::PlanningEditorStaged {
                        correlation: second.clone(),
                        result: Err("planning editor stage worker panicked".to_string()),
                    },
                ))
                .events
                .as_slice(),
            [AppEvent::PlanningEditorStaged {
                correlation,
                result: Err(error),
            }] if correlation == &second
                && error == "planning editor stage worker panicked"
        ));
        assert!(matches!(
            controller
                .handle_input(CoreInput::Command(command))
                .events
                .as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation == second.generation + 1
        ));
    }

    #[test]
    fn editor_mutation_accepts_exact_payload_then_settles_malformed_results_for_retry() {
        let workspace = "/workspace";
        let source = PlanningEditorSessionIdentity::new(31, workspace, "draft-a");
        let identity = PlanningEditorMutationIdentity::new(
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationTarget::Planning,
            "draft-a",
            source.clone(),
            4,
        );
        let request = || {
            Box::new(PlanningEditorMutationRequest {
                identity: identity.clone(),
                editable_files: vec![PlanningEditorFileSnapshot {
                    active_path: "active.md".to_string(),
                    staged_path: "staged.md".to_string(),
                    body: "body".to_string(),
                }],
            })
        };
        let command = || AppCommand::MutatePlanningEditor {
            workspace_directory: workspace.to_string(),
            request: request(),
        };

        let mut invalid_controller = CoreController::new();
        let invalid =
            invalid_controller.handle_input(CoreInput::Command(AppCommand::MutatePlanningEditor {
                workspace_directory: workspace.to_string(),
                request: Box::new(PlanningEditorMutationRequest {
                    identity: PlanningEditorMutationIdentity::new(
                        PlanningEditorMutationAction::Save,
                        PlanningEditorMutationTarget::Planning,
                        "draft-a",
                        PlanningEditorSessionIdentity::new(31, "/other", "draft-a"),
                        4,
                    ),
                    editable_files: Vec::new(),
                }),
            }));
        assert!(invalid.events.is_empty());
        assert!(invalid.effects.is_empty());

        let mut controller = CoreController::new();
        let started = controller.handle_input(CoreInput::Command(command()));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("editor mutation should start");
        };
        let mut active = correlation.clone();
        let first = active.clone();
        assert!(matches!(
            started.effects.as_slice(),
            [CoreEffect::MutatePlanningEditor {
                correlation,
                request: effect_request,
            }] if correlation == &active
                && effect_request.identity == identity
                && effect_request.editable_files[0].body == "body"
        ));
        assert!(matches!(
            controller
                .handle_input(CoreInput::Command(command()))
                .events
                .as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Coalesced { correlation }
            )] if correlation == &active
        ));
        let mut newer_revision = request();
        newer_revision.identity.buffer_revision += 1;
        assert!(matches!(
            controller
                .handle_input(CoreInput::Command(AppCommand::MutatePlanningEditor {
                    workspace_directory: workspace.to_string(),
                    request: newer_revision,
                }))
                .events
                .as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation,
                    ..
                }
            )] if active_correlation == &active
        ));

        let malformed = [
            PlanningEditorMutationResult::Saved {
                identity: PlanningEditorMutationIdentity {
                    buffer_revision: identity.buffer_revision + 1,
                    ..identity.clone()
                },
                draft_name: "draft-a".to_string(),
                validation_report: Default::default(),
            },
            PlanningEditorMutationResult::Promoted {
                identity: identity.clone(),
                draft_name: "draft-a".to_string(),
                promoted_file_count: 1,
                validation_report: Default::default(),
                committed_planning_revision: None,
            },
            PlanningEditorMutationResult::Saved {
                identity: identity.clone(),
                draft_name: "wrong-draft".to_string(),
                validation_report: Default::default(),
            },
        ];
        for (index, malformed) in malformed.into_iter().enumerate() {
            let settled = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::PlanningEditorMutationCompleted {
                    correlation: active.clone(),
                    result: Ok(Box::new(malformed)),
                },
            ));
            assert!(matches!(
                settled.events.as_slice(),
                [AppEvent::PlanningEditorMutationCompleted {
                    correlation,
                    result: Err(error),
                }] if correlation == &active
                    && error == "planning editor mutation completion identity mismatch"
            ));
            if index + 1 < 3 {
                let restarted = controller.handle_input(CoreInput::Command(command()));
                let [
                    AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                        PlanningWorkspaceOperationAdmission::Started { correlation },
                    ),
                ] = restarted.events.as_slice()
                else {
                    panic!("malformed exact completion must reopen admission");
                };
                active = correlation.clone();
                if index == 0 {
                    assert!(
                        controller
                            .handle_input(CoreInput::EffectCompleted(
                                CoreEffectCompletion::PlanningEditorMutationCompleted {
                                    correlation: first.clone(),
                                    result: Err("aba".to_string()),
                                },
                            ))
                            .events
                            .is_empty()
                    );
                }
            }
        }
        assert!(matches!(
            controller
                .handle_input(CoreInput::Command(command()))
                .events
                .as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation > first.generation
        ));
    }

    #[test]
    fn simple_editor_load_and_promotion_require_exact_draft_and_session_identity() {
        let mut controller = CoreController::new();
        let source = PlanningEditorSessionIdentity::new(41, "/workspace", "draft-a");
        let load_command = AppCommand::LoadSimplePlanningEditor {
            workspace_directory: "/workspace".to_string(),
            draft_name: "draft-a".to_string(),
            source_session: source.clone(),
        };
        let started = controller.handle_input(CoreInput::Command(load_command.clone()));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = started.events.as_slice()
        else {
            panic!("simple editor load should start");
        };
        let first_load = correlation.clone();
        assert_eq!(
            started.effects,
            vec![CoreEffect::LoadSimplePlanningEditor {
                correlation: first_load.clone(),
            }]
        );

        let wrong_load = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningSimpleEditorLoaded {
                correlation: first_load.clone(),
                result: Ok(Box::new(PlanningEditorSessionSnapshot {
                    session_identity: first_load.editor_session_identity("draft-b"),
                    draft_directory: "/workspace/drafts/draft-b".to_string(),
                    editable_files: Vec::new(),
                    validation_report: Default::default(),
                    source_planning_revision: None,
                })),
            },
        ));
        assert!(matches!(
            wrong_load.events.as_slice(),
            [AppEvent::PlanningSimpleEditorLoaded {
                correlation,
                result: Err(error),
            }] if correlation == &first_load
                && error == "planning simple editor completion identity mismatch"
        ));

        let restarted = controller.handle_input(CoreInput::Command(load_command));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = restarted.events.as_slice()
        else {
            panic!("simple editor load should restart");
        };
        let second_load = correlation.clone();
        let editor = Box::new(PlanningEditorSessionSnapshot {
            session_identity: second_load.editor_session_identity("draft-a"),
            draft_directory: "/workspace/drafts/draft-a".to_string(),
            editable_files: vec![PlanningEditorFileSnapshot {
                active_path: "planning/result-output.md".to_string(),
                staged_path: "drafts/draft-a/planning/result-output.md".to_string(),
                body: "operator payload".to_string(),
            }],
            validation_report: Default::default(),
            source_planning_revision: None,
        });
        let loaded = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningSimpleEditorLoaded {
                correlation: second_load.clone(),
                result: Ok(editor.clone()),
            },
        ));
        assert_eq!(
            loaded.events,
            vec![AppEvent::PlanningSimpleEditorLoaded {
                correlation: second_load,
                result: Ok(editor),
            }]
        );

        let promote_command = AppCommand::PromoteSimplePlanningDraft {
            workspace_directory: "/workspace".to_string(),
            draft_name: "draft-a".to_string(),
            source_session: source,
        };
        let promote_started = controller.handle_input(CoreInput::Command(promote_command.clone()));
        let [
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation },
            ),
        ] = promote_started.events.as_slice()
        else {
            panic!("simple draft promotion should start");
        };
        let first_promote = correlation.clone();
        assert_eq!(
            promote_started.effects,
            vec![CoreEffect::PromoteSimplePlanningDraft {
                correlation: first_promote.clone(),
            }]
        );
        let wrong_promotion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningSimpleDraftPromoted {
                correlation: first_promote.clone(),
                result: Ok(Box::new(PlanningSimpleDraftPromotionSnapshot {
                    draft_name: "draft-b".to_string(),
                    promoted_file_count: 2,
                    validation_report: Default::default(),
                })),
            },
        ));
        assert!(matches!(
            wrong_promotion.events.as_slice(),
            [AppEvent::PlanningSimpleDraftPromoted {
                correlation,
                result: Err(error),
            }] if correlation == &first_promote
                && error == "planning simple draft promotion completion identity mismatch"
        ));

        let restarted = controller.handle_input(CoreInput::Command(promote_command));
        assert!(matches!(
            restarted.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation == first_promote.generation + 1
        ));
    }

    #[test]
    fn malformed_simple_authoring_source_sessions_are_rejected_before_admission() {
        let mut controller = CoreController::new();

        for command in [
            AppCommand::LoadSimplePlanningEditor {
                workspace_directory: "/workspace".to_string(),
                draft_name: "draft-a".to_string(),
                source_session: PlanningEditorSessionIdentity::new(
                    41,
                    "/other-workspace",
                    "draft-a",
                ),
            },
            AppCommand::PromoteSimplePlanningDraft {
                workspace_directory: "/workspace".to_string(),
                draft_name: "draft-a".to_string(),
                source_session: PlanningEditorSessionIdentity::new(42, "/workspace", "other-draft"),
            },
        ] {
            let rejected = controller.handle_input(CoreInput::Command(command));
            assert!(rejected.events.is_empty());
            assert!(rejected.effects.is_empty());
        }

        let valid =
            controller.handle_input(CoreInput::Command(AppCommand::LoadSimplePlanningEditor {
                workspace_directory: "/workspace".to_string(),
                draft_name: "draft-a".to_string(),
                source_session: PlanningEditorSessionIdentity::new(43, "/workspace", "draft-a"),
            }));
        assert!(matches!(
            valid.events.as_slice(),
            [AppEvent::PlanningWorkspaceOperationAdmissionResolved(
                PlanningWorkspaceOperationAdmission::Started { correlation }
            )] if correlation.generation == 1
        ));
        assert!(matches!(
            valid.effects.as_slice(),
            [CoreEffect::LoadSimplePlanningEditor { correlation }]
                if correlation.generation == 1
        ));
    }

    #[test]
    fn wrong_planning_completion_variants_never_release_the_active_lease() {
        assert_wrong_planning_completion_kind_keeps_active_lease(
            AppCommand::ResetPlanningWorkspace(PlanningWorkspaceResetIntent::new(
                "/workspace",
                PlanningWorkspaceResetTarget::Directions,
            )),
            |correlation| CoreEffectCompletion::PlanningSimpleDraftStaged {
                correlation,
                result: Err("wrong completion variant".to_string()),
            },
            |correlation| CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation,
                result: Err("exact completion".to_string()),
            },
        );
        assert_wrong_planning_completion_kind_keeps_active_lease(
            AppCommand::StageSimplePlanningDraft {
                workspace_directory: "/workspace".to_string(),
            },
            |correlation| CoreEffectCompletion::PlanningSimpleEditorLoaded {
                correlation,
                result: Err("wrong completion variant".to_string()),
            },
            |correlation| CoreEffectCompletion::PlanningSimpleDraftStaged {
                correlation,
                result: Err("exact completion".to_string()),
            },
        );
        assert_wrong_planning_completion_kind_keeps_active_lease(
            AppCommand::StagePlanningEditor {
                workspace_directory: "/workspace".to_string(),
                target: PlanningEditorStageTarget::PlanningManual,
            },
            |correlation| CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation,
                result: Err("wrong completion variant".to_string()),
            },
            |correlation| CoreEffectCompletion::PlanningEditorStaged {
                correlation,
                result: Err("exact completion".to_string()),
            },
        );
        assert_wrong_planning_completion_kind_keeps_active_lease(
            AppCommand::MutatePlanningEditor {
                workspace_directory: "/workspace".to_string(),
                request: Box::new(PlanningEditorMutationRequest {
                    identity: PlanningEditorMutationIdentity::new(
                        PlanningEditorMutationAction::Save,
                        PlanningEditorMutationTarget::Planning,
                        "draft-a",
                        PlanningEditorSessionIdentity::new(41, "/workspace", "draft-a"),
                        0,
                    ),
                    editable_files: Vec::new(),
                }),
            },
            |correlation| CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation,
                result: Err("wrong completion variant".to_string()),
            },
            |correlation| CoreEffectCompletion::PlanningEditorMutationCompleted {
                correlation,
                result: Err("exact completion".to_string()),
            },
        );
        assert_wrong_planning_completion_kind_keeps_active_lease(
            AppCommand::LoadSimplePlanningEditor {
                workspace_directory: "/workspace".to_string(),
                draft_name: "draft-a".to_string(),
                source_session: PlanningEditorSessionIdentity::new(41, "/workspace", "draft-a"),
            },
            |correlation| CoreEffectCompletion::PlanningSimpleDraftPromoted {
                correlation,
                result: Err("wrong completion variant".to_string()),
            },
            |correlation| CoreEffectCompletion::PlanningSimpleEditorLoaded {
                correlation,
                result: Err("exact completion".to_string()),
            },
        );
        assert_wrong_planning_completion_kind_keeps_active_lease(
            AppCommand::PromoteSimplePlanningDraft {
                workspace_directory: "/workspace".to_string(),
                draft_name: "draft-a".to_string(),
                source_session: PlanningEditorSessionIdentity::new(42, "/workspace", "draft-a"),
            },
            |correlation| CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation,
                result: Err("wrong completion variant".to_string()),
            },
            |correlation| CoreEffectCompletion::PlanningSimpleDraftPromoted {
                correlation,
                result: Err("exact completion".to_string()),
            },
        );
    }

    #[test]
    fn run_startup_checks_marks_startup_loading() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(run_startup_checks_command("/tmp/workspace"));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(outcome.snapshot.startup, StartupSnapshot::Loading);
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged {
                correlation: startup_check_correlation(1),
                snapshot: StartupSnapshot::Loading,
            }]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::RunStartupChecks {
                correlation: startup_check_correlation(1),
            }]
        );
    }

    #[test]
    fn startup_completion_marks_startup_ready() {
        let mut controller = CoreController::new();
        let ready_snapshot = sample_startup_ready_snapshot();
        controller.handle_input(run_startup_checks_command("/tmp/workspace"));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_check_correlation(1),
                result: Ok(Box::new(ready_snapshot.clone())),
            },
        ));

        assert_eq!(outcome.snapshot.revision, 2);
        assert_eq!(
            outcome.snapshot.startup,
            StartupSnapshot::Ready(Box::new(ready_snapshot.clone()))
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged {
                correlation: startup_check_correlation(1),
                snapshot: StartupSnapshot::Ready(Box::new(ready_snapshot)),
            }]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn startup_completion_marks_startup_failed() {
        let mut controller = CoreController::new();
        controller.handle_input(run_startup_checks_command("/tmp/workspace"));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_check_correlation(1),
                result: Err("codex missing".to_string()),
            },
        ));

        assert_eq!(outcome.snapshot.revision, 2);
        assert_eq!(
            outcome.snapshot.startup,
            StartupSnapshot::Failed {
                message: "codex missing".to_string()
            }
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged {
                correlation: startup_check_correlation(1),
                snapshot: StartupSnapshot::Failed {
                    message: "codex missing".to_string(),
                },
            }]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn startup_completion_only_accepts_latest_generation_in_both_orders() {
        let mut stale_success = CoreController::new();
        stale_success.handle_input(run_startup_checks_command("/tmp/workspace"));
        stale_success.handle_input(run_startup_checks_command("/tmp/workspace"));

        let dropped = stale_success.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_check_correlation(1),
                result: Ok(Box::new(sample_startup_ready_snapshot())),
            },
        ));
        assert!(dropped.events.is_empty());
        assert_eq!(dropped.snapshot.startup, StartupSnapshot::Loading);

        let accepted = stale_success.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_check_correlation(2),
                result: Err("latest startup failed".to_string()),
            },
        ));
        assert!(matches!(
            accepted.snapshot.startup,
            StartupSnapshot::Failed { ref message } if message == "latest startup failed"
        ));

        let mut stale_failure = CoreController::new();
        stale_failure.handle_input(run_startup_checks_command("/tmp/workspace"));
        stale_failure.handle_input(run_startup_checks_command("/tmp/workspace"));
        let accepted = stale_failure.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_check_correlation(2),
                result: Ok(Box::new(sample_startup_ready_snapshot())),
            },
        ));
        let accepted_snapshot = accepted.snapshot.startup.clone();
        let dropped = stale_failure.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_check_correlation(1),
                result: Err("stale startup failed".to_string()),
            },
        ));
        assert!(dropped.events.is_empty());
        assert_eq!(dropped.snapshot.startup, accepted_snapshot);
    }

    #[test]
    fn startup_completion_requires_exact_aba_workspace_correlation_and_is_single_use() {
        let mut controller = CoreController::new();
        controller.handle_input(run_startup_checks_command("/tmp/workspace-a"));
        controller.handle_input(run_startup_checks_command("/tmp/workspace-b"));
        controller.handle_input(run_startup_checks_command("/tmp/workspace-a"));

        for correlation in [
            startup_check_correlation_for(1, "/tmp/workspace-a"),
            startup_check_correlation_for(2, "/tmp/workspace-b"),
            startup_check_correlation_for(3, "/tmp/workspace-b"),
        ] {
            let stale = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::StartupChecksLoaded {
                    correlation,
                    result: Err("stale startup failure".to_string()),
                },
            ));
            assert!(stale.events.is_empty());
            assert_eq!(stale.snapshot.startup, StartupSnapshot::Loading);
        }

        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_check_correlation_for(3, "/tmp/workspace-a"),
                result: Ok(Box::new(sample_startup_ready_snapshot())),
            },
        ));
        assert!(matches!(
            accepted.snapshot.startup,
            StartupSnapshot::Ready(_)
        ));

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_check_correlation_for(3, "/tmp/workspace-a"),
                result: Err("duplicate startup failure".to_string()),
            },
        ));
        assert!(duplicate.events.is_empty());
        assert_eq!(duplicate.snapshot.startup, accepted.snapshot.startup);
    }

    #[test]
    fn load_session_catalog_marks_session_loading() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(ensure_session_catalog_command(10, "/tmp/workspace"));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            outcome.snapshot.session_catalog,
            SessionCatalogSnapshot::Loading
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::SessionCatalogChanged(
                SessionCatalogSnapshot::Loading
            )]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadSessionCatalog {
                correlation: session_catalog_correlation(1, 10, "/tmp/workspace"),
            }]
        );
    }

    #[test]
    fn ensure_session_catalog_is_idle_only_while_refresh_restarts_settled_states() {
        let mut controller = CoreController::new();
        controller.handle_input(ensure_session_catalog_command(10, "/tmp/workspace"));
        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(1, 10, "/tmp/workspace"),
                result: Err("catalog unavailable".to_string()),
            },
        ));

        let ensure_after_failure =
            controller.handle_input(ensure_session_catalog_command(10, "/tmp/workspace"));
        assert!(ensure_after_failure.events.is_empty());
        assert!(ensure_after_failure.effects.is_empty());
        assert_eq!(ensure_after_failure.snapshot, failed.snapshot);

        let refresh_after_failure =
            controller.handle_input(refresh_session_catalog_command(10, "/tmp/workspace"));
        assert_eq!(
            refresh_after_failure.effects,
            vec![CoreEffect::LoadSessionCatalog {
                correlation: session_catalog_correlation(2, 10, "/tmp/workspace"),
            }]
        );
        let ready = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(2, 10, "/tmp/workspace"),
                result: Ok(SessionCatalogReadySnapshot::from_catalog(
                    RecentSessions {
                        items: Vec::new(),
                        warnings: Vec::new(),
                        next_cursor: None,
                    }
                    .into(),
                )),
            },
        ));

        let ensure_after_ready =
            controller.handle_input(ensure_session_catalog_command(10, "/tmp/workspace"));
        assert!(ensure_after_ready.events.is_empty());
        assert!(ensure_after_ready.effects.is_empty());
        assert_eq!(ensure_after_ready.snapshot, ready.snapshot);

        let refresh_after_ready =
            controller.handle_input(refresh_session_catalog_command(10, "/tmp/workspace"));
        assert_eq!(
            refresh_after_ready.effects,
            vec![CoreEffect::LoadSessionCatalog {
                correlation: session_catalog_correlation(3, 10, "/tmp/workspace"),
            }]
        );
    }

    #[test]
    fn identical_active_session_catalog_target_coalesces_without_consuming_generation() {
        let mut controller = CoreController::new();
        let first = controller.handle_input(refresh_session_catalog_command(10, "/tmp/workspace"));
        assert_eq!(
            first.effects,
            vec![CoreEffect::LoadSessionCatalog {
                correlation: session_catalog_correlation(1, 10, "/tmp/workspace"),
            }]
        );

        for duplicate in [
            ensure_session_catalog_command(10, "/tmp/workspace"),
            refresh_session_catalog_command(10, "/tmp/workspace"),
        ] {
            let outcome = controller.handle_input(duplicate);
            assert!(outcome.events.is_empty());
            assert!(outcome.effects.is_empty());
        }

        let replacement =
            controller.handle_input(refresh_session_catalog_command(20, "/tmp/workspace"));
        assert_eq!(
            replacement.effects,
            vec![CoreEffect::LoadSessionCatalog {
                correlation: session_catalog_correlation(2, 20, "/tmp/workspace"),
            }]
        );
    }

    #[test]
    fn different_active_session_catalog_target_replaces_even_for_ensure() {
        let mut controller = CoreController::new();
        controller.handle_input(ensure_session_catalog_command(10, "/tmp/workspace-a"));

        let replacement =
            controller.handle_input(ensure_session_catalog_command(10, "/tmp/workspace-b"));

        assert_eq!(
            replacement.effects,
            vec![CoreEffect::LoadSessionCatalog {
                correlation: session_catalog_correlation(2, 10, "/tmp/workspace-b"),
            }]
        );
        assert_eq!(
            controller.session_feature.active_catalog_load_for_test(),
            Some(&session_catalog_correlation(2, 10, "/tmp/workspace-b"))
        );
    }

    #[test]
    fn rename_session_dispatches_core_effect_without_changing_snapshot() {
        let mut controller = CoreController::new();
        let request = SessionRenameRequest::new("thread-1", "Renamed");

        let outcome =
            controller.handle_input(CoreInput::Command(AppCommand::RenameSession(request)));

        assert_eq!(
            outcome.events,
            vec![AppEvent::SessionRenameAdmissionResolved(
                SessionRenameAdmission::Accepted {
                    correlation: session_rename_correlation(1, "thread-1", "Renamed"),
                },
            )]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::RenameSession {
                correlation: session_rename_correlation(1, "thread-1", "Renamed"),
            }]
        );
        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
    }

    #[test]
    fn matching_session_rename_updates_core_projections_in_one_revision() {
        let mut controller = CoreController::new();
        load_test_session_catalog(&mut controller);
        load_test_conversation(&mut controller, "thread-beta");
        let revision_before_rename = controller.snapshot().revision;
        let correlation = session_rename_correlation(1, "thread-beta", "Beta renamed");
        let command = controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            correlation.request.clone(),
        )));
        assert!(matches!(
            command.effects.as_slice(),
            [CoreEffect::RenameSession {
                correlation: effect_correlation
            }] if effect_correlation == &correlation
        ));
        let snapshot_before_rename = command.snapshot.clone();

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: correlation.clone(),
                result: Ok(()),
            },
        ));

        assert!(!Arc::ptr_eq(&snapshot_before_rename, &outcome.snapshot));
        let ConversationSnapshot::Ready(previous_conversation) =
            &snapshot_before_rename.conversation
        else {
            panic!("retained conversation should remain ready");
        };
        assert_eq!(previous_conversation.title, "Core runtime");
        assert_eq!(previous_conversation.conversation.title, "Core runtime");
        assert_eq!(outcome.snapshot.revision, revision_before_rename + 1);
        let SessionCatalogSnapshot::Ready(catalog) = &outcome.snapshot.session_catalog else {
            panic!("session catalog should remain ready");
        };
        let sessions = catalog
            .catalog
            .recent_sessions()
            .expect("ready catalog should expose rows");
        assert_eq!(
            sessions
                .items
                .iter()
                .find(|session| session.id == "thread-alpha")
                .and_then(|session| session.name.as_deref()),
            Some("Alpha")
        );
        assert_eq!(
            sessions
                .items
                .iter()
                .find(|session| session.id == "thread-beta")
                .and_then(|session| session.name.as_deref()),
            Some("Beta renamed")
        );
        let ConversationSnapshot::Ready(conversation) = &outcome.snapshot.conversation else {
            panic!("conversation should remain ready");
        };
        assert_eq!(conversation.title, "Beta renamed");
        assert_eq!(conversation.conversation.title, "Beta renamed");
        let [
            AppEvent::SessionRenameCompleted {
                correlation: event_correlation,
                result: Ok(accepted),
            },
        ] = outcome.events.as_slice()
        else {
            panic!("matching rename should publish one accepted projection");
        };
        assert_eq!(event_correlation, &correlation);
        assert_eq!(
            accepted.session_catalog,
            outcome.snapshot.session_catalog.clone()
        );
        let stream = accepted
            .turn_stream
            .as_deref()
            .expect("matching loaded conversation should publish stream identity");
        assert_eq!(stream.title.as_deref(), Some("Beta renamed"));
        assert_eq!(
            stream.update,
            TurnStreamUpdate::SessionRenamed {
                thread_id: "thread-beta".to_string(),
                title: "Beta renamed".to_string(),
            }
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn session_rename_does_not_change_another_loaded_conversation() {
        let mut controller = CoreController::new();
        load_test_session_catalog(&mut controller);
        load_test_conversation(&mut controller, "thread-alpha");
        let correlation = session_rename_correlation(1, "thread-beta", "Beta renamed");
        controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            correlation.request.clone(),
        )));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation,
                result: Ok(()),
            },
        ));

        let ConversationSnapshot::Ready(conversation) = &outcome.snapshot.conversation else {
            panic!("conversation should remain ready");
        };
        assert_eq!(conversation.thread_id, "thread-alpha");
        assert_eq!(conversation.title, "Core runtime");
        assert_eq!(conversation.conversation.title, "Core runtime");
    }

    #[test]
    fn renamed_title_wins_over_a_late_same_thread_prepared_event() {
        let mut controller = CoreController::new();
        load_test_session_catalog(&mut controller);
        load_test_conversation(&mut controller, "thread-beta");
        let correlation = session_rename_correlation(1, "thread-beta", "Beta renamed");
        controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            correlation.request.clone(),
        )));
        let turn = submit_test_turn(&mut controller, Some("thread-beta"));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation,
                result: Ok(()),
            },
        ));

        let outcome = controller.handle_input(test_turn_stream_input(
            turn,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-beta".to_string(),
                title: "Stale provider title".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));

        assert!(matches!(
            outcome.events.as_slice(),
            [AppEvent::TurnStreamSnapshotChanged(snapshot)]
                if snapshot.title.as_deref() == Some("Beta renamed")
                    && matches!(
                        &snapshot.update,
                        TurnStreamUpdate::ThreadPrepared { title, .. }
                            if title == "Beta renamed"
                    )
        ));
    }

    #[test]
    fn provider_title_is_accepted_for_a_turn_started_after_rename() {
        let mut controller = CoreController::new();
        load_test_session_catalog(&mut controller);
        load_test_conversation(&mut controller, "thread-beta");
        let correlation = session_rename_correlation(1, "thread-beta", "Beta renamed");
        controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            correlation.request.clone(),
        )));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation,
                result: Ok(()),
            },
        ));

        let turn = submit_test_turn(&mut controller, Some("thread-beta"));
        let outcome = controller.handle_input(test_turn_stream_input(
            turn,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-beta".to_string(),
                title: "Provider retitled".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));

        assert!(matches!(
            outcome.events.as_slice(),
            [AppEvent::TurnStreamSnapshotChanged(snapshot)]
                if snapshot.title.as_deref() == Some("Provider retitled")
                    && matches!(
                        &snapshot.update,
                        TurnStreamUpdate::ThreadPrepared { title, .. }
                            if title == "Provider retitled"
                    )
        ));
    }

    #[test]
    fn session_rename_failure_and_stale_completions_leave_snapshot_unchanged() {
        let mut controller = CoreController::new();
        load_test_session_catalog(&mut controller);
        load_test_conversation(&mut controller, "thread-alpha");
        let snapshot_before_rename = controller.snapshot();
        let correlation = session_rename_correlation(1, "thread-alpha", "Renamed");
        controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            correlation.request.clone(),
        )));
        let duplicate_command = controller.handle_input(CoreInput::Command(
            AppCommand::RenameSession(SessionRenameRequest::new("thread-2", "Other")),
        ));
        assert_eq!(
            duplicate_command.events,
            vec![AppEvent::SessionRenameAdmissionResolved(
                SessionRenameAdmission::RejectedActive {
                    active_correlation: correlation.clone(),
                },
            )]
        );
        assert!(duplicate_command.effects.is_empty());

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: session_rename_correlation(99, "thread-alpha", "Renamed"),
                result: Ok(()),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(*stale.snapshot, snapshot_before_rename);

        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: correlation.clone(),
                result: Err("provider unavailable".to_string()),
            },
        ));
        assert_eq!(*failed.snapshot, snapshot_before_rename);
        assert!(matches!(
            failed.events.as_slice(),
            [AppEvent::SessionRenameCompleted {
                correlation: event_correlation,
                result: Err(message),
            }] if event_correlation == &correlation && message == "provider unavailable"
        ));

        let duplicate_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation,
                result: Ok(()),
            },
        ));
        assert!(duplicate_completion.events.is_empty());
        assert_eq!(*duplicate_completion.snapshot, snapshot_before_rename);
    }

    #[test]
    fn same_request_session_rename_aba_is_filtered_before_adapter_events() {
        let mut controller = CoreController::new();
        let request = SessionRenameRequest::new("thread-unloaded", "Renamed");
        let first = session_rename_correlation(1, "thread-unloaded", "Renamed");
        controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            request.clone(),
        )));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: first.clone(),
                result: Err("first attempt failed".to_string()),
            },
        ));

        let second = session_rename_correlation(2, "thread-unloaded", "Renamed");
        let retried =
            controller.handle_input(CoreInput::Command(AppCommand::RenameSession(request)));
        assert_eq!(
            retried.events,
            vec![AppEvent::SessionRenameAdmissionResolved(
                SessionRenameAdmission::Accepted {
                    correlation: second.clone(),
                },
            )]
        );

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: first,
                result: Ok(()),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(
            controller.session_feature.active_rename_for_test(),
            Some(&second)
        );

        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: second.clone(),
                result: Ok(()),
            },
        ));
        assert!(matches!(
            accepted.events.as_slice(),
            [AppEvent::SessionRenameCompleted {
                correlation,
                result: Ok(_),
            }] if correlation == &second
        ));

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: second,
                result: Ok(()),
            },
        ));
        assert!(duplicate.events.is_empty());
    }

    #[test]
    fn session_rename_success_without_a_projected_row_keeps_revision() {
        let mut controller = CoreController::new();
        let correlation = session_rename_correlation(1, "thread-unloaded", "Renamed");
        controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            correlation.request.clone(),
        )));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: correlation.clone(),
                result: Ok(()),
            },
        ));

        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            outcome.events,
            vec![AppEvent::SessionRenameCompleted {
                correlation,
                result: Ok(SessionRenameAcceptedSnapshot {
                    session_catalog: SessionCatalogSnapshot::Idle,
                    turn_stream: None,
                }),
            }]
        );
    }

    #[test]
    fn session_rename_rejects_conflicting_loads_but_allows_other_thread_load() {
        let mut catalog_loading = CoreController::new();
        catalog_loading.handle_input(ensure_session_catalog_command(10, "/tmp/workspace"));
        let rejected = catalog_loading.handle_input(CoreInput::Command(AppCommand::RenameSession(
            SessionRenameRequest::new("thread-1", "Renamed"),
        )));
        assert!(rejected.effects.is_empty());
        assert_eq!(
            rejected.events,
            vec![AppEvent::SessionRenameAdmissionResolved(
                SessionRenameAdmission::RejectedCatalogLoading {
                    active_correlation: session_catalog_correlation(1, 10, "/tmp/workspace"),
                },
            )]
        );

        let mut same_thread_loading = CoreController::new();
        same_thread_loading.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-1".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        let rejected = same_thread_loading.handle_input(CoreInput::Command(
            AppCommand::RenameSession(SessionRenameRequest::new("thread-1", "Renamed")),
        ));
        assert!(rejected.effects.is_empty());
        assert!(matches!(
            rejected.events.as_slice(),
            [AppEvent::SessionRenameAdmissionResolved(
                SessionRenameAdmission::RejectedConversationLoading { active_correlation }
            )] if active_correlation.generation == 1
                && active_correlation.requested_thread_id == "thread-1"
        ));

        let accepted = same_thread_loading.handle_input(CoreInput::Command(
            AppCommand::RenameSession(SessionRenameRequest::new("thread-2", "Other renamed")),
        ));
        assert!(matches!(
            accepted.effects.as_slice(),
            [CoreEffect::RenameSession { correlation }]
                if correlation.generation == 1 && correlation.request.thread_id == "thread-2"
        ));
    }

    #[test]
    fn active_session_rename_defers_conflicting_reads_and_allows_other_thread_load() {
        let mut catalog = CoreController::new();
        load_test_session_catalog(&mut catalog);
        let catalog_rename = session_rename_correlation(1, "thread-1", "Renamed");
        catalog.handle_input(CoreInput::Command(AppCommand::RenameSession(
            catalog_rename.request.clone(),
        )));
        let skipped_ensure =
            catalog.handle_input(ensure_session_catalog_command(20, "/tmp/ignored"));
        assert!(skipped_ensure.events.is_empty());
        assert!(skipped_ensure.effects.is_empty());
        assert!(!catalog.session_feature.has_deferred_catalog_load_for_test());

        let deferred = catalog.handle_input(refresh_session_catalog_command(10, "/tmp/first"));
        assert!(deferred.events.is_empty());
        assert!(deferred.effects.is_empty());
        let replacement = catalog.handle_input(refresh_session_catalog_command(20, "/tmp/latest"));
        assert!(replacement.events.is_empty());
        assert!(replacement.effects.is_empty());
        let resumed = catalog.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: catalog_rename,
                result: Ok(()),
            },
        ));
        assert!(matches!(
            resumed.events.as_slice(),
            [
                AppEvent::SessionRenameCompleted { result: Ok(_), .. },
                AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Loading),
            ]
        ));
        assert!(matches!(
            resumed.effects.as_slice(),
            [CoreEffect::LoadSessionCatalog {
                correlation: SessionCatalogLoadCorrelation {
                    generation: 2,
                    limit: 20,
                    workspace_directory,
                },
            }] if workspace_directory == "/tmp/latest"
        ));

        let mut conversations = CoreController::new();
        load_test_session_catalog(&mut conversations);
        load_test_conversation(&mut conversations, "thread-beta");
        let snapshot_before_rename = conversations.snapshot();
        let conversation_rename = session_rename_correlation(1, "thread-beta", "Beta renamed");
        conversations.handle_input(CoreInput::Command(AppCommand::RenameSession(
            conversation_rename.request.clone(),
        )));
        let deferred =
            conversations.handle_input(CoreInput::Command(AppCommand::LoadConversation {
                thread_id: "thread-beta".to_string(),
                fallback_workspace_directory: "/tmp/workspace".to_string(),
            }));
        assert!(deferred.effects.is_empty());
        assert_eq!(*deferred.snapshot, snapshot_before_rename);
        let resumed = conversations.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: conversation_rename,
                result: Ok(()),
            },
        ));
        assert!(matches!(
            resumed.events.as_slice(),
            [
                AppEvent::SessionRenameCompleted { result: Ok(_), .. },
                AppEvent::ConversationChanged {
                    correlation: Some(ConversationLoadCorrelation {
                        generation: 2,
                        requested_thread_id,
                    }),
                    snapshot: ConversationSnapshot::Loading,
                },
            ] if requested_thread_id == "thread-beta"
        ));
        assert!(matches!(
            resumed.effects.as_slice(),
            [CoreEffect::LoadConversation { correlation, .. }]
                if correlation.requested_thread_id == "thread-beta"
        ));

        let mut other_thread = CoreController::new();
        other_thread.handle_input(CoreInput::Command(AppCommand::RenameSession(
            SessionRenameRequest::new("thread-1", "Renamed"),
        )));
        let allowed = other_thread.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-2".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        assert!(matches!(
            allowed.effects.as_slice(),
            [CoreEffect::LoadConversation { correlation, .. }]
                if correlation.requested_thread_id == "thread-2"
        ));
    }

    #[test]
    fn session_rename_resumes_deferred_catalog_before_same_thread_conversation() {
        let mut controller = CoreController::new();
        load_test_session_catalog(&mut controller);
        let rename = session_rename_correlation(1, "thread-beta", "Beta renamed");
        controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            rename.request.clone(),
        )));
        controller.handle_input(refresh_session_catalog_command(20, "/tmp/latest"));
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-beta".to_string(),
            fallback_workspace_directory: "/tmp/fallback".to_string(),
        }));

        let resumed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: rename.clone(),
                result: Ok(()),
            },
        ));

        assert!(matches!(
            resumed.events.as_slice(),
            [
                AppEvent::SessionRenameCompleted {
                    correlation,
                    result: Ok(_),
                },
                AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Loading),
                AppEvent::ConversationChanged {
                    correlation: Some(ConversationLoadCorrelation {
                        generation: 1,
                        requested_thread_id,
                    }),
                    snapshot: ConversationSnapshot::Loading,
                },
            ] if correlation == &rename && requested_thread_id == "thread-beta"
        ));
        assert!(matches!(
            resumed.effects.as_slice(),
            [
                CoreEffect::LoadSessionCatalog {
                    correlation: SessionCatalogLoadCorrelation {
                        generation: 2,
                        limit: 20,
                        workspace_directory,
                    },
                },
                CoreEffect::LoadConversation {
                    correlation: ConversationLoadCorrelation {
                        generation: 1,
                        requested_thread_id,
                    },
                    fallback_workspace_directory,
                },
            ] if workspace_directory == "/tmp/latest"
                && requested_thread_id == "thread-beta"
                && fallback_workspace_directory == "/tmp/fallback"
        ));
    }

    #[test]
    fn deferred_conversation_load_cancels_active_post_turn_authority_immediately() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let request = post_turn_request(
            RuntimeProjection::invalid("refresh required"),
            Vec::new(),
            false,
            PlanningWorkerPanelState::default(),
        );
        let started = controller.handle_input(CoreInput::Command(AppCommand::EvaluatePostTurn(
            Box::new(request),
        )));
        let stale_correlation = post_turn_effect_correlation(&started);
        let stale_permit = post_turn_effect_permit(&started);

        controller.handle_input(CoreInput::Command(AppCommand::RenameSession(
            SessionRenameRequest::new("thread-2", "Renamed"),
        )));
        let deferred = controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-2".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));

        assert!(deferred.effects.is_empty());
        assert!(
            controller
                .active_post_turn_evaluation_correlation()
                .is_none()
        );
        assert!(
            !stale_permit.is_current(),
            "the conversation intent must revoke old worker authority before its load can start"
        );

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: stale_correlation,
                execution: Box::new(sample_post_turn_execution()),
            },
        ));
        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());
    }

    #[test]
    fn load_conversation_marks_conversation_loading() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-1".to_string(),
            fallback_workspace_directory: "/tmp/root".to_string(),
        }));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(outcome.snapshot.conversation, ConversationSnapshot::Loading);
        assert_eq!(
            outcome.events,
            vec![AppEvent::ConversationChanged {
                correlation: Some(conversation_load_correlation(1, "thread-1")),
                snapshot: ConversationSnapshot::Loading,
            }]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadConversation {
                correlation: conversation_load_correlation(1, "thread-1"),
                fallback_workspace_directory: "/tmp/root".to_string(),
            }]
        );
    }

    #[test]
    fn parallel_peek_load_dispatches_correlated_effect_without_replacing_active_conversation() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::Command(
            AppCommand::LoadParallelPeekConversation {
                thread_id: "thread-peek".to_string(),
            },
        ));

        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
        assert!(outcome.events.is_empty());
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadParallelPeekConversation {
                correlation: parallel_peek_load_correlation(1, "thread-peek"),
            }]
        );
    }

    #[test]
    fn newer_parallel_peek_load_supersedes_the_active_correlation() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(
            AppCommand::LoadParallelPeekConversation {
                thread_id: "thread-old".to_string(),
            },
        ));

        let outcome = controller.handle_input(CoreInput::Command(
            AppCommand::LoadParallelPeekConversation {
                thread_id: "thread-new".to_string(),
            },
        ));

        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadParallelPeekConversation {
                correlation: parallel_peek_load_correlation(2, "thread-new"),
            }]
        );
    }

    #[test]
    fn newer_review_center_load_supersedes_the_active_correlation() {
        let mut controller = CoreController::new();
        let first = controller.handle_input(CoreInput::Command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/old".to_string(),
            active_thread_id: Some("thread-old".to_string()),
        }));
        let second = controller.handle_input(CoreInput::Command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/new".to_string(),
            active_thread_id: Some("thread-new".to_string()),
        }));
        let first_correlation = review_center_load_correlation(1, "/tmp/old", Some("thread-old"));
        let second_correlation = review_center_load_correlation(2, "/tmp/new", Some("thread-new"));

        assert_eq!(
            first.events,
            vec![AppEvent::ReviewCenterLoadStarted {
                correlation: first_correlation.clone(),
            }]
        );
        assert_eq!(
            first.effects,
            vec![CoreEffect::LoadReviewCenter {
                correlation: first_correlation,
            }]
        );
        assert_eq!(
            second.events,
            vec![AppEvent::ReviewCenterLoadStarted {
                correlation: second_correlation.clone(),
            }]
        );
        assert_eq!(
            second.effects,
            vec![CoreEffect::LoadReviewCenter {
                correlation: second_correlation,
            }]
        );
    }

    #[test]
    fn newer_queue_authority_load_supersedes_the_active_correlation() {
        let mut controller = CoreController::new();
        let first = controller.handle_input(CoreInput::Command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/old".to_string(),
            active_thread_id: Some("thread-old".to_string()),
        }));
        let second = controller.handle_input(CoreInput::Command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/new".to_string(),
            active_thread_id: Some("thread-new".to_string()),
        }));
        let first_correlation = queue_authority_load_correlation(1, "/tmp/old", Some("thread-old"));
        let second_correlation =
            queue_authority_load_correlation(2, "/tmp/new", Some("thread-new"));

        assert_eq!(
            first.events,
            vec![AppEvent::QueueAuthorityLoadStarted {
                correlation: first_correlation.clone(),
            }]
        );
        assert_eq!(
            first.effects,
            vec![CoreEffect::LoadQueueAuthority {
                correlation: first_correlation,
            }]
        );
        assert_eq!(
            second.events,
            vec![AppEvent::QueueAuthorityLoadStarted {
                correlation: second_correlation.clone(),
            }]
        );
        assert_eq!(
            second.effects,
            vec![CoreEffect::LoadQueueAuthority {
                correlation: second_correlation,
            }]
        );
    }

    #[test]
    fn directions_maintenance_load_dispatches_a_correlated_effect() {
        let mut controller = CoreController::new();
        let correlation = directions_maintenance_load_correlation(1, "/tmp/workspace");

        let outcome =
            controller.handle_input(CoreInput::Command(AppCommand::LoadDirectionsMaintenance {
                workspace_directory: "/tmp/workspace".to_string(),
            }));

        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            outcome.events,
            vec![AppEvent::DirectionsMaintenanceLoadStarted {
                correlation: correlation.clone(),
            }]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadDirectionsMaintenance { correlation }]
        );
    }

    #[test]
    fn newer_planning_runtime_refresh_supersedes_the_active_correlation() {
        let mut controller = CoreController::new();
        let first =
            controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
                workspace_directory: "/tmp/a".to_string(),
            }));
        let second =
            controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
                workspace_directory: "/tmp/b".to_string(),
            }));
        let first_correlation = planning_runtime_refresh_correlation(1, "/tmp/a");
        let second_correlation = planning_runtime_refresh_correlation(2, "/tmp/b");

        assert_eq!(
            first.events,
            vec![AppEvent::PlanningRuntimeRefreshStarted {
                correlation: first_correlation.clone(),
            }]
        );
        assert_eq!(
            first.effects,
            vec![CoreEffect::LoadPlanningRuntime {
                correlation: first_correlation.clone(),
            }]
        );
        assert_eq!(
            second.events,
            vec![
                AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: second_correlation.clone(),
                },
                AppEvent::PlanningRuntimeRefreshCancelled {
                    correlation: first_correlation,
                },
            ]
        );
        assert_eq!(
            second.effects,
            vec![CoreEffect::LoadPlanningRuntime {
                correlation: second_correlation,
            }]
        );
    }

    #[test]
    fn planning_runtime_refresh_completion_accepts_only_the_latest_generation_once() {
        let mut controller = CoreController::new();
        for workspace_directory in ["/tmp/old", "/tmp/new"] {
            controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
                workspace_directory: workspace_directory.to_string(),
            }));
        }

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: planning_runtime_refresh_correlation(1, "/tmp/old"),
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("stale"),
                )),
            },
        ));
        assert!(stale.events.is_empty());

        let correlation = planning_runtime_refresh_correlation(2, "/tmp/new");
        let projection = PlanningRuntimeProjection::invalid("latest");
        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: correlation.clone(),
                result: Ok(planning_runtime_refresh_snapshot(projection.clone())),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::PlanningRuntimeRefreshed {
                correlation: correlation.clone(),
                result: Ok(planning_doctor_snapshot(&projection)),
            }]
        );
        assert_eq!(
            *accepted.snapshot.planning_parallel.planning_runtime,
            projection
        );

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation,
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("duplicate"),
                )),
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
        assert_eq!(
            *duplicate.snapshot.planning_parallel.planning_runtime,
            PlanningRuntimeProjection::invalid("latest")
        );
    }

    #[test]
    fn planning_runtime_refresh_failure_preserves_projection_and_reports_error() {
        let mut controller = CoreController::new();
        let previous_projection = PlanningRuntimeProjection::ready(
            "current prompt".to_string(),
            "current queue".to_string(),
            None,
        );
        controller.handle_input(runtime_projection_changed(
            "/tmp/workspace",
            previous_projection.clone(),
        ));
        controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/workspace".to_string(),
        }));
        let correlation = planning_runtime_refresh_correlation(1, "/tmp/workspace");

        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: correlation.clone(),
                result: Err("workspace inspection failed".to_string()),
            },
        ));

        assert_eq!(
            failed.events,
            vec![AppEvent::PlanningRuntimeRefreshed {
                correlation,
                result: Err("workspace inspection failed".to_string()),
            }]
        );
        assert_eq!(failed.snapshot.revision, 1);
        assert_eq!(
            failed
                .snapshot
                .planning_parallel
                .planning_runtime_workspace_directory
                .as_deref(),
            Some("/tmp/workspace")
        );
        assert_eq!(
            *failed.snapshot.planning_parallel.planning_runtime,
            previous_projection
        );
    }

    #[test]
    fn planning_runtime_refresh_same_workspace_aba_rejects_the_older_generation() {
        let mut controller = CoreController::new();
        for workspace_directory in ["/tmp/a", "/tmp/b", "/tmp/a"] {
            controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
                workspace_directory: workspace_directory.to_string(),
            }));
        }

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: planning_runtime_refresh_correlation(1, "/tmp/a"),
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("old a"),
                )),
            },
        ));
        assert!(stale.events.is_empty());

        let latest = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: planning_runtime_refresh_correlation(3, "/tmp/a"),
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("new a"),
                )),
            },
        ));
        assert_eq!(
            latest.events,
            vec![AppEvent::PlanningRuntimeRefreshed {
                correlation: planning_runtime_refresh_correlation(3, "/tmp/a"),
                result: Ok(planning_doctor_snapshot(
                    &PlanningRuntimeProjection::invalid("new a",)
                )),
            }]
        );
    }

    #[test]
    fn projection_writer_cannot_complete_an_active_refresh_operation() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/workspace".to_string(),
        }));
        let accepted_projection = PlanningRuntimeProjection::invalid("new authority");

        let writer = controller.handle_input(runtime_projection_changed(
            "/tmp/workspace",
            accepted_projection.clone(),
        ));

        assert!(
            !writer
                .events
                .iter()
                .any(|event| matches!(event, AppEvent::PlanningRuntimeRefreshed { .. }))
        );
        assert!(
            writer
                .events
                .contains(&AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: planning_runtime_refresh_correlation(2, "/tmp/workspace"),
                })
        );
        assert!(
            writer
                .events
                .contains(&AppEvent::PlanningRuntimeRefreshCancelled {
                    correlation: planning_runtime_refresh_correlation(1, "/tmp/workspace"),
                })
        );
        assert_eq!(
            writer.effects,
            vec![CoreEffect::LoadPlanningRuntime {
                correlation: planning_runtime_refresh_correlation(2, "/tmp/workspace"),
            }]
        );
        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: planning_runtime_refresh_correlation(1, "/tmp/workspace"),
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("stale read"),
                )),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(
            *stale.snapshot.planning_parallel.planning_runtime,
            accepted_projection
        );
        let replacement = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: planning_runtime_refresh_correlation(2, "/tmp/workspace"),
                result: Ok(planning_runtime_refresh_snapshot(
                    accepted_projection.clone(),
                )),
            },
        ));
        assert_eq!(
            replacement.events,
            vec![AppEvent::PlanningRuntimeRefreshed {
                correlation: planning_runtime_refresh_correlation(2, "/tmp/workspace"),
                result: Ok(planning_doctor_snapshot(&accepted_projection)),
            }]
        );
    }

    #[test]
    fn projection_writer_for_another_workspace_does_not_settle_an_active_refresh() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/root".to_string(),
        }));

        let ignored = controller.handle_input(runtime_projection_changed(
            "/tmp/slot",
            PlanningRuntimeProjection::invalid("slot projection"),
        ));

        assert!(ignored.events.is_empty());
        assert_eq!(*ignored.snapshot, AppSnapshot::initial());

        let correlation = planning_runtime_refresh_correlation(1, "/tmp/root");
        let loaded = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: correlation.clone(),
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("root projection"),
                )),
            },
        ));
        assert_eq!(
            loaded.events,
            vec![AppEvent::PlanningRuntimeRefreshed {
                correlation,
                result: Ok(planning_doctor_snapshot(
                    &PlanningRuntimeProjection::invalid("root projection",)
                )),
            }]
        );
        assert_eq!(
            loaded
                .snapshot
                .planning_parallel
                .planning_runtime_workspace_directory
                .as_deref(),
            Some("/tmp/root")
        );
    }

    #[test]
    fn conversation_transitions_cancel_an_active_planning_runtime_refresh() {
        let mut load_controller = CoreController::new();
        load_controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/a".to_string(),
        }));
        let load = load_controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-b".to_string(),
            fallback_workspace_directory: "/tmp/b".to_string(),
        }));
        assert!(matches!(
            load.events.first(),
            Some(AppEvent::PlanningRuntimeRefreshCancelled { correlation })
                if correlation == &planning_runtime_refresh_correlation(1, "/tmp/a")
        ));

        let mut invalidate_controller = CoreController::new();
        invalidate_controller.handle_input(CoreInput::Command(
            AppCommand::RefreshPlanningRuntime {
                workspace_directory: "/tmp/a".to_string(),
            },
        ));
        let invalidated = invalidate_controller
            .handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        assert!(matches!(
            invalidated.events.first(),
            Some(AppEvent::PlanningRuntimeRefreshCancelled { correlation })
                if correlation == &planning_runtime_refresh_correlation(1, "/tmp/a")
        ));

        for controller in [&mut load_controller, &mut invalidate_controller] {
            let stale = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::PlanningRuntimeLoaded {
                    correlation: planning_runtime_refresh_correlation(1, "/tmp/a"),
                    result: Ok(planning_runtime_refresh_snapshot(
                        PlanningRuntimeProjection::invalid("stale"),
                    )),
                },
            ));
            assert!(stale.events.is_empty());
        }
    }

    #[test]
    fn turn_workspace_change_does_not_cancel_the_conversation_planning_refresh() {
        let mut controller = CoreController::new();
        let turn = controller.begin_test_turn_submission();
        controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/conversation-root".to_string(),
        }));

        let changed = controller.handle_input(CoreInput::ConversationTurnWorkspaceChanged {
            correlation: turn,
            workspace_directory: "/tmp/parallel-slot".to_string(),
        });
        assert!(matches!(
            changed.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                AppEvent::ConversationTurnWorkspaceChanged {
                    workspace_directory,
                },
            ] if runtime
                .active_turn
                .as_ref()
                .is_some_and(|turn| turn.workspace_directory == "/tmp/parallel-slot")
                && workspace_directory == "/tmp/parallel-slot"
        ));

        let completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: planning_runtime_refresh_correlation(1, "/tmp/conversation-root"),
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("loaded root"),
                )),
            },
        ));
        assert_eq!(
            completion.events,
            vec![AppEvent::PlanningRuntimeRefreshed {
                correlation: planning_runtime_refresh_correlation(1, "/tmp/conversation-root"),
                result: Ok(planning_doctor_snapshot(
                    &PlanningRuntimeProjection::invalid("loaded root",)
                )),
            }]
        );
    }

    #[test]
    #[should_panic(expected = "planning runtime refresh generation exhausted")]
    fn planning_runtime_refresh_generation_overflow_fails_closed() {
        let mut controller = CoreController::new();
        controller.planning.exhaust_runtime_refresh_generation();

        controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/workspace".to_string(),
        }));
    }

    #[test]
    fn queue_mutation_is_single_flight_and_failure_reopens_the_gate() {
        let mut controller = CoreController::new();
        let first_intent = queue_mutation_intent(
            "/tmp/workspace",
            Some("thread-1"),
            QueueMutationKind::RemoveSelected,
        );
        let second_intent = queue_mutation_intent(
            "/tmp/workspace",
            Some("thread-1"),
            QueueMutationKind::UndoLatestRegistration,
        );
        let first_correlation = QueueMutationCorrelation::new(1, first_intent.clone());

        let first = controller.handle_input(CoreInput::Command(AppCommand::SubmitQueueMutation(
            Box::new(first_intent),
        )));
        assert_eq!(
            first.events,
            vec![AppEvent::QueueMutationStarted {
                correlation: first_correlation.clone(),
            }]
        );
        assert_eq!(
            first.effects,
            vec![CoreEffect::ExecuteQueueMutation {
                correlation: first_correlation.clone(),
            }]
        );

        let blocked = controller.handle_input(CoreInput::Command(AppCommand::SubmitQueueMutation(
            Box::new(second_intent.clone()),
        )));
        assert!(blocked.events.is_empty());
        assert!(blocked.effects.is_empty());

        let failed_result = Box::new(QueueMutationResult {
            mutation: Err("mutation failed".to_string()),
            authority: Err(QueueAuthorityLoadError::AuthorityUnavailable(
                "refresh failed".to_string(),
            )),
        });
        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueMutationCompleted {
                correlation: first_correlation.clone(),
                result: failed_result.clone(),
            },
        ));
        assert_eq!(
            failed.events,
            vec![AppEvent::QueueMutationCompleted {
                correlation: first_correlation,
                result: failed_result,
            }]
        );

        let retry = controller.handle_input(CoreInput::Command(AppCommand::SubmitQueueMutation(
            Box::new(second_intent.clone()),
        )));
        let retry_correlation = QueueMutationCorrelation::new(2, second_intent);
        assert_eq!(
            retry.events,
            vec![AppEvent::QueueMutationStarted {
                correlation: retry_correlation.clone(),
            }]
        );
        assert_eq!(
            retry.effects,
            vec![CoreEffect::ExecuteQueueMutation {
                correlation: retry_correlation,
            }]
        );
    }

    #[test]
    #[should_panic(expected = "queue mutation generation exhausted")]
    fn queue_mutation_generation_panics_before_it_can_wrap() {
        let mut controller = CoreController::new();
        controller.planning.exhaust_queue_mutation_generation();

        controller.handle_input(CoreInput::Command(AppCommand::SubmitQueueMutation(
            Box::new(queue_mutation_intent(
                "/tmp/workspace",
                Some("thread-1"),
                QueueMutationKind::RemoveSelected,
            )),
        )));
    }

    #[test]
    fn submit_turn_returns_core_effect_and_publishes_runtime_authority() {
        let mut controller = CoreController::new();
        let request = crate::core::app::TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            image_paths: Vec::new(),
            prompt: "ship it".to_string(),
            prompt_origin: crate::core::app::CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        };

        let outcome = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(request.clone()),
        )));

        assert!(matches!(
            outcome.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                AppEvent::TurnSubmissionAdmissionResolved(
                    TurnSubmissionAdmission::Accepted {
                        correlation: TurnSubmissionCorrelation { generation: 1 },
                    },
                ),
            ] if runtime.active_turn.as_ref().is_some_and(|turn| {
                turn.correlation == TurnSubmissionCorrelation::new(1)
                    && turn.workspace_directory == "/tmp/workspace"
                    && turn.prompt_origin == CorePromptOrigin::Manual
            })
        ));
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::SubmitTurn {
                correlation: TurnSubmissionCorrelation::new(1),
                request: Box::new(request),
            }]
        );
        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            outcome
                .snapshot
                .conversation_runtime
                .active_turn
                .as_ref()
                .map(|turn| turn.correlation),
            Some(TurnSubmissionCorrelation::new(1))
        );
    }

    #[test]
    fn active_turn_submission_rejects_a_second_submit_effect() {
        let mut controller = CoreController::new();
        let first = test_turn_submission_request(Some("thread-1"));
        let second = test_turn_submission_request(Some("thread-1"));

        let first_outcome =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(Box::new(first))));
        let second_outcome =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(Box::new(second))));

        assert_eq!(first_outcome.effects.len(), 1);
        assert!(matches!(
            first_outcome.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(_),
                AppEvent::TurnSubmissionAdmissionResolved(TurnSubmissionAdmission::Accepted {
                    correlation: TurnSubmissionCorrelation { generation: 1 },
                },),
            ]
        ));
        assert_eq!(
            second_outcome.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedActive {
                    active_correlation: TurnSubmissionCorrelation::new(1),
                },
            )]
        );
        assert!(second_outcome.effects.is_empty());
    }

    #[test]
    fn idle_stop_request_is_single_flight_and_failure_reopens_the_gate() {
        let mut controller = CoreController::new();
        let correlation = StopRequestCorrelation::new(1, None);

        let accepted =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        assert!(matches!(
            accepted.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                AppEvent::StopRequestAdmissionResolved(StopRequestAdmission::Accepted {
                    correlation: actual,
                }),
            ] if runtime.auto_follow.continuation_paused
                && !runtime.auto_follow.parallel_rearmed_after_stop
                && *actual == correlation
        ));
        assert_eq!(
            accepted.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation,
                attempt: StopRequestAttempt::Initial,
            }]
        );

        let duplicate =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        assert_eq!(
            duplicate.events,
            vec![AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::RejectedActive {
                    active_correlation: correlation,
                },
            )]
        );
        assert!(duplicate.effects.is_empty());

        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt: StopRequestAttempt::Initial,
                result: Err("runtime unavailable".to_string()),
            },
        ));
        assert_eq!(
            failed.events,
            vec![AppEvent::StopRequestAttemptCompleted {
                correlation,
                attempt: StopRequestAttempt::Initial,
                result: Err("runtime unavailable".to_string()),
            }]
        );

        let retry = controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        let retry_correlation = StopRequestCorrelation::new(2, None);
        assert_eq!(
            retry.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation: retry_correlation,
                attempt: StopRequestAttempt::Initial,
            }]
        );

        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation: retry_correlation,
                attempt: StopRequestAttempt::Initial,
                result: Ok(()),
            },
        ));
        let repeated =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        assert_eq!(
            repeated.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation: StopRequestCorrelation::new(3, None),
                attempt: StopRequestAttempt::Initial,
            }]
        );
    }

    #[test]
    fn stop_command_atomically_disarms_all_post_turn_continuation_paths() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::SetAutoFollowMaxTurns {
            value: 3,
        }));
        controller.handle_input(CoreInput::Command(AppCommand::PausePostTurnContinuation));
        controller.handle_input(CoreInput::Command(AppCommand::SetParallelPostTurnRearm {
            rearmed: true,
        }));

        let outcome =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));

        assert!(
            outcome
                .snapshot
                .conversation_runtime
                .auto_follow
                .continuation_paused
        );
        assert!(
            !outcome
                .snapshot
                .conversation_runtime
                .auto_follow
                .parallel_rearmed_after_stop
        );
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            AppEvent::ConversationRuntimeAuthorityChanged(runtime)
                if runtime.auto_follow.continuation_paused
                    && !runtime.auto_follow.parallel_rearmed_after_stop
        )));
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            AppEvent::StopRequestAdmissionResolved(StopRequestAdmission::Accepted { .. })
        )));
    }

    #[test]
    fn pre_start_stop_synchronizes_once_and_fail_closed_stream_error_keeps_the_gate() {
        let mut controller = CoreController::new();
        let turn_submission = submit_test_turn(&mut controller, Some("thread-1"));
        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Core runtime".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        let correlation = StopRequestCorrelation::new(1, Some(turn_submission));

        let requested =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        assert_eq!(
            requested.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation,
                attempt: StopRequestAttempt::Initial,
            }]
        );

        let started = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        ));
        assert!(started.effects.is_empty());

        let initial_completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt: StopRequestAttempt::Initial,
                result: Ok(()),
            },
        ));
        assert_eq!(
            initial_completed.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation,
                attempt: StopRequestAttempt::AfterTurnStarted,
            }]
        );

        let duplicate_started = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        ));
        assert!(duplicate_started.effects.is_empty());

        let synchronized = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt: StopRequestAttempt::AfterTurnStarted,
                result: Ok(()),
            },
        ));
        assert_eq!(
            synchronized.events,
            vec![AppEvent::StopRequestAttemptCompleted {
                correlation,
                attempt: StopRequestAttempt::AfterTurnStarted,
                result: Ok(()),
            }]
        );

        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::TurnInterruptRequestFailed {
                message: "interrupt retries exhausted".to_string(),
            },
        ));
        let still_blocked =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        assert!(matches!(
            still_blocked.events.as_slice(),
            [AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::RejectedActive {
                    active_correlation,
                }
            )] if *active_correlation == correlation
        ));

        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::TurnRetrying {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                error: ConversationTurnError::new("retrying", None::<&str>, None),
            },
        ));
        let retried =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        let retry_correlation = StopRequestCorrelation::new(2, Some(turn_submission));
        assert_eq!(
            retried.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation: retry_correlation,
                attempt: StopRequestAttempt::Initial,
            }]
        );

        let retry_initial_completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation: retry_correlation,
                attempt: StopRequestAttempt::Initial,
                result: Ok(()),
            },
        ));
        assert!(retry_initial_completed.effects.is_empty());

        let duplicate_started = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        ));
        assert!(duplicate_started.effects.is_empty());
    }

    #[test]
    fn post_start_stop_does_not_resynchronize_and_stale_completion_cannot_cross_aba() {
        let mut controller = CoreController::new();
        let first_turn = start_test_turn(&mut controller, "thread-1", "turn-same");
        let first_stop = StopRequestCorrelation::new(1, Some(first_turn));
        controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        let first_completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation: first_stop,
                attempt: StopRequestAttempt::Initial,
                result: Ok(()),
            },
        ));
        assert!(first_completed.effects.is_empty());

        controller.handle_input(test_turn_stream_input(
            first_turn,
            TurnStreamEvent::Failed {
                message: "first turn stopped".to_string(),
            },
        ));
        let second_turn = start_test_turn(&mut controller, "thread-1", "turn-same");
        let second_stop = StopRequestCorrelation::new(2, Some(second_turn));
        controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation: first_stop,
                attempt: StopRequestAttempt::Initial,
                result: Err("late first completion".to_string()),
            },
        ));
        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());

        let duplicate =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        assert_eq!(
            duplicate.events,
            vec![AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::RejectedActive {
                    active_correlation: second_stop,
                },
            )]
        );
    }

    #[test]
    #[should_panic(expected = "runtime stop request generation exhausted")]
    fn stop_request_generation_panics_before_it_can_wrap() {
        let mut controller = CoreController::new();
        controller
            .conversation_turn
            .exhaust_stop_generation_for_test();

        controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
    }

    #[test]
    fn turn_steer_is_admitted_once_for_the_exact_active_turn() {
        let mut controller = CoreController::new();
        let request = ConversationTurnSteerRequest {
            thread_id: "thread-1".to_string(),
            expected_turn_id: "turn-1".to_string(),
            prompt: "focus the active work".to_string(),
        };
        let unavailable =
            controller.handle_input(CoreInput::Command(AppCommand::SteerTurn(request.clone())));
        assert_eq!(
            unavailable.events,
            vec![AppEvent::TurnSteerAdmissionResolved(
                TurnSteerAdmission::RejectedUnavailable,
            )]
        );
        assert!(unavailable.effects.is_empty());

        let turn_submission = start_test_turn(&mut controller, "thread-1", "turn-1");
        let mismatched = controller.handle_input(CoreInput::Command(AppCommand::SteerTurn(
            ConversationTurnSteerRequest {
                expected_turn_id: "turn-other".to_string(),
                ..request.clone()
            },
        )));
        assert_eq!(
            mismatched.events,
            vec![AppEvent::TurnSteerAdmissionResolved(
                TurnSteerAdmission::RejectedUnavailable,
            )]
        );
        assert!(mismatched.effects.is_empty());

        let accepted =
            controller.handle_input(CoreInput::Command(AppCommand::SteerTurn(request.clone())));
        let correlation = TurnSteerCorrelation::new(1, turn_submission);
        assert_eq!(
            accepted.events,
            vec![AppEvent::TurnSteerAdmissionResolved(
                TurnSteerAdmission::Accepted { correlation },
            )]
        );
        assert_eq!(
            accepted.effects,
            vec![CoreEffect::SteerTurn {
                correlation,
                request: request.clone(),
            }]
        );

        let duplicate = controller.handle_input(CoreInput::Command(AppCommand::SteerTurn(request)));
        assert_eq!(
            duplicate.events,
            vec![AppEvent::TurnSteerAdmissionResolved(
                TurnSteerAdmission::RejectedActive {
                    active_correlation: correlation,
                },
            )]
        );
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn turn_steer_completion_is_generation_guarded_across_terminal_and_invalidation() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-1", "turn-1");
        let request = ConversationTurnSteerRequest {
            thread_id: "thread-1".to_string(),
            expected_turn_id: "turn-1".to_string(),
            prompt: "focus the active work".to_string(),
        };
        controller.handle_input(CoreInput::Command(AppCommand::SteerTurn(request.clone())));
        let correlation = TurnSteerCorrelation::new(1, turn_submission);

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::TurnSteered {
                correlation: TurnSteerCorrelation::new(2, turn_submission),
                result: Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                    turn_id: "turn-1".to_string(),
                }),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(
            controller.conversation_turn.active_turn_steer_for_test(),
            Some(correlation)
        );

        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::TurnTerminal {
                receipt: confirmed_terminal_receipt("thread-1", "turn-1", Vec::new()),
                execution_snapshot_capture: None,
            },
        ));
        let post_turn = start_post_turn_evaluation(&mut controller, "turn-1");
        let post_turn_correlation = post_turn_effect_correlation(&post_turn);
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: post_turn_correlation.clone(),
                execution: Box::new(sample_post_turn_execution()),
            },
        ));
        controller.handle_input(CoreInput::Command(
            AppCommand::ResolvePostTurnContinuation {
                correlation: post_turn_correlation,
                resolution: PostTurnRouteResolution::NoContinuation,
            },
        ));
        let next_submit = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(test_turn_submission_request(Some("thread-1"))),
        )));
        assert!(matches!(
            next_submit.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(_),
                AppEvent::TurnSubmissionAdmissionResolved(TurnSubmissionAdmission::Accepted {
                    correlation: TurnSubmissionCorrelation { generation: 2 },
                }),
            ]
        ));
        assert!(matches!(
            next_submit.effects.as_slice(),
            [CoreEffect::SubmitTurn { correlation, .. }]
                if *correlation == TurnSubmissionCorrelation::new(2)
        ));

        let completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::TurnSteered {
                correlation,
                result: Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                    turn_id: "turn-1".to_string(),
                }),
            },
        ));
        assert!(matches!(
            completed.events.as_slice(),
            [AppEvent::TurnSteerCompleted {
                correlation: completed_correlation,
                result: Ok(receipt),
            }] if *completed_correlation == correlation && receipt.turn_id == "turn-1"
        ));
        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::TurnSteered {
                correlation,
                result: Err("late duplicate".to_string()),
            },
        ));
        assert!(duplicate.events.is_empty());

        let next_turn = TurnSubmissionCorrelation::new(2);
        controller.handle_input(test_turn_stream_input(
            next_turn,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Core runtime".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        controller.handle_input(test_turn_stream_input(
            next_turn,
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-2".to_string(),
                runtime_request: Box::default(),
            },
        ));
        let next_request = ConversationTurnSteerRequest {
            expected_turn_id: "turn-2".to_string(),
            ..request
        };
        controller.handle_input(CoreInput::Command(AppCommand::SteerTurn(next_request)));
        let next_correlation = TurnSteerCorrelation::new(2, next_turn);
        controller.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        let late = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::TurnSteered {
                correlation: next_correlation,
                result: Err("late completion".to_string()),
            },
        ));
        assert!(late.events.is_empty());
        assert!(
            controller
                .conversation_turn
                .active_turn_steer_for_test()
                .is_none()
        );
    }

    #[test]
    fn approval_decision_is_single_flight_until_the_exact_request_resolves() {
        let mut controller = CoreController::new();
        let unavailable =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                request_identity: approval_identity("approval-1"),
                decision: ConversationApprovalDecision::Accept,
            }));
        assert_eq!(
            unavailable.events,
            vec![AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedUnavailable,
            )]
        );

        let turn_submission = start_test_turn(&mut controller, "thread-1", "turn-1");
        request_test_approval(&mut controller, turn_submission, "approval-1");
        let mismatched =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                request_identity: approval_identity("approval-other"),
                decision: ConversationApprovalDecision::Accept,
            }));
        assert_eq!(
            mismatched.events,
            vec![AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedUnavailable,
            )]
        );

        let correlation = ApprovalDecisionCorrelation::new(
            1,
            turn_submission,
            approval_identity("approval-1"),
            ConversationApprovalDecision::Accept,
        );
        let accepted =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                request_identity: approval_identity("approval-1"),
                decision: ConversationApprovalDecision::Accept,
            }));
        assert!(matches!(
            accepted.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                AppEvent::ApprovalDecisionAdmissionResolved(
                    ApprovalDecisionAdmission::Accepted {
                        correlation: accepted,
                    },
                ),
            ] if runtime
                .approval
                .as_ref()
                .is_some_and(|approval| approval.phase == ApprovalAuthorityPhase::Submitting)
                && accepted == &correlation
        ));
        assert_eq!(
            accepted.effects,
            vec![CoreEffect::SubmitApprovalDecision {
                correlation: correlation.clone(),
            }]
        );

        let duplicate =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                request_identity: approval_identity("approval-1"),
                decision: ConversationApprovalDecision::Decline,
            }));
        assert_eq!(
            duplicate.events,
            vec![AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedActive {
                    active_correlation: correlation.clone(),
                },
            )]
        );

        let completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: correlation.clone(),
                result: Ok(()),
            },
        ));
        assert!(matches!(
            completed.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                AppEvent::ApprovalDecisionSubmissionCompleted {
                    correlation: completed,
                    result: Ok(()),
                },
            ] if runtime
                .approval
                .as_ref()
                .is_some_and(|approval| approval.phase == ApprovalAuthorityPhase::Submitted)
                && completed == &correlation
        ));
        assert!(
            controller
                .conversation_turn
                .approval_decision_is_submitted_for_test()
        );

        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalResolved {
                request_identity: approval_identity("approval-stale"),
                resolution: ConversationApprovalResolution::Declined,
            },
        ));
        let waiting =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                request_identity: approval_identity("approval-1"),
                decision: ConversationApprovalDecision::Accept,
            }));
        assert!(matches!(
            waiting.events.as_slice(),
            [AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedActive { .. }
            )]
        ));

        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalResolved {
                request_identity: approval_identity("approval-1"),
                resolution: ConversationApprovalResolution::Accepted,
            },
        ));
        assert!(
            controller
                .conversation_turn
                .active_approval_decision_for_test()
                .is_none()
        );
        let after_resolution =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                request_identity: approval_identity("approval-1"),
                decision: ConversationApprovalDecision::Accept,
            }));
        assert_eq!(
            after_resolution.events,
            vec![AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedUnavailable,
            )]
        );
        let duplicate_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation,
                result: Err("late duplicate".to_string()),
            },
        ));
        assert!(duplicate_completion.events.is_empty());
    }

    #[test]
    fn failed_approval_submission_allows_a_generation_checked_retry() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-1", "turn-1");
        request_test_approval(&mut controller, turn_submission, "approval-1");
        let first = ApprovalDecisionCorrelation::new(
            1,
            turn_submission,
            approval_identity("approval-1"),
            ConversationApprovalDecision::Accept,
        );
        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            request_identity: approval_identity("approval-1"),
            decision: ConversationApprovalDecision::Accept,
        }));

        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: first,
                result: Err("runtime unavailable".to_string()),
            },
        ));
        assert!(matches!(
            failed.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                AppEvent::ApprovalDecisionSubmissionCompleted {
                    result: Err(message),
                    ..
                },
            ] if runtime
                .approval
                .as_ref()
                .is_some_and(|approval| approval.phase == ApprovalAuthorityPhase::Pending)
                && message == "runtime unavailable"
        ));
        assert!(
            controller
                .conversation_turn
                .active_approval_decision_for_test()
                .is_none()
        );

        let retried =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                request_identity: approval_identity("approval-1"),
                decision: ConversationApprovalDecision::Decline,
            }));
        assert!(matches!(
            retried.effects.as_slice(),
            [CoreEffect::SubmitApprovalDecision { correlation }]
                if correlation.generation == 2
                    && correlation.turn_submission == turn_submission
                    && correlation.request_identity == approval_identity("approval-1")
                    && correlation.decision == ConversationApprovalDecision::Decline
        ));
    }

    #[test]
    fn approval_decision_same_identity_aba_drops_the_older_completion() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-1", "turn-1");
        request_test_approval(&mut controller, turn_submission, "approval-a");
        let old = ApprovalDecisionCorrelation::new(
            1,
            turn_submission,
            approval_identity("approval-a"),
            ConversationApprovalDecision::Accept,
        );
        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            request_identity: approval_identity("approval-a"),
            decision: ConversationApprovalDecision::Accept,
        }));

        request_test_approval(&mut controller, turn_submission, "approval-b");
        assert!(
            controller
                .conversation_turn
                .active_approval_decision_for_test()
                .is_none()
        );
        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            request_identity: approval_identity("approval-b"),
            decision: ConversationApprovalDecision::Accept,
        }));
        request_test_approval(&mut controller, turn_submission, "approval-a");
        assert!(
            controller
                .conversation_turn
                .active_approval_decision_for_test()
                .is_none()
        );
        let current = ApprovalDecisionCorrelation::new(
            3,
            turn_submission,
            approval_identity("approval-a"),
            ConversationApprovalDecision::Accept,
        );
        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            request_identity: approval_identity("approval-a"),
            decision: ConversationApprovalDecision::Accept,
        }));

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: old,
                result: Ok(()),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(
            controller
                .conversation_turn
                .active_approval_decision_for_test(),
            Some(&current)
        );

        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: current.clone(),
                result: Ok(()),
            },
        ));
        assert!(matches!(
            accepted.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                AppEvent::ApprovalDecisionSubmissionCompleted {
                    correlation,
                    result: Ok(()),
                },
            ] if runtime
                .approval
                .as_ref()
                .is_some_and(|approval| approval.phase == ApprovalAuthorityPhase::Submitted)
                && correlation == &current
        ));
    }

    #[test]
    fn terminal_and_conversation_invalidation_drop_approval_completions() {
        let mut terminal = CoreController::new();
        let terminal_turn = start_test_turn(&mut terminal, "thread-1", "turn-1");
        request_test_approval(&mut terminal, terminal_turn, "approval-terminal");
        let terminal_correlation = ApprovalDecisionCorrelation::new(
            1,
            terminal_turn,
            approval_identity("approval-terminal"),
            ConversationApprovalDecision::Accept,
        );
        terminal.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            request_identity: approval_identity("approval-terminal"),
            decision: ConversationApprovalDecision::Accept,
        }));
        terminal.handle_input(test_turn_stream_input(
            terminal_turn,
            TurnStreamEvent::TurnTerminal {
                receipt: confirmed_terminal_receipt("thread-1", "turn-1", Vec::new()),
                execution_snapshot_capture: None,
            },
        ));
        let after_terminal = terminal.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: terminal_correlation,
                result: Ok(()),
            },
        ));
        assert!(after_terminal.events.is_empty());
        assert!(
            terminal
                .conversation_turn
                .active_approval_decision_for_test()
                .is_none()
        );

        let mut invalidated = CoreController::new();
        let invalidated_turn = start_test_turn(&mut invalidated, "thread-1", "turn-1");
        request_test_approval(&mut invalidated, invalidated_turn, "approval-invalidated");
        let invalidated_correlation = ApprovalDecisionCorrelation::new(
            1,
            invalidated_turn,
            approval_identity("approval-invalidated"),
            ConversationApprovalDecision::Decline,
        );
        invalidated.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            request_identity: approval_identity("approval-invalidated"),
            decision: ConversationApprovalDecision::Decline,
        }));
        invalidated.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        let after_invalidation = invalidated.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: invalidated_correlation,
                result: Err("late".to_string()),
            },
        ));
        assert!(after_invalidation.events.is_empty());
        assert!(
            invalidated
                .conversation_turn
                .active_approval_decision_for_test()
                .is_none()
        );
    }

    #[test]
    fn conversation_preference_persistence_is_single_writer_and_reports_each_settlement() {
        let mut controller = CoreController::new();
        let first_request = conversation_preference_request("gpt-5.6-sol", "thread-a");
        let second_request = conversation_preference_request("gpt-5.6-terra", "thread-b");

        let first = controller.handle_input(CoreInput::Command(
            AppCommand::PersistConversationPreferences(Box::new(first_request.clone())),
        ));
        let [
            CoreEffect::PersistConversationPreferences {
                correlation: first_correlation,
            },
        ] = first.effects.as_slice()
        else {
            panic!("first preference selection should start a writer");
        };
        let first_correlation = first_correlation.clone();
        assert_eq!(first_correlation.generation, 1);
        assert_eq!(first_correlation.request, first_request);

        let queued = controller.handle_input(CoreInput::Command(
            AppCommand::PersistConversationPreferences(Box::new(second_request.clone())),
        ));
        assert!(queued.events.is_empty());
        assert!(queued.effects.is_empty());

        let first_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationPreferencesPersisted {
                correlation: first_correlation.clone(),
                result: crate::core::app::ConversationPreferencePersistenceResult {
                    global: Ok("/tmp/.akra/config.toml".to_string()),
                    current_thread: Some(Ok(())),
                },
            },
        ));
        assert!(matches!(
            first_completion.events.as_slice(),
            [AppEvent::ConversationPreferencesPersisted {
                correlation,
                result,
            }] if correlation == &first_correlation
                && result.global.is_ok()
                && matches!(result.current_thread, Some(Ok(())))
        ));
        let [
            CoreEffect::PersistConversationPreferences {
                correlation: second_correlation,
            },
        ] = first_completion.effects.as_slice()
        else {
            panic!("the next preference selection should start after completion");
        };
        let second_correlation = second_correlation.clone();
        assert_eq!(second_correlation.generation, 2);
        assert_eq!(second_correlation.request, second_request);

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationPreferencesPersisted {
                correlation: first_correlation,
                result: crate::core::app::ConversationPreferencePersistenceResult {
                    global: Err("stale".to_string()),
                    current_thread: None,
                },
            },
        ));
        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());

        let final_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationPreferencesPersisted {
                correlation: second_correlation.clone(),
                result: crate::core::app::ConversationPreferencePersistenceResult {
                    global: Err("global unavailable".to_string()),
                    current_thread: Some(Err("thread unavailable".to_string())),
                },
            },
        ));
        assert!(matches!(
            final_completion.events.as_slice(),
            [AppEvent::ConversationPreferencesPersisted {
                correlation,
                result,
            }] if correlation == &second_correlation
                && result.global == Err("global unavailable".to_string())
                && result.current_thread == Some(Err("thread unavailable".to_string()))
        ));
        assert!(final_completion.effects.is_empty());
    }

    #[test]
    fn approval_review_persistence_uses_exact_stream_workspace_and_thread() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-review", "turn-1");
        let review = test_approval_review("tool-1");

        let scheduled = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: review.clone(),
            },
        ));
        let correlation = ApprovalReviewPersistenceCorrelation::new(
            1,
            turn_submission,
            "/tmp/workspace",
            "thread-review",
            review,
        );
        assert_eq!(
            scheduled.effects,
            vec![CoreEffect::PersistApprovalReview {
                correlation: correlation.clone(),
            }]
        );

        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalReviewPersisted {
                correlation,
                result: Err("authority unavailable".to_string()),
            },
        ));
        assert!(matches!(
            failed.events.as_slice(),
            [AppEvent::TurnStreamSnapshotChanged(snapshot)]
                if matches!(
                    &snapshot.update,
                    TurnStreamUpdate::RuntimeNotice { notice }
                        if notice == "review-center persistence failed: authority unavailable"
                )
        ));
    }

    #[test]
    fn approval_review_persistence_coalesces_only_pending_duplicate_updates() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-review", "turn-1");
        let review = test_approval_review("tool-duplicate");

        let first = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: review.clone(),
            },
        ));
        let [
            CoreEffect::PersistApprovalReview {
                correlation: first_correlation,
            },
        ] = first.effects.as_slice()
        else {
            panic!("first review update should start one persistence effect");
        };
        let first_correlation = first_correlation.clone();

        let duplicate = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: review.clone(),
            },
        ));
        assert!(duplicate.effects.is_empty());

        let first_completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalReviewPersisted {
                correlation: first_correlation,
                result: Ok(()),
            },
        ));
        assert!(first_completed.effects.is_empty());

        let repeated_after_completion = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: review.clone(),
            },
        ));
        let [
            CoreEffect::PersistApprovalReview {
                correlation: duplicate_correlation,
            },
        ] = repeated_after_completion.effects.as_slice()
        else {
            panic!("a later duplicate should still reach the idempotent service");
        };
        assert_eq!(duplicate_correlation.generation, 2);
        assert_eq!(duplicate_correlation.turn_submission, turn_submission);
        assert_eq!(duplicate_correlation.workspace_directory, "/tmp/workspace");
        assert_eq!(duplicate_correlation.thread_id, "thread-review");
        assert_eq!(duplicate_correlation.review, review);
    }

    #[test]
    fn approval_review_persistence_error_requires_current_workspace() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-review", "turn-1");
        let scheduled = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: test_approval_review("tool-workspace"),
            },
        ));
        let [CoreEffect::PersistApprovalReview { correlation }] = scheduled.effects.as_slice()
        else {
            panic!("review update should start one persistence effect");
        };
        let correlation = correlation.clone();

        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-review".to_string(),
                title: "Moved workspace".to_string(),
                cwd: "/tmp/other-workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalReviewPersisted {
                correlation,
                result: Err("late workspace failure".to_string()),
            },
        ));

        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());
    }

    #[test]
    fn approval_review_persistence_error_requires_current_turn_submission() {
        let mut controller = CoreController::new();
        let old_turn = start_test_turn(&mut controller, "thread-review", "turn-1");
        let scheduled = controller.handle_input(test_turn_stream_input(
            old_turn,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: test_approval_review("tool-old-turn"),
            },
        ));
        let [CoreEffect::PersistApprovalReview { correlation }] = scheduled.effects.as_slice()
        else {
            panic!("review update should start one persistence effect");
        };
        let correlation = correlation.clone();

        controller.handle_input(test_turn_stream_input(
            old_turn,
            TurnStreamEvent::Failed {
                message: "old turn closed".to_string(),
            },
        ));
        let new_turn = start_test_turn(&mut controller, "thread-review", "turn-2");
        assert_ne!(new_turn, old_turn);
        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalReviewPersisted {
                correlation,
                result: Err("late turn failure".to_string()),
            },
        ));

        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());
    }

    #[test]
    fn stale_approval_review_persistence_error_does_not_reach_a_new_conversation() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-old", "turn-1");
        let scheduled = controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: test_approval_review("tool-old"),
            },
        ));
        let [CoreEffect::PersistApprovalReview { correlation }] = scheduled.effects.as_slice()
        else {
            panic!("review update should start one persistence effect");
        };
        let correlation = correlation.clone();

        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-new".to_string(),
            fallback_workspace_directory: "/tmp/new-workspace".to_string(),
        }));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-new"),
                result: Ok(Box::new(sample_conversation_ready_snapshot_for(
                    "thread-new",
                ))),
            },
        ));
        let new_turn = start_test_turn(&mut controller, "thread-new", "turn-2");
        let queued_new_review = controller.handle_input(test_turn_stream_input(
            new_turn,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: test_approval_review("tool-new"),
            },
        ));
        assert!(queued_new_review.effects.is_empty());

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalReviewPersisted {
                correlation: correlation.clone(),
                result: Err("late authority failure".to_string()),
            },
        ));
        assert!(stale.events.is_empty());
        assert!(matches!(
            stale.effects.as_slice(),
            [CoreEffect::PersistApprovalReview { correlation }]
                if correlation.generation == 2
                    && correlation.turn_submission == new_turn
                    && correlation.thread_id == "thread-new"
                    && correlation.review.target_item_id == "tool-new"
        ));

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalReviewPersisted {
                correlation,
                result: Err("duplicate late failure".to_string()),
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    #[should_panic(expected = "approval decision generation exhausted")]
    fn approval_decision_generation_panics_before_it_can_wrap() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-1", "turn-1");
        request_test_approval(&mut controller, turn_submission, "approval-1");
        controller
            .conversation_turn
            .exhaust_approval_generation_for_test();

        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            request_identity: approval_identity("approval-1"),
            decision: ConversationApprovalDecision::Accept,
        }));
    }

    #[test]
    fn github_review_setup_coalesces_and_completes_before_first_poll() {
        let mut controller = CoreController::new();
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let request = github_review_setup_request("/workspace-a", target.clone());
        let correlation = GithubReviewPollingSetupCorrelation::new(1, "/workspace-a");

        let setup = controller.handle_input(CoreInput::Command(
            AppCommand::SetupGithubReviewPolling(request.clone()),
        ));
        assert_eq!(
            setup.events,
            vec![AppEvent::GithubReviewPollingSetupStarted {
                correlation: correlation.clone(),
            }]
        );
        assert_eq!(
            setup.effects,
            vec![CoreEffect::SetupGithubReviewPolling {
                correlation: correlation.clone(),
                request: request.clone(),
            }]
        );
        let duplicate = controller.handle_input(CoreInput::Command(
            AppCommand::SetupGithubReviewPolling(request),
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());

        let early_poll = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert!(early_poll.events.is_empty());
        assert!(early_poll.effects.is_empty());

        let completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                correlation: correlation.clone(),
                result: Ok(GithubReviewPollingSetupResult::Active {
                    target: target.clone(),
                }),
            },
        ));
        assert!(matches!(
            completion.events.as_slice(),
            [AppEvent::GithubReviewPollingSetupCompleted {
                correlation: completed,
                result: Ok(GithubReviewPollingSetupResult::Active { target: completed_target }),
            }] if completed == &correlation && completed_target == &target
        ));

        let started = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        let poll_correlation = GithubReviewPollCorrelation::new(1, target);
        assert_eq!(
            started.events,
            vec![AppEvent::GithubReviewPollStarted {
                correlation: poll_correlation.clone(),
            }]
        );
        assert_eq!(
            started.effects,
            vec![CoreEffect::PollGithubReview {
                setup_correlation: correlation,
                correlation: poll_correlation,
                previous_state: None,
            }]
        );

        let duplicate = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn github_review_setup_aba_rejects_stale_and_duplicate_completions() {
        let mut controller = CoreController::new();
        let target_a = GithubPullRequestTarget::new("acme/widgets", 42);
        let target_b = GithubPullRequestTarget::new("acme/widgets", 43);
        let a1 = start_github_review_setup(&mut controller, "/workspace-a", target_a.clone());
        let b2 = start_github_review_setup(&mut controller, "/workspace-b", target_b.clone());
        let a3 = start_github_review_setup(&mut controller, "/workspace-a", target_a.clone());
        assert_eq!(a1.generation, 1);
        assert_eq!(b2.generation, 2);
        assert_eq!(a3.generation, 3);

        for (correlation, target) in [(a1, target_a.clone()), (b2, target_b)] {
            let stale = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                    correlation,
                    result: Ok(GithubReviewPollingSetupResult::Active { target }),
                },
            ));
            assert!(stale.events.is_empty());
            assert!(stale.effects.is_empty());
        }
        complete_github_review_setup(&mut controller, a3.clone(), target_a.clone());
        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                correlation: a3.clone(),
                result: Ok(GithubReviewPollingSetupResult::Disabled),
            },
        ));
        assert!(duplicate.events.is_empty());

        let poll = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert!(matches!(
            poll.effects.as_slice(),
            [CoreEffect::PollGithubReview {
                setup_correlation,
                correlation: GithubReviewPollCorrelation { target, .. },
                previous_state: None,
            }] if setup_correlation == &a3 && target == &target_a
        ));
    }

    #[test]
    fn github_review_setup_wrong_or_invalid_results_fail_closed() {
        for (actual, expected_message) in [
            (
                GithubPullRequestTarget::new("other/repository", 7),
                "GitHub review polling setup returned a different target",
            ),
            (
                GithubPullRequestTarget::new("invalid", 0),
                "GitHub review polling setup returned an invalid target",
            ),
        ] {
            let mut controller = CoreController::new();
            let expected = GithubPullRequestTarget::new("acme/widgets", 42);
            let correlation = start_github_review_setup(&mut controller, "/workspace", expected);
            let completed = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                    correlation,
                    result: Ok(GithubReviewPollingSetupResult::Active { target: actual }),
                },
            ));
            assert!(matches!(
                completed.events.as_slice(),
                [AppEvent::GithubReviewPollingSetupCompleted {
                    result: Err(message),
                    ..
                }] if message == expected_message
            ));
            let poll = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
            assert!(poll.events.is_empty());
            assert!(poll.effects.is_empty());
        }
    }

    #[test]
    fn github_review_poll_success_advances_cursor_and_failure_preserves_it() {
        let mut controller = CoreController::new();
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let setup_correlation =
            activate_github_review_setup(&mut controller, "/workspace", target.clone());
        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        let first_correlation = GithubReviewPollCorrelation::new(1, target.clone());
        let first_result = github_review_poll_result(&target, "2026-07-19T10:00:00Z");

        let completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: first_correlation.clone(),
                result: Ok(first_result.clone()),
            },
        ));
        assert_eq!(
            completed.events,
            vec![AppEvent::GithubReviewPollCompleted {
                correlation: first_correlation.clone(),
                result: Ok(first_result.clone()),
            }]
        );
        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: first_correlation.clone(),
                result: Ok(first_result.clone()),
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());

        let second_correlation = GithubReviewPollCorrelation::new(2, target.clone());
        let second = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert_eq!(
            second.effects,
            vec![CoreEffect::PollGithubReview {
                setup_correlation: setup_correlation.clone(),
                correlation: second_correlation.clone(),
                previous_state: Some(first_result.next_state.clone()),
            }]
        );
        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: second_correlation,
                result: Err("GitHub unavailable".to_string()),
            },
        ));
        assert!(matches!(
            failed.events.as_slice(),
            [AppEvent::GithubReviewPollCompleted {
                result: Err(message),
                ..
            }] if message == "GitHub unavailable"
        ));

        let third = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert_eq!(
            third.effects,
            vec![CoreEffect::PollGithubReview {
                setup_correlation,
                correlation: GithubReviewPollCorrelation::new(3, target),
                previous_state: Some(first_result.next_state.clone()),
            }]
        );

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: first_correlation,
                result: Ok(first_result),
            },
        ));
        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());
    }

    #[test]
    fn github_review_poll_rejects_a_wrong_provider_target_without_losing_cursor() {
        let mut controller = CoreController::new();
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let setup_correlation =
            activate_github_review_setup(&mut controller, "/workspace", target.clone());
        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        let previous = github_review_poll_result(&target, "2026-07-19T10:00:00Z");
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: GithubReviewPollCorrelation::new(1, target.clone()),
                result: Ok(previous.clone()),
            },
        ));
        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));

        let wrong_target = GithubPullRequestTarget::new("other/repository", 7);
        let rejected = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: GithubReviewPollCorrelation::new(2, target.clone()),
                result: Ok(github_review_poll_result(
                    &wrong_target,
                    "2026-07-19T10:01:00Z",
                )),
            },
        ));
        assert!(matches!(
            rejected.events.as_slice(),
            [AppEvent::GithubReviewPollCompleted {
                result: Err(message),
                ..
            }] if message == "GitHub review poll provider returned a different target"
        ));

        let retried = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert_eq!(
            retried.effects,
            vec![CoreEffect::PollGithubReview {
                setup_correlation,
                correlation: GithubReviewPollCorrelation::new(3, target),
                previous_state: Some(previous.next_state),
            }]
        );
    }

    #[test]
    fn newer_github_review_setup_invalidates_prior_cursor_target_and_poll() {
        let mut controller = CoreController::new();
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        activate_github_review_setup(&mut controller, "/workspace-a", target.clone());
        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        let first = GithubReviewPollCorrelation::new(1, target.clone());
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: first,
                result: Ok(github_review_poll_result(&target, "2026-07-19T10:00:00Z")),
            },
        ));
        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        let superseded_poll = GithubReviewPollCorrelation::new(2, target.clone());

        let setup_b = start_github_review_setup(&mut controller, "/workspace-b", target.clone());
        let poll_before_setup =
            controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert!(poll_before_setup.effects.is_empty());
        let late_poll = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: superseded_poll,
                result: Ok(github_review_poll_result(&target, "2026-07-19T10:01:00Z")),
            },
        ));
        assert!(late_poll.events.is_empty());

        complete_github_review_setup(&mut controller, setup_b.clone(), target.clone());
        let restarted = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        let current_poll = GithubReviewPollCorrelation::new(3, target.clone());
        assert_eq!(
            restarted.effects,
            vec![CoreEffect::PollGithubReview {
                setup_correlation: setup_b,
                correlation: current_poll.clone(),
                previous_state: None,
            }]
        );

        let setup_c = start_github_review_setup(&mut controller, "/workspace-c", target.clone());
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                correlation: setup_c,
                result: Ok(GithubReviewPollingSetupResult::Disabled),
            },
        ));
        let late_current = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: current_poll,
                result: Ok(github_review_poll_result(&target, "2026-07-19T10:02:00Z")),
            },
        ));
        assert!(late_current.events.is_empty());
        let disabled_poll =
            controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert!(disabled_poll.effects.is_empty());
    }

    #[test]
    #[should_panic(expected = "GitHub review polling setup generation exhausted")]
    fn github_review_setup_generation_panics_before_it_can_wrap() {
        let mut controller = CoreController::new();
        controller.github_review.exhaust_setup_generation();

        controller.handle_input(CoreInput::Command(AppCommand::SetupGithubReviewPolling(
            github_review_setup_request(
                "/workspace",
                GithubPullRequestTarget::new("acme/widgets", 42),
            ),
        )));
    }

    #[test]
    #[should_panic(expected = "GitHub review poll generation exhausted")]
    fn github_review_poll_generation_panics_before_it_can_wrap() {
        let mut controller = CoreController::new();
        activate_github_review_setup(
            &mut controller,
            "/workspace",
            GithubPullRequestTarget::new("acme/widgets", 42),
        );
        controller.github_review.exhaust_poll_generation();

        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
    }

    #[test]
    fn next_submission_recovers_pre_start_failure_and_ignores_stale_worker_inputs() {
        let mut controller = CoreController::new();
        let old_correlation = apply_completed_turn(&mut controller, "thread-1", "turn-1");
        controller.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        let current_correlation = submit_test_turn(&mut controller, Some("thread-1"));

        let stale_notice = controller.handle_input(CoreInput::ConversationTurnRuntimeNotice {
            correlation: old_correlation,
            notice: "stale worker notice".to_string(),
        });
        let stale_failure = controller.handle_input(test_turn_stream_input(
            old_correlation,
            TurnStreamEvent::Failed {
                message: "stale worker failed".to_string(),
            },
        ));
        assert!(stale_notice.events.is_empty());
        assert!(stale_failure.events.is_empty());

        let current_failure = controller.handle_input(test_turn_stream_input(
            current_correlation,
            TurnStreamEvent::Failed {
                message: "resume failed before turn/start".to_string(),
            },
        ));
        assert!(current_failure.events.iter().any(|event| matches!(
            event,
            AppEvent::TurnStreamSnapshotChanged(snapshot)
                if matches!(snapshot.update, TurnStreamUpdate::Failed { .. })
        )));

        let next = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(Box::new(
            test_turn_submission_request(Some("thread-1")),
        ))));
        assert!(matches!(
            next.effects.as_slice(),
            [CoreEffect::SubmitTurn {
                correlation: TurnSubmissionCorrelation { generation: 3 },
                ..
            }]
        ));
    }

    #[test]
    fn unconfirmed_terminal_receipt_stays_recovery_pending_and_closes_submission() {
        let mut controller = CoreController::new();
        let correlation = controller.begin_test_turn_submission();
        controller.handle_input(test_turn_stream_input(
            correlation,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Recovery pending".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        controller.handle_input(test_turn_stream_input(
            correlation,
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        ));
        let receipt = crate::domain::turn_terminal::ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            Vec::new(),
        )
        .with_application_delivery(
            crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Unconfirmed(
                crate::domain::turn_terminal::ConversationTurnApplicationDeliveryFailure::Disconnected,
            ),
        );

        let outcome = controller.handle_input(test_turn_stream_input(
            correlation,
            TurnStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
                execution_snapshot_capture: None,
            },
        ));

        assert!(matches!(
            outcome.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                AppEvent::TurnStreamSnapshotChanged(snapshot),
            ]
                if runtime.active_turn.is_none()
                    && matches!(
                    &snapshot.update,
                    TurnStreamUpdate::TurnTerminal { receipt: projected, status_text, .. }
                        if projected.as_ref() == &receipt && status_text == "turn recovery pending"
                )
        ));
        assert!(
            controller
                .conversation_turn
                .active_turn_submission_for_test()
                .is_none()
        );
    }

    #[test]
    fn prepare_manual_prompt_returns_core_effect_without_state_revision() {
        let mut controller = CoreController::new();
        let intent = ManualPromptPreparationIntent {
            workspace_directory: "/tmp/workspace".to_string(),
            raw_prompt: "ship it".to_string(),
            parent_thread_id: Some("thread-1".to_string()),
            parent_turn_id: Some("turn-1".to_string()),
        };
        let correlation = manual_prompt_correlation(1, "/tmp/workspace");
        let request = ManualPromptRequest {
            correlation: correlation.clone(),
            raw_prompt: intent.raw_prompt.clone(),
            parent_thread_id: intent.parent_thread_id.clone(),
            parent_turn_id: intent.parent_turn_id.clone(),
        };

        let outcome = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(intent),
        )));

        assert_eq!(
            outcome.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted { correlation },
            )]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::PrepareManualPrompt(Box::new(request))]
        );
        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
    }

    #[test]
    fn manual_prompt_preparation_dispatch_and_completion_are_exactly_once() {
        let mut controller = CoreController::new();
        let intent = manual_prompt_intent("/tmp/workspace", "ship it");
        let correlation = manual_prompt_correlation(1, "/tmp/workspace");

        let first = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(intent.clone()),
        )));
        let duplicate = controller.handle_input(CoreInput::Command(
            AppCommand::PrepareManualPrompt(Box::new(intent)),
        ));
        let overlapping = controller.handle_input(CoreInput::Command(
            AppCommand::PrepareManualPrompt(Box::new(manual_prompt_intent("/tmp/other", "other"))),
        ));

        assert_eq!(first.effects.len(), 1);
        assert_eq!(
            first.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted {
                    correlation: correlation.clone(),
                },
            )]
        );
        assert!(duplicate.effects.is_empty());
        assert_eq!(
            duplicate.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::RejectedActive {
                    active_correlation: correlation.clone(),
                },
            )]
        );
        assert!(overlapping.effects.is_empty());
        assert_eq!(
            overlapping.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::RejectedActive {
                    active_correlation: correlation.clone(),
                },
            )]
        );

        let stale_result = Box::new(ManualPromptOutcome::Rejected {
            correlation: manual_prompt_correlation(99, "/tmp/other"),
            transcript_text: "other".to_string(),
            runtime_projection: Box::new(PlanningRuntimeProjection::invalid("stale")),
            reason: "stale".to_string(),
        });
        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(stale_result),
        ));
        assert!(stale.events.is_empty());

        let matching_result = Box::new(ManualPromptOutcome::Rejected {
            correlation: correlation.clone(),
            transcript_text: "ship it".to_string(),
            runtime_projection: Box::new(PlanningRuntimeProjection::invalid("blocked")),
            reason: "blocked".to_string(),
        });
        let matching = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(matching_result.clone()),
        ));
        let duplicate_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(matching_result),
        ));
        assert!(
            matching
                .events
                .iter()
                .any(|event| matches!(event, AppEvent::ManualPromptPrepared(_)))
        );
        assert!(duplicate_completion.events.is_empty());

        let next = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(manual_prompt_intent("/tmp/other", "other")),
        )));
        let next_correlation = manual_prompt_correlation(2, "/tmp/other");
        assert_eq!(
            next.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted {
                    correlation: next_correlation.clone(),
                },
            )]
        );
        assert!(matches!(
            next.effects.as_slice(),
            [CoreEffect::PrepareManualPrompt(request)]
                if request.correlation == next_correlation
        ));
    }

    #[test]
    fn cancelling_manual_prompt_preparation_keeps_the_physical_lease_until_settlement() {
        let mut controller = CoreController::new();
        let first_correlation = manual_prompt_correlation(1, "/tmp/workspace");
        let _ = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(manual_prompt_intent(
                "/tmp/workspace",
                "old workspace prompt",
            )),
        )));

        let cancelled = controller.handle_input(CoreInput::Command(
            AppCommand::CancelManualPromptPreparation,
        ));
        assert!(cancelled.events.is_empty());
        assert_eq!(
            cancelled.effects,
            vec![CoreEffect::CancelManualPromptPreparation {
                correlation: first_correlation.clone(),
            }]
        );
        assert!(controller.planning.manual_prompt_preparation_is_active());
        assert!(controller.planning.manual_prompt_preparation_is_cancelled());

        let second_correlation = manual_prompt_correlation(2, "/tmp/other-workspace");
        let blocked_second = controller.handle_input(CoreInput::Command(
            AppCommand::PrepareManualPrompt(Box::new(manual_prompt_intent(
                "/tmp/other-workspace",
                "new workspace prompt",
            ))),
        ));
        assert_eq!(
            blocked_second.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::RejectedActive {
                    active_correlation: first_correlation.clone(),
                },
            )]
        );
        assert!(blocked_second.effects.is_empty());

        let late = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(Box::new(ManualPromptOutcome::Rejected {
                correlation: first_correlation,
                transcript_text: "old workspace prompt".to_string(),
                runtime_projection: Box::new(PlanningRuntimeProjection::invalid("stale")),
                reason: "late completion".to_string(),
            })),
        ));
        assert!(late.events.is_empty());
        assert!(!controller.planning.manual_prompt_preparation_is_active());

        let second = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(manual_prompt_intent(
                "/tmp/other-workspace",
                "new workspace prompt",
            )),
        )));
        assert!(matches!(
            second.effects.as_slice(),
            [CoreEffect::PrepareManualPrompt(request)]
                if request.correlation == second_correlation
        ));

        let current = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(Box::new(ManualPromptOutcome::Rejected {
                correlation: second_correlation,
                transcript_text: "new workspace prompt".to_string(),
                runtime_projection: Box::new(PlanningRuntimeProjection::invalid("blocked")),
                reason: "current completion".to_string(),
            })),
        ));
        assert!(matches!(
            current.events.as_slice(),
            [AppEvent::ManualPromptPrepared(_)]
        ));
        assert!(!controller.planning.manual_prompt_preparation_is_active());

        let back_to_first = controller.handle_input(CoreInput::Command(
            AppCommand::PrepareManualPrompt(Box::new(manual_prompt_intent(
                "/tmp/workspace",
                "new prompt in the first workspace",
            ))),
        ));
        assert!(matches!(
            back_to_first.effects.as_slice(),
            [CoreEffect::PrepareManualPrompt(request)]
                if request.correlation == manual_prompt_correlation(3, "/tmp/workspace")
        ));
    }

    #[test]
    fn session_catalog_full_identity_filters_stale_duplicate_and_aba_completions() {
        let mut controller = CoreController::new();
        let ready = SessionCatalogReadySnapshot {
            catalog: Box::new(
                RecentSessions {
                    items: Vec::new(),
                    warnings: vec!["partial row".to_string()],
                    next_cursor: None,
                }
                .into(),
            ),
            tier_label: "provider-backed catalog".to_string(),
            item_count: 0,
            warnings: vec!["partial row".to_string()],
        };

        for (limit, workspace_directory) in [
            (10, "/tmp/workspace-a"),
            (20, "/tmp/workspace-b"),
            (10, "/tmp/workspace-a"),
        ] {
            controller.handle_input(refresh_session_catalog_command(limit, workspace_directory));
        }

        for correlation in [
            session_catalog_correlation(1, 10, "/tmp/workspace-a"),
            session_catalog_correlation(2, 20, "/tmp/workspace-b"),
            session_catalog_correlation(3, 10, "/tmp/workspace-b"),
            session_catalog_correlation(3, 20, "/tmp/workspace-a"),
        ] {
            let stale = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::SessionCatalogLoaded {
                    correlation,
                    result: Err("stale catalog".to_string()),
                },
            ));
            assert!(stale.events.is_empty());
            assert!(stale.effects.is_empty());
            assert_eq!(
                controller.session_feature.active_catalog_load_for_test(),
                Some(&session_catalog_correlation(3, 10, "/tmp/workspace-a"))
            );
            assert_eq!(
                stale.snapshot.session_catalog,
                SessionCatalogSnapshot::Loading
            );
        }

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(3, 10, "/tmp/workspace-a"),
                result: Ok(ready.clone()),
            },
        ));

        assert_eq!(outcome.snapshot.revision, 4);
        assert_eq!(
            outcome.snapshot.session_catalog,
            SessionCatalogSnapshot::Ready(ready.clone())
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::SessionCatalogChanged(
                SessionCatalogSnapshot::Ready(ready.clone())
            )]
        );
        assert!(outcome.effects.is_empty());

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(3, 10, "/tmp/workspace-a"),
                result: Err("duplicate catalog".to_string()),
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
        assert_eq!(
            duplicate.snapshot.session_catalog,
            SessionCatalogSnapshot::Ready(ready)
        );
    }

    #[test]
    fn session_catalog_completion_marks_failed() {
        let mut controller = CoreController::new();
        controller.handle_input(ensure_session_catalog_command(10, "/tmp/workspace"));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(1, 10, "/tmp/workspace"),
                result: Err("catalog unavailable".to_string()),
            },
        ));

        assert_eq!(outcome.snapshot.revision, 2);
        assert_eq!(
            outcome.snapshot.session_catalog,
            SessionCatalogSnapshot::Failed {
                message: "catalog unavailable".to_string()
            }
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::SessionCatalogChanged(
                SessionCatalogSnapshot::Failed {
                    message: "catalog unavailable".to_string()
                }
            )]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_completion_marks_ready() {
        let mut controller = CoreController::new();
        let ready = sample_conversation_ready_snapshot();
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-1".to_string(),
            fallback_workspace_directory: "/tmp/root".to_string(),
        }));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-1"),
                result: Ok(Box::new(ready.clone())),
            },
        ));

        assert_eq!(outcome.snapshot.revision, 2);
        assert_eq!(
            outcome.snapshot.conversation,
            ConversationSnapshot::Ready(Box::new(ready.clone()))
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::ConversationChanged {
                correlation: Some(conversation_load_correlation(1, "thread-1")),
                snapshot: ConversationSnapshot::Ready(Box::new(ready)),
            }]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn successful_conversation_completion_clears_the_previous_workspace_projection() {
        let mut controller = CoreController::new();
        controller.handle_input(runtime_projection_changed(
            "/tmp/a",
            PlanningRuntimeProjection::ready(
                "workspace a prompt".to_string(),
                "workspace a queue".to_string(),
                None,
            ),
        ));
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-b".to_string(),
            fallback_workspace_directory: "/tmp/b".to_string(),
        }));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-b"),
                result: Ok(Box::new(sample_conversation_ready_snapshot_for("thread-b"))),
            },
        ));

        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            PlanningRuntimeProjection::uninitialized()
        );
    }

    #[test]
    fn conversation_completion_marks_failed() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-1".to_string(),
            fallback_workspace_directory: "/tmp/root".to_string(),
        }));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-1"),
                result: Err("thread unavailable".to_string()),
            },
        ));

        assert_eq!(outcome.snapshot.revision, 2);
        assert_eq!(
            outcome.snapshot.conversation,
            ConversationSnapshot::Failed {
                message: "thread unavailable".to_string()
            }
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::ConversationChanged {
                correlation: Some(conversation_load_correlation(1, "thread-1")),
                snapshot: ConversationSnapshot::Failed {
                    message: "thread unavailable".to_string()
                },
            }]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_completion_only_accepts_latest_request() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-a".to_string(),
            fallback_workspace_directory: "/tmp/a".to_string(),
        }));
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-b".to_string(),
            fallback_workspace_directory: "/tmp/b".to_string(),
        }));

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-a"),
                result: Err("stale A failure".to_string()),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(stale.snapshot.conversation, ConversationSnapshot::Loading);

        let thread_b = sample_conversation_ready_snapshot_for("thread-b");
        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(2, "thread-b"),
                result: Ok(Box::new(thread_b.clone())),
            },
        ));
        assert_eq!(
            accepted.snapshot.conversation,
            ConversationSnapshot::Ready(Box::new(thread_b.clone()))
        );

        let late_success = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-a"),
                result: Ok(Box::new(sample_conversation_ready_snapshot_for("thread-a"))),
            },
        ));
        assert!(late_success.events.is_empty());
        assert_eq!(
            late_success.snapshot.conversation,
            ConversationSnapshot::Ready(Box::new(thread_b))
        );
    }

    #[test]
    fn invalidated_conversation_load_cannot_reset_new_turn_stream() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-a".to_string(),
            fallback_workspace_directory: "/tmp/a".to_string(),
        }));
        controller.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        let turn_correlation = controller.begin_test_turn_submission();
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-draft".to_string(),
                title: "New draft".to_string(),
                cwd: "/tmp/new".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-new".to_string(),
                runtime_request: Box::default(),
            },
        ));

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-a"),
                result: Ok(Box::new(sample_conversation_ready_snapshot_for("thread-a"))),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(stale.snapshot.conversation, ConversationSnapshot::Idle);

        let notice = controller.handle_input(CoreInput::ConversationRuntimeNotice(
            "new turn still active".to_string(),
        ));
        assert!(matches!(
            notice.events.as_slice(),
            [AppEvent::TurnStreamSnapshotChanged(snapshot)]
                if snapshot.thread_id.as_deref() == Some("thread-draft")
                    && snapshot.active_turn_id.as_deref() == Some("turn-new")
        ));
    }

    #[test]
    fn matching_conversation_request_rejects_provider_thread_mismatch() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-a".to_string(),
            fallback_workspace_directory: "/tmp/a".to_string(),
        }));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-a"),
                result: Ok(Box::new(sample_conversation_ready_snapshot_for(
                    "thread-other",
                ))),
            },
        ));
        assert!(matches!(
            outcome.snapshot.conversation,
            ConversationSnapshot::Failed { ref message }
                if message == "conversation provider returned a different thread"
        ));
    }

    #[test]
    fn parallel_peek_completion_without_an_active_load_is_ignored() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation: parallel_peek_load_correlation(1, "thread-peek"),
                result: Ok(Box::new(sample_conversation_ready_snapshot())),
            },
        ));

        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
        assert!(outcome.events.is_empty());
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn parallel_peek_completion_accepts_only_the_latest_generation_once() {
        let mut controller = CoreController::new();
        let ready = sample_conversation_ready_snapshot();
        controller.handle_input(CoreInput::Command(
            AppCommand::LoadParallelPeekConversation {
                thread_id: "thread-old".to_string(),
            },
        ));
        controller.handle_input(CoreInput::Command(
            AppCommand::LoadParallelPeekConversation {
                thread_id: "thread-new".to_string(),
            },
        ));

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation: parallel_peek_load_correlation(1, "thread-old"),
                result: Ok(Box::new(ready.clone())),
            },
        ));
        assert!(stale.events.is_empty());

        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation: parallel_peek_load_correlation(2, "thread-new"),
                result: Ok(Box::new(ready.clone())),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::ParallelPeekConversationLoaded {
                correlation: parallel_peek_load_correlation(2, "thread-new"),
                result: Ok(Box::new(ready.clone())),
            }]
        );
        assert_eq!(*accepted.snapshot, AppSnapshot::initial());

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation: parallel_peek_load_correlation(2, "thread-new"),
                result: Ok(Box::new(ready)),
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn same_thread_parallel_peek_reload_rejects_the_older_generation() {
        let mut controller = CoreController::new();
        for _ in 0..2 {
            controller.handle_input(CoreInput::Command(
                AppCommand::LoadParallelPeekConversation {
                    thread_id: "thread-peek".to_string(),
                },
            ));
        }

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation: parallel_peek_load_correlation(1, "thread-peek"),
                result: Err("stale result".to_string()),
            },
        ));
        assert!(stale.events.is_empty());

        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation: parallel_peek_load_correlation(2, "thread-peek"),
                result: Err("latest result".to_string()),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::ParallelPeekConversationLoaded {
                correlation: parallel_peek_load_correlation(2, "thread-peek"),
                result: Err("latest result".to_string()),
            }]
        );
    }

    #[test]
    fn review_center_completion_accepts_only_the_latest_generation_once() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/old".to_string(),
            active_thread_id: Some("thread-old".to_string()),
        }));
        controller.handle_input(CoreInput::Command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/new".to_string(),
            active_thread_id: Some("thread-new".to_string()),
        }));
        let snapshot = empty_review_center_snapshot();

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ReviewCenterLoaded {
                correlation: review_center_load_correlation(1, "/tmp/old", Some("thread-old")),
                snapshot: snapshot.clone(),
            },
        ));
        assert!(stale.events.is_empty());

        let correlation = review_center_load_correlation(2, "/tmp/new", Some("thread-new"));
        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ReviewCenterLoaded {
                correlation: correlation.clone(),
                snapshot: snapshot.clone(),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::ReviewCenterLoaded {
                correlation: correlation.clone(),
                snapshot: snapshot.clone(),
            }]
        );

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ReviewCenterLoaded {
                correlation,
                snapshot,
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn review_center_same_identity_aba_rejects_the_older_generation() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/a".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        }));
        controller.handle_input(CoreInput::Command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/b".to_string(),
            active_thread_id: Some("thread-b".to_string()),
        }));
        controller.handle_input(CoreInput::Command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/a".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        }));
        let snapshot = empty_review_center_snapshot();

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ReviewCenterLoaded {
                correlation: review_center_load_correlation(1, "/tmp/a", Some("thread-a")),
                snapshot: snapshot.clone(),
            },
        ));
        assert!(stale.events.is_empty());

        let correlation = review_center_load_correlation(3, "/tmp/a", Some("thread-a"));
        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ReviewCenterLoaded {
                correlation: correlation.clone(),
                snapshot: snapshot.clone(),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::ReviewCenterLoaded {
                correlation,
                snapshot,
            }]
        );
    }

    #[test]
    fn queue_authority_completion_accepts_only_the_latest_generation_once() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/old".to_string(),
            active_thread_id: Some("thread-old".to_string()),
        }));
        controller.handle_input(CoreInput::Command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/new".to_string(),
            active_thread_id: Some("thread-new".to_string()),
        }));
        let result = Ok(Box::new(empty_queue_authority_snapshot()));

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueAuthorityLoaded {
                correlation: queue_authority_load_correlation(1, "/tmp/old", Some("thread-old")),
                result: result.clone(),
            },
        ));
        assert!(stale.events.is_empty());

        let correlation = queue_authority_load_correlation(2, "/tmp/new", Some("thread-new"));
        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueAuthorityLoaded {
                correlation: correlation.clone(),
                result: result.clone(),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::QueueAuthorityLoaded {
                correlation: correlation.clone(),
                result: result.clone(),
            }]
        );

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueAuthorityLoaded {
                correlation,
                result,
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn queue_authority_same_identity_aba_rejects_the_older_generation() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/a".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        }));
        controller.handle_input(CoreInput::Command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/b".to_string(),
            active_thread_id: Some("thread-b".to_string()),
        }));
        controller.handle_input(CoreInput::Command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/a".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        }));
        let result = Ok(Box::new(empty_queue_authority_snapshot()));

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueAuthorityLoaded {
                correlation: queue_authority_load_correlation(1, "/tmp/a", Some("thread-a")),
                result: result.clone(),
            },
        ));
        assert!(stale.events.is_empty());

        let correlation = queue_authority_load_correlation(3, "/tmp/a", Some("thread-a"));
        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueAuthorityLoaded {
                correlation: correlation.clone(),
                result: result.clone(),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::QueueAuthorityLoaded {
                correlation,
                result,
            }]
        );
    }

    #[test]
    fn directions_maintenance_completion_requires_the_latest_exact_correlation_once_across_aba() {
        let mut controller = CoreController::new();
        for workspace_directory in ["/tmp/a", "/tmp/b", "/tmp/a"] {
            controller.handle_input(CoreInput::Command(AppCommand::LoadDirectionsMaintenance {
                workspace_directory: workspace_directory.to_string(),
            }));
        }
        let result = Ok(directions_maintenance_summary());

        for correlation in [
            directions_maintenance_load_correlation(1, "/tmp/a"),
            directions_maintenance_load_correlation(2, "/tmp/b"),
            directions_maintenance_load_correlation(3, "/tmp/forged"),
        ] {
            let stale = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::DirectionsMaintenanceLoaded {
                    correlation,
                    result: result.clone(),
                },
            ));
            assert!(stale.events.is_empty());
            assert!(stale.effects.is_empty());
            assert_eq!(*stale.snapshot, AppSnapshot::initial());
        }

        let correlation = directions_maintenance_load_correlation(3, "/tmp/a");
        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::DirectionsMaintenanceLoaded {
                correlation: correlation.clone(),
                result: result.clone(),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::DirectionsMaintenanceLoaded {
                correlation: correlation.clone(),
                result: result.clone(),
            }]
        );
        assert!(accepted.effects.is_empty());
        assert_eq!(*accepted.snapshot, AppSnapshot::initial());

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::DirectionsMaintenanceLoaded {
                correlation,
                result,
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
        assert_eq!(*duplicate.snapshot, AppSnapshot::initial());
    }

    #[test]
    fn queue_mutation_completion_requires_the_exact_correlation_once_across_aba() {
        let mut controller = CoreController::new();
        let intent = queue_mutation_intent(
            "/tmp/workspace",
            Some("thread-1"),
            QueueMutationKind::RemoveSelected,
        );
        controller.handle_input(CoreInput::Command(AppCommand::SubmitQueueMutation(
            Box::new(intent.clone()),
        )));
        let first_correlation = QueueMutationCorrelation::new(1, intent.clone());
        let result = successful_queue_mutation_result();

        let forged = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueMutationCompleted {
                correlation: QueueMutationCorrelation::new(
                    1,
                    queue_mutation_intent(
                        "/tmp/workspace",
                        Some("thread-forged"),
                        QueueMutationKind::RemoveSelected,
                    ),
                ),
                result: result.clone(),
            },
        ));
        assert!(forged.events.is_empty());
        assert_eq!(
            controller.planning.active_queue_mutation(),
            Some(&first_correlation)
        );

        let first = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueMutationCompleted {
                correlation: first_correlation.clone(),
                result: result.clone(),
            },
        ));
        assert_eq!(
            first.events,
            vec![AppEvent::QueueMutationCompleted {
                correlation: first_correlation.clone(),
                result: result.clone(),
            }]
        );

        controller.handle_input(CoreInput::Command(AppCommand::SubmitQueueMutation(
            Box::new(intent.clone()),
        )));
        let second_correlation = QueueMutationCorrelation::new(2, intent);
        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueMutationCompleted {
                correlation: first_correlation,
                result: result.clone(),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(
            controller.planning.active_queue_mutation(),
            Some(&second_correlation)
        );

        let second = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueMutationCompleted {
                correlation: second_correlation.clone(),
                result: result.clone(),
            },
        ));
        assert_eq!(
            second.events,
            vec![AppEvent::QueueMutationCompleted {
                correlation: second_correlation.clone(),
                result: result.clone(),
            }]
        );

        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::QueueMutationCompleted {
                correlation: second_correlation,
                result,
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn conversation_stream_event_preserves_the_existing_runtime_snapshot() {
        let mut controller = CoreController::new();
        let turn_correlation = controller.begin_test_turn_submission();
        let stream_event = TurnStreamEvent::StatusUpdated {
            text: "thinking".to_string(),
        };
        let before = controller
            .handle_input(CoreInput::Command(AppCommand::Noop))
            .snapshot;

        let outcome =
            controller.handle_input(test_turn_stream_input(turn_correlation, stream_event));

        assert!(Arc::ptr_eq(&before, &outcome.snapshot));
        assert_eq!(
            outcome
                .snapshot
                .conversation_runtime
                .active_turn
                .as_ref()
                .map(|turn| turn.correlation),
            Some(turn_correlation)
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::turn_stream_snapshot_changed(TurnStreamSnapshot {
                revision: 1,
                thread_id: None,
                title: None,
                cwd: None,
                runtime_envelope: None,
                item_lifecycle: Default::default(),
                progressive_activity: Default::default(),
                active_turn_id: None,
                status_text: Some("thinking".to_string()),
                terminal: None,
                update: TurnStreamUpdate::StatusUpdated {
                    text: "thinking".to_string()
                },
            })]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn typed_turn_completion_reduces_to_core_snapshot() {
        let mut controller = CoreController::new();
        let turn_correlation = controller.begin_test_turn_submission();
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Typed terminal".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        ));
        let execution_snapshot_capture = TurnSnapshotCapture::capture_failed(
            "/tmp/workspace",
            "planning capture failed".to_string(),
        );
        let terminal_receipt =
            confirmed_terminal_receipt("thread-1", "turn-1", vec!["new/docs/plan.md".to_string()]);

        let outcome = controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::TurnTerminal {
                receipt: terminal_receipt.clone(),
                execution_snapshot_capture: Some(execution_snapshot_capture.clone()),
            },
        ));

        assert!(outcome.snapshot.revision > 0);
        assert!(outcome.snapshot.conversation_runtime.active_turn.is_none());
        let [
            AppEvent::ConversationRuntimeAuthorityChanged(runtime),
            AppEvent::TurnStreamSnapshotChanged(snapshot),
        ] = outcome.events.as_slice()
        else {
            panic!("typed completion should publish authority and one stream snapshot");
        };
        assert!(runtime.active_turn.is_none());
        assert_eq!(snapshot.revision, 3);
        assert_eq!(
            snapshot.terminal,
            Some(TurnStreamTerminalSnapshot::Turn {
                receipt: Box::new(terminal_receipt),
            })
        );
        assert!(matches!(
            &snapshot.update,
            TurnStreamUpdate::TurnCompleted {
                execution_snapshot_capture: Some(capture),
                ..
            } if capture == &execution_snapshot_capture
        ));
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_runtime_notice_reduces_to_core_snapshot_without_state_revision() {
        let mut controller = CoreController::new();
        let before = controller
            .handle_input(CoreInput::Command(AppCommand::Noop))
            .snapshot;

        let outcome = controller.handle_input(CoreInput::ConversationRuntimeNotice(
            "reattached runtime".to_string(),
        ));

        assert!(Arc::ptr_eq(&before, &outcome.snapshot));
        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            outcome.events,
            vec![AppEvent::turn_stream_snapshot_changed(TurnStreamSnapshot {
                revision: 1,
                thread_id: None,
                title: None,
                cwd: None,
                runtime_envelope: None,
                item_lifecycle: Default::default(),
                progressive_activity: Default::default(),
                active_turn_id: None,
                status_text: None,
                terminal: None,
                update: TurnStreamUpdate::RuntimeNotice {
                    notice: "reattached runtime".to_string()
                },
            })]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_load_replaces_previous_turn_stream_identity() {
        let mut controller = CoreController::new();
        let old_turn_correlation = controller.begin_test_turn_submission();
        controller.handle_input(test_turn_stream_input(
            old_turn_correlation,
            TurnStreamEvent::ThreadPrepared {
                thread_id: "old-thread".to_string(),
                title: "Old Thread".to_string(),
                cwd: "/tmp/old".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-1".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-1"),
                result: Ok(Box::new(sample_conversation_ready_snapshot())),
            },
        ));

        let outcome = controller.handle_input(CoreInput::ConversationRuntimeNotice(
            "runtime reattached".to_string(),
        ));

        assert_eq!(
            outcome.events,
            vec![AppEvent::turn_stream_snapshot_changed(TurnStreamSnapshot {
                revision: 1,
                thread_id: Some("thread-1".to_string()),
                title: Some("Core runtime".to_string()),
                cwd: Some("/tmp/workspace".to_string()),
                runtime_envelope: None,
                item_lifecycle: Default::default(),
                progressive_activity: Default::default(),
                active_turn_id: None,
                status_text: None,
                terminal: None,
                update: TurnStreamUpdate::RuntimeNotice {
                    notice: "runtime reattached".to_string()
                },
            })]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_load_hydrates_item_lifecycle_into_turn_stream_state() {
        let mut lifecycle = ConversationItemLifecycleProjection::default();
        lifecycle
            .apply(ConversationItemLifecycleObservation {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-loaded".to_string(),
                item_id: "item-loaded".to_string(),
                kind: ConversationItemKind::Reasoning,
                phase: ConversationItemLifecyclePhase::SnapshotObserved,
                source: ConversationItemLifecycleSource::Snapshot,
                observed_at_ms: None,
                outcome: ConversationItemOutcome::NotReported,
                summary: "reasoning content_blocks=1; summary_blocks=1".to_string(),
                command_actions: Default::default(),
            })
            .unwrap();
        let mut ready = sample_conversation_ready_snapshot();
        ready.conversation.item_lifecycle = lifecycle.snapshot();
        let expected_lifecycle = ready.conversation.item_lifecycle.clone();
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-1".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-1"),
                result: Ok(Box::new(ready)),
            },
        ));

        let outcome = controller.handle_input(CoreInput::ConversationRuntimeNotice(
            "runtime reattached".to_string(),
        ));

        let [AppEvent::TurnStreamSnapshotChanged(stream)] = outcome.events.as_slice() else {
            panic!("loaded lifecycle should be visible on the next turn-stream snapshot");
        };
        assert!(std::sync::Arc::ptr_eq(
            &stream.item_lifecycle,
            &expected_lifecycle
        ));
        assert_eq!(stream.item_lifecycle.records.len(), 1);
        assert_eq!(
            stream.item_lifecycle.records[0].consistency,
            ConversationItemLifecycleConsistency::SnapshotObserved
        );
    }

    #[test]
    fn conversation_load_rejects_invalid_lifecycle_before_app_state_acceptance() {
        let mut ready = sample_conversation_ready_snapshot();
        ready.conversation.item_lifecycle = std::sync::Arc::new(
            crate::domain::conversation_item_lifecycle::ConversationItemLifecycleProjectionSnapshot {
                truncated_record_count: 1,
                ..Default::default()
            },
        );
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-1".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-1"),
                result: Ok(Box::new(ready)),
            },
        ));

        assert!(matches!(
            outcome.snapshot.conversation,
            ConversationSnapshot::Failed { ref message }
                if message == "conversation provider returned an invalid item lifecycle projection"
        ));
        let notice = controller.handle_input(CoreInput::ConversationRuntimeNotice(
            "load rejected".to_string(),
        ));
        assert!(matches!(
            notice.events.as_slice(),
            [AppEvent::TurnStreamSnapshotChanged(stream)] if stream.thread_id.is_none()
        ));
    }

    #[test]
    fn post_turn_start_state_obeys_priority_and_preserves_panel_detail() {
        let existing = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshSucceeded,
            last_operation_label: Some("previous operation".to_string()),
            last_summary: Some("previous summary".to_string()),
            last_rejected_summary: Some("previous rejection".to_string()),
            last_queue_summary: Some("previous queue".to_string()),
            last_notice_detail: Some("previous detail".to_string()),
            last_prompt: Some("previous prompt".to_string()),
            last_response: Some("previous response".to_string()),
            last_host_detail: Some("previous host".to_string()),
        };
        let ready_empty =
            RuntimeProjection::ready("prompt".to_string(), "queue empty".to_string(), None);
        let refresh_empty = ready_empty.clone().with_queue_idle_policy(
            QueueIdlePolicy::ReviewAndEnqueue,
            Some("docs/planning/queue-idle-prompt.md".to_string()),
        );
        let protected_change = vec![RESULT_OUTPUT_FILE_PATH.to_string()];
        let mut repair = existing.clone();
        repair.status = PlanningWorkerStatus::RepairRunning;
        let refresh_seed = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshSucceeded,
            last_summary: Some("previous summary".to_string()),
            last_notice_detail: Some("previous detail".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let mut refresh = refresh_seed.clone();
        refresh.status = PlanningWorkerStatus::RefreshRunning;
        let adapter_supplied_seed = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RepairFailed,
            last_summary: Some("adapter must not own panel history".to_string()),
            ..PlanningWorkerPanelState::default()
        };

        for (label, history_seed, request, expected) in [
            (
                "explicit settlement pause",
                existing.clone(),
                post_turn_request(
                    ready_empty.clone(),
                    protected_change.clone(),
                    true,
                    adapter_supplied_seed.clone(),
                ),
                existing.clone(),
            ),
            (
                "protected planning file change",
                existing.clone(),
                post_turn_request(
                    ready_empty.clone(),
                    protected_change,
                    false,
                    adapter_supplied_seed.clone(),
                ),
                repair,
            ),
            (
                "empty stop-policy queue",
                existing.clone(),
                post_turn_request(
                    ready_empty,
                    Vec::new(),
                    false,
                    adapter_supplied_seed.clone(),
                ),
                existing,
            ),
            (
                "remaining refresh path",
                refresh_seed.clone(),
                post_turn_request(
                    refresh_empty,
                    Vec::new(),
                    false,
                    adapter_supplied_seed.clone(),
                ),
                refresh,
            ),
        ] {
            let mut controller = CoreController::new();
            controller
                .planning
                .replace_worker_panel_history(history_seed.clone());
            apply_completed_turn(&mut controller, "thread-1", "turn-1");
            if request.context.planning_settlement_paused {
                controller.handle_input(CoreInput::Command(AppCommand::PausePostTurnContinuation));
            }
            let outcome = controller.handle_input(CoreInput::Command(
                AppCommand::EvaluatePostTurn(Box::new(request)),
            ));
            assert_eq!(
                controller.planning.worker_panel_history(),
                &history_seed,
                "{label} start must not replace exact-accepted panel history"
            );
            assert!(
                matches!(
                    outcome.events.as_slice(),
                    [
                        AppEvent::ConversationRuntimeAuthorityChanged(runtime),
                        AppEvent::PostTurnEvaluationStarted(actual),
                    ] if runtime.post_turn.is_in_flight() && actual == &expected
                ),
                "{label} must publish authority and the full computed state first"
            );
            let [
                CoreEffect::EvaluatePostTurn {
                    correlation,
                    request: effect_request,
                },
            ] = outcome.effects.as_slice()
            else {
                panic!("{label} must dispatch exactly one post-turn effect");
            };
            assert_eq!(
                correlation,
                &PostTurnEvaluationCorrelation::new(
                    1,
                    "thread-1",
                    "turn-1",
                    "/tmp/workspace",
                    "/tmp/workspace",
                ),
                "{label} must correlate the full admitted target"
            );
            assert_eq!(
                effect_request.planning_worker_panel_state, expected,
                "{label} event and effect request must carry the same full state"
            );
        }
    }

    #[test]
    fn post_turn_panel_history_updates_only_after_exact_accepted_completion() {
        let mut controller = CoreController::new();
        let initial_history = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshSucceeded,
            last_summary: Some("last accepted evaluation".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        controller
            .planning
            .replace_worker_panel_history(initial_history.clone());
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let started = start_post_turn_evaluation(&mut controller, "turn-1");
        let correlation = post_turn_effect_correlation(&started);
        assert_eq!(
            controller.planning.worker_panel_history(),
            &initial_history,
            "a started effect is not accepted panel history"
        );

        let mut mismatched = sample_post_turn_execution();
        mismatched.thread_id = "forged-thread".to_string();
        mismatched.planning_worker_panel_state = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RepairFailed,
            last_summary: Some("mismatched completion".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: correlation.clone(),
                execution: Box::new(mismatched),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(
            controller.planning.worker_panel_history(),
            &initial_history,
            "a mismatched completion must not replace panel history"
        );

        let accepted_history = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RepairSucceeded,
            last_summary: Some("exact accepted evaluation".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let mut accepted = sample_post_turn_execution();
        accepted.planning_worker_panel_state = accepted_history.clone();
        let exact = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: correlation.clone(),
                execution: Box::new(accepted),
            },
        ));
        assert!(
            exact.events.iter().any(|event| matches!(
                event,
                AppEvent::PostTurnContinuationRoutingRequested { .. }
            ))
        );
        assert_eq!(
            controller.planning.worker_panel_history(),
            &accepted_history,
            "only the exact accepted completion becomes the next history seed"
        );

        let duplicate_history = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshFailed,
            last_summary: Some("late duplicate".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let mut duplicate = sample_post_turn_execution();
        duplicate.planning_worker_panel_state = duplicate_history;
        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation,
                execution: Box::new(duplicate),
            },
        ));
        assert!(duplicate.events.is_empty());
        assert_eq!(
            controller.planning.worker_panel_history(),
            &accepted_history,
            "a duplicate completion must not replace exact accepted history"
        );
    }

    #[test]
    fn conversation_lifecycle_resets_post_turn_panel_history() {
        let retained_history = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshSucceeded,
            last_summary: Some("previous conversation".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let mut controller = CoreController::new();
        controller
            .planning
            .replace_worker_panel_history(retained_history.clone());

        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-2".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        assert_eq!(
            controller.planning.worker_panel_history(),
            &PlanningWorkerPanelState::default(),
            "a conversation load intent must clear the previous panel history"
        );

        controller
            .planning
            .replace_worker_panel_history(retained_history);
        controller.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        assert_eq!(
            controller.planning.worker_panel_history(),
            &PlanningWorkerPanelState::default(),
            "opening a new conversation lifecycle must clear panel history"
        );
    }

    #[test]
    fn post_turn_start_requires_latest_completed_turn_and_an_idle_lease() {
        let mut controller = CoreController::new();
        let no_terminal = start_post_turn_evaluation(&mut controller, "turn-1");
        assert!(no_terminal.events.is_empty());
        assert!(no_terminal.effects.is_empty());

        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let mut wrong_thread = post_turn_request(
            RuntimeProjection::invalid("refresh required"),
            Vec::new(),
            false,
            PlanningWorkerPanelState::default(),
        );
        wrong_thread.context.thread_id = "thread-2".to_string();
        let wrong_thread = controller.handle_input(CoreInput::Command(
            AppCommand::EvaluatePostTurn(Box::new(wrong_thread)),
        ));
        assert!(wrong_thread.events.is_empty());
        assert!(wrong_thread.effects.is_empty());

        let wrong_turn = start_post_turn_evaluation(&mut controller, "turn-2");
        assert!(wrong_turn.events.is_empty());
        assert!(wrong_turn.effects.is_empty());

        let accepted = start_post_turn_evaluation(&mut controller, "turn-1");
        assert!(matches!(
            accepted.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(_),
                AppEvent::PostTurnEvaluationStarted(_),
            ]
        ));
        assert!(matches!(
            accepted.effects.as_slice(),
            [CoreEffect::EvaluatePostTurn { .. }]
        ));

        let duplicate = start_post_turn_evaluation(&mut controller, "turn-1");
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());

        let mut wrong_completion = sample_post_turn_execution();
        wrong_completion.completed_turn_id = "turn-2".to_string();
        let wrong_completion = controller.handle_input(CoreInput::EffectCompleted(
            post_turn_completion("/tmp/workspace", Box::new(wrong_completion)),
        ));
        assert!(wrong_completion.events.is_empty());
        let expected_correlation = PostTurnEvaluationCorrelation::new(
            1,
            "thread-1",
            "turn-1",
            "/tmp/workspace",
            "/tmp/workspace",
        );
        assert_eq!(
            controller.active_post_turn_evaluation_correlation(),
            Some(&expected_correlation),
            "a mismatched completion must leave the exact lease active"
        );

        let accepted_completion = controller.handle_input(CoreInput::EffectCompleted(
            post_turn_completion("/tmp/workspace", Box::new(sample_post_turn_execution())),
        ));
        assert!(
            accepted_completion.events.iter().any(|event| matches!(
                event,
                AppEvent::PostTurnContinuationRoutingRequested { .. }
            ))
        );
        assert!(
            controller
                .active_post_turn_evaluation_correlation()
                .is_none()
        );

        let already_applied = start_post_turn_evaluation(&mut controller, "turn-1");
        assert!(already_applied.events.is_empty());
        assert!(already_applied.effects.is_empty());
    }

    #[test]
    fn post_turn_policy_is_normalized_from_core_authority_at_admission() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        controller.handle_input(CoreInput::Command(AppCommand::SetAutoFollowMaxTurns {
            value: 3,
        }));
        controller.handle_input(CoreInput::Command(AppCommand::PausePostTurnContinuation));
        controller.handle_input(CoreInput::Command(AppCommand::SetParallelPostTurnRearm {
            rearmed: true,
        }));
        let mut request = post_turn_request(
            RuntimeProjection::invalid("refresh required"),
            vec!["src/main.rs".to_string()],
            true,
            PlanningWorkerPanelState::default(),
        );
        request.context.latest_main_reply = Some("finished: AUTO_STOP!".to_string());
        request.context.parallel_mode_enabled = true;
        request.context.continuation_paused = true;
        request.context.can_queue_next = false;
        request.context.stop_keyword = "forged".to_string();
        request.context.stop_keyword_matched = false;
        request.context.no_file_changes_stop_matched = true;

        let outcome = controller.handle_input(CoreInput::Command(AppCommand::EvaluatePostTurn(
            Box::new(request),
        )));
        let [
            CoreEffect::EvaluatePostTurn {
                request: admitted, ..
            },
        ] = outcome.effects.as_slice()
        else {
            panic!("the exact completed turn should admit one post-turn effect");
        };

        assert!(!admitted.context.planning_settlement_paused);
        assert!(!admitted.context.continuation_paused);
        assert!(admitted.context.can_queue_next);
        assert_eq!(admitted.context.stop_keyword, "AUTO_STOP");
        assert!(admitted.context.stop_keyword_matched);
        assert!(!admitted.context.no_file_changes_stop_matched);
    }

    #[test]
    fn auto_follow_control_commands_publish_the_authoritative_snapshot() {
        let mut controller = CoreController::new();

        let budget =
            controller.handle_input(CoreInput::Command(AppCommand::SetAutoFollowMaxTurns {
                value: 3,
            }));
        assert!(matches!(
            budget.events.as_slice(),
            [AppEvent::ConversationRuntimeAuthorityChanged(snapshot)]
                if snapshot.auto_follow.max_auto_turns == 3
                    && !snapshot.auto_follow.continuation_paused
        ));
        assert_eq!(
            budget.snapshot.conversation_runtime,
            match &budget.events[0] {
                AppEvent::ConversationRuntimeAuthorityChanged(snapshot) => {
                    snapshot.as_ref().clone()
                }
                event => panic!("unexpected authority event: {event:?}"),
            }
        );

        let paused =
            controller.handle_input(CoreInput::Command(AppCommand::PausePostTurnContinuation));
        assert!(matches!(
            paused.events.as_slice(),
            [AppEvent::ConversationRuntimeAuthorityChanged(snapshot)]
                if snapshot.auto_follow.continuation_paused
                    && !snapshot.auto_follow.parallel_rearmed_after_stop
        ));

        let rearmed =
            controller.handle_input(CoreInput::Command(AppCommand::SetParallelPostTurnRearm {
                rearmed: true,
            }));
        assert!(matches!(
            rearmed.events.as_slice(),
            [AppEvent::ConversationRuntimeAuthorityChanged(snapshot)]
                if snapshot.auto_follow.continuation_paused
                    && snapshot.auto_follow.parallel_rearmed_after_stop
        ));
    }

    #[test]
    fn post_turn_policy_change_settles_the_exact_terminal_and_drops_the_stale_worker() {
        for (label, command) in [
            (
                "turn budget",
                AppCommand::SetAutoFollowMaxTurns { value: 3 },
            ),
            ("continuation pause", AppCommand::PausePostTurnContinuation),
            (
                "parallel rearm",
                AppCommand::SetParallelPostTurnRearm { rearmed: true },
            ),
        ] {
            let mut controller = CoreController::new();
            apply_completed_turn(&mut controller, "thread-1", "turn-1");
            let started = start_post_turn_evaluation(&mut controller, "turn-1");
            let correlation = post_turn_effect_correlation(&started);
            let permit = post_turn_effect_permit(&started);
            let retained_history = PlanningWorkerPanelState {
                status: PlanningWorkerStatus::RefreshSucceeded,
                last_summary: Some(format!("retained after {label}")),
                ..PlanningWorkerPanelState::default()
            };
            controller
                .planning
                .replace_worker_panel_history(retained_history.clone());

            let changed = controller.handle_input(CoreInput::Command(command));

            assert!(
                !permit.is_current(),
                "{label} must revoke the worker permit"
            );
            assert!(
                controller
                    .active_post_turn_evaluation_correlation()
                    .is_none(),
                "{label} must release the worker lease"
            );
            assert!(matches!(
                &changed.snapshot.conversation_runtime.post_turn,
                PostTurnAuthoritySnapshot::Settled {
                    correlation: settled,
                    resolution: PostTurnRouteResolution::NoContinuation,
                } if settled == &correlation
            ));

            let duplicate = start_post_turn_evaluation(&mut controller, "turn-1");
            assert!(
                duplicate.events.is_empty() && duplicate.effects.is_empty(),
                "{label} must consume the exact terminal instead of retrying it"
            );

            let manual = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
                Box::new(test_turn_submission_request(Some("thread-1"))),
            )));
            let [
                CoreEffect::SubmitTurn {
                    correlation: manual_correlation,
                    ..
                },
            ] = manual.effects.as_slice()
            else {
                panic!("{label} must reopen manual turn admission");
            };
            let manual_correlation = *manual_correlation;

            let mut stale_execution = sample_post_turn_execution();
            stale_execution.planning_worker_panel_state = PlanningWorkerPanelState {
                status: PlanningWorkerStatus::RepairFailed,
                last_summary: Some(format!("stale worker after {label}")),
                ..PlanningWorkerPanelState::default()
            };
            let stale = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::PostTurnEvaluationCompleted {
                    correlation,
                    execution: Box::new(stale_execution),
                },
            ));

            assert!(
                stale.events.is_empty() && stale.effects.is_empty(),
                "{label} must drop the stale worker completion"
            );
            assert_eq!(
                stale
                    .snapshot
                    .conversation_runtime
                    .active_turn
                    .as_ref()
                    .map(|turn| turn.correlation),
                Some(manual_correlation),
                "{label} stale completion must not disturb the new manual turn"
            );
            assert_eq!(
                controller.planning.worker_panel_history(),
                &retained_history,
                "{label} stale completion must not apply its projection history"
            );
        }
    }

    #[test]
    fn parallel_disable_preserves_in_flight_explicit_single_session_continuation() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::SetAutoFollowMaxTurns {
            value: 1,
        }));
        controller.handle_input(CoreInput::Command(AppCommand::SetParallelPostTurnRearm {
            rearmed: true,
        }));
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let started = start_post_turn_evaluation(&mut controller, "turn-1");
        let correlation = post_turn_effect_correlation(&started);
        let permit = post_turn_effect_permit(&started);

        let disabled =
            controller.handle_input(CoreInput::Command(AppCommand::SetParallelPostTurnRearm {
                rearmed: false,
            }));

        assert!(
            permit.is_current(),
            "parallel disable must preserve a worker backed by an explicit single-session budget"
        );
        assert_eq!(
            controller.active_post_turn_evaluation_correlation(),
            Some(&correlation)
        );
        assert!(matches!(
            &disabled.snapshot.conversation_runtime.post_turn,
            PostTurnAuthoritySnapshot::Evaluating {
                correlation: active,
                ..
            } if active == &correlation
        ));
        assert!(
            !disabled
                .snapshot
                .conversation_runtime
                .auto_follow
                .parallel_rearmed_after_stop
        );

        let completed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: correlation.clone(),
                execution: Box::new(sample_post_turn_execution()),
            },
        ));
        assert!(completed.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested {
                correlation: routed,
                ..
            } if routed == &correlation
        )));

        let resolved = controller.handle_input(CoreInput::Command(
            AppCommand::ResolvePostTurnContinuation {
                correlation,
                resolution: PostTurnRouteResolution::AutoSubmit,
            },
        ));
        assert!(matches!(
            resolved.snapshot.conversation_runtime.auto_follow.phase,
            AutoFollowPhase::Queued { .. }
        ));

        let disabled_again =
            controller.handle_input(CoreInput::Command(AppCommand::SetParallelPostTurnRearm {
                rearmed: false,
            }));
        assert!(
            matches!(
                disabled_again
                    .snapshot
                    .conversation_runtime
                    .auto_follow
                    .phase,
                AutoFollowPhase::Queued { .. }
            ),
            "an idempotent parallel disable must not cancel the accepted single-session queue lease"
        );
    }

    #[test]
    fn stop_settles_post_turn_before_waiting_for_stop_completion() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let started = start_post_turn_evaluation(&mut controller, "turn-1");
        let post_turn_correlation = post_turn_effect_correlation(&started);
        let permit = post_turn_effect_permit(&started);

        let stopped =
            controller.handle_input(CoreInput::Command(AppCommand::RequestStopAllSessions));
        let [
            CoreEffect::RequestStopAllSessions {
                correlation: stop_correlation,
                attempt: StopRequestAttempt::Initial,
            },
        ] = stopped.effects.as_slice()
        else {
            panic!("stop must start one exact runtime synchronization");
        };
        let stop_correlation = *stop_correlation;

        assert!(!permit.is_current());
        assert!(matches!(
            &stopped.snapshot.conversation_runtime.post_turn,
            PostTurnAuthoritySnapshot::Settled {
                correlation,
                resolution: PostTurnRouteResolution::NoContinuation,
            } if correlation == &post_turn_correlation
        ));

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: post_turn_correlation,
                execution: Box::new(sample_post_turn_execution()),
            },
        ));
        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());

        let while_stopping = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(test_turn_submission_request(Some("thread-1"))),
        )));
        assert!(matches!(
            while_stopping.events.as_slice(),
            [AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedStopPending { .. }
            )]
        ));

        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation: stop_correlation,
                attempt: StopRequestAttempt::Initial,
                result: Ok(()),
            },
        ));
        let after_stop = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(test_turn_submission_request(Some("thread-1"))),
        )));
        assert!(
            matches!(
                after_stop.effects.as_slice(),
                [CoreEffect::SubmitTurn { .. }]
            ),
            "manual admission must reopen after the exact stop settlement"
        );
    }

    #[test]
    fn early_returning_inner_branch_cannot_bypass_runtime_authority_sync() {
        let mut controller = CoreController::new();
        controller.conversation_turn.set_auto_follow_max_turns(3);

        let outcome = controller.handle_input(CoreInput::Command(
            AppCommand::CancelManualPromptPreparation,
        ));

        assert!(matches!(
            outcome.events.as_slice(),
            [AppEvent::ConversationRuntimeAuthorityChanged(snapshot)]
                if snapshot.auto_follow.max_auto_turns == 3
        ));
        assert_eq!(
            outcome
                .snapshot
                .conversation_runtime
                .auto_follow
                .max_auto_turns,
            3
        );
    }

    #[test]
    fn post_turn_routing_emits_one_final_completion_after_exact_resolution() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let started = start_post_turn_evaluation(&mut controller, "turn-1");
        let correlation = post_turn_effect_correlation(&started);
        let execution = Box::new(sample_post_turn_execution());

        let routed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: correlation.clone(),
                execution: execution.clone(),
            },
        ));
        assert!(routed.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested {
                correlation: routed_correlation,
                execution: routed_execution,
            } if routed_correlation == &correlation && routed_execution == &execution
        )));
        assert!(
            !routed
                .events
                .iter()
                .any(|event| matches!(event, AppEvent::PostTurnEvaluationCompleted { .. }))
        );

        let stale = PostTurnEvaluationCorrelation::new(
            correlation.generation + 1,
            &correlation.thread_id,
            &correlation.completed_turn_id,
            &correlation.turn_workspace_directory,
            &correlation.planning_workspace_directory,
        );
        let stale_resolution = controller.handle_input(CoreInput::Command(
            AppCommand::ResolvePostTurnContinuation {
                correlation: stale,
                resolution: PostTurnRouteResolution::NoContinuation,
            },
        ));
        assert!(stale_resolution.events.is_empty());

        let resolved = controller.handle_input(CoreInput::Command(
            AppCommand::ResolvePostTurnContinuation {
                correlation: correlation.clone(),
                resolution: PostTurnRouteResolution::NoContinuation,
            },
        ));
        assert_eq!(
            resolved
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    AppEvent::PostTurnEvaluationCompleted {
                        correlation: completed_correlation,
                        execution: completed,
                        route_resolution: PostTurnRouteResolution::NoContinuation,
                    } if completed_correlation == &correlation && completed == &execution
                ))
                .count(),
            1
        );

        let duplicate = controller.handle_input(CoreInput::Command(
            AppCommand::ResolvePostTurnContinuation {
                correlation,
                resolution: PostTurnRouteResolution::NoContinuation,
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn manual_turn_waits_until_the_confirmed_terminal_post_turn_route_is_settled() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");

        let before_evaluation = controller.handle_input(CoreInput::Command(
            AppCommand::SubmitTurn(Box::new(test_turn_submission_request(Some("thread-1")))),
        ));
        assert!(matches!(
            before_evaluation.events.as_slice(),
            [AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedUnavailable
            )]
        ));
        assert!(before_evaluation.effects.is_empty());

        let started = start_post_turn_evaluation(&mut controller, "turn-1");
        let correlation = post_turn_effect_correlation(&started);
        let routed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: correlation.clone(),
                execution: Box::new(sample_post_turn_execution()),
            },
        ));
        assert!(routed.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested {
                correlation: routed_correlation,
                ..
            } if routed_correlation == &correlation
        )));

        let while_routing = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(test_turn_submission_request(Some("thread-1"))),
        )));
        assert!(matches!(
            while_routing.events.as_slice(),
            [AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedUnavailable
            )]
        ));
        assert!(while_routing.effects.is_empty());

        controller.handle_input(CoreInput::Command(
            AppCommand::ResolvePostTurnContinuation {
                correlation,
                resolution: PostTurnRouteResolution::NoContinuation,
            },
        ));
        let after_resolution = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(test_turn_submission_request(Some("thread-1"))),
        )));
        assert!(after_resolution.events.iter().any(|event| matches!(
            event,
            AppEvent::TurnSubmissionAdmissionResolved(TurnSubmissionAdmission::Accepted { .. })
        )));
        assert!(matches!(
            after_resolution.effects.as_slice(),
            [CoreEffect::SubmitTurn { .. }]
        ));
    }

    #[test]
    fn auto_follow_submission_requires_and_consumes_the_exact_post_turn_source_lease() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::SetAutoFollowMaxTurns {
            value: 1,
        }));
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let started = start_post_turn_evaluation(&mut controller, "turn-1");
        let source = post_turn_effect_correlation(&started);
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: source.clone(),
                execution: Box::new(sample_post_turn_execution()),
            },
        ));
        let resolved = controller.handle_input(CoreInput::Command(
            AppCommand::ResolvePostTurnContinuation {
                correlation: source.clone(),
                resolution: PostTurnRouteResolution::AutoSubmit,
            },
        ));
        assert!(matches!(
            resolved.snapshot.conversation_runtime.auto_follow.phase,
            AutoFollowPhase::Queued { .. }
        ));

        let mut auto_request = test_turn_submission_request(Some("thread-1"));
        auto_request.prompt_origin = CorePromptOrigin::AutoFollow;
        let direct = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(Box::new(
            auto_request.clone(),
        ))));
        assert!(matches!(
            direct.events.as_slice(),
            [AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedUnavailable
            )]
        ));
        assert!(matches!(
            direct.snapshot.conversation_runtime.auto_follow.phase,
            AutoFollowPhase::Queued { .. }
        ));

        let stale_source = PostTurnEvaluationCorrelation::new(
            source.generation + 1,
            &source.thread_id,
            &source.completed_turn_id,
            &source.turn_workspace_directory,
            &source.planning_workspace_directory,
        );
        auto_request.auto_follow_source = Some(stale_source);
        let stale = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(Box::new(
            auto_request.clone(),
        ))));
        assert!(matches!(
            stale.events.as_slice(),
            [AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedUnavailable
            )]
        ));
        assert!(matches!(
            stale.snapshot.conversation_runtime.auto_follow.phase,
            AutoFollowPhase::Queued { .. }
        ));

        auto_request.auto_follow_source = Some(source.clone());
        let accepted = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(auto_request.clone()),
        )));
        let accepted_correlation = accepted
            .events
            .iter()
            .find_map(|event| match event {
                AppEvent::TurnSubmissionAdmissionResolved(TurnSubmissionAdmission::Accepted {
                    correlation,
                }) => Some(*correlation),
                _ => None,
            })
            .expect("the exact source lease should admit one auto-follow turn");
        assert!(matches!(
            accepted.effects.as_slice(),
            [CoreEffect::SubmitTurn {
                request,
                ..
            }] if request.auto_follow_source.as_ref() == Some(&source)
        ));
        assert!(matches!(
            accepted.snapshot.conversation_runtime.auto_follow.phase,
            AutoFollowPhase::Submitting { .. }
        ));

        let duplicate = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(auto_request),
        )));
        assert!(matches!(
            duplicate.events.as_slice(),
            [AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedActive { active_correlation }
            )] if *active_correlation == accepted_correlation
        ));
        assert!(duplicate.effects.is_empty());
        assert!(matches!(
            duplicate.snapshot.conversation_runtime.auto_follow.phase,
            AutoFollowPhase::Submitting { .. }
        ));
    }

    #[test]
    fn post_turn_completion_requires_latest_exact_correlation_once_across_aba() {
        let mut controller = CoreController::new();
        let initial_history = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshSucceeded,
            last_summary: Some("accepted before ABA".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        controller
            .planning
            .replace_worker_panel_history(initial_history.clone());

        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let first = start_post_turn_evaluation_for(
            &mut controller,
            "thread-1",
            "turn-1",
            "/tmp/turn",
            "/tmp/planning",
        );
        let first_correlation = post_turn_effect_correlation(&first);

        controller.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        apply_completed_turn(&mut controller, "thread-2", "turn-2");
        let second = start_post_turn_evaluation_for(
            &mut controller,
            "thread-2",
            "turn-2",
            "/tmp/turn",
            "/tmp/planning",
        );
        let second_correlation = post_turn_effect_correlation(&second);

        controller.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        controller
            .planning
            .replace_worker_panel_history(initial_history.clone());
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let current = start_post_turn_evaluation_for(
            &mut controller,
            "thread-1",
            "turn-1",
            "/tmp/turn",
            "/tmp/planning",
        );
        let current_correlation = post_turn_effect_correlation(&current);
        assert_eq!(
            (
                first_correlation.generation,
                second_correlation.generation,
                current_correlation.generation,
            ),
            (1, 2, 3)
        );

        for (label, stale_correlation, mut execution) in [
            (
                "old A",
                first_correlation,
                sample_post_turn_execution_for("thread-1", "turn-1", "/tmp/turn"),
            ),
            (
                "intermediate B",
                second_correlation,
                sample_post_turn_execution_for("thread-2", "turn-2", "/tmp/turn"),
            ),
        ] {
            execution.planning_worker_panel_state = PlanningWorkerPanelState {
                status: PlanningWorkerStatus::RepairFailed,
                last_summary: Some(format!("forged stale {label} panel")),
                ..PlanningWorkerPanelState::default()
            };
            let stale = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::PostTurnEvaluationCompleted {
                    correlation: stale_correlation,
                    execution: Box::new(execution),
                },
            ));
            assert!(
                stale.events.is_empty() && stale.effects.is_empty(),
                "{label} completion must be ignored"
            );
            assert_eq!(
                controller.active_post_turn_evaluation_correlation(),
                Some(&current_correlation),
                "{label} completion must not clear the current A lease"
            );
            assert_eq!(
                controller.planning.worker_panel_history(),
                &initial_history,
                "{label} completion must not replace the current panel history seed"
            );
        }

        let accepted_history = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RepairSucceeded,
            last_summary: Some("current A accepted".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let mut current_execution =
            sample_post_turn_execution_for("thread-1", "turn-1", "/tmp/planning");
        current_execution.planning_worker_panel_state = accepted_history.clone();
        let exact = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: current_correlation.clone(),
                execution: Box::new(current_execution),
            },
        ));
        assert!(
            exact.events.iter().any(|event| matches!(
                event,
                AppEvent::PostTurnContinuationRoutingRequested { .. }
            ))
        );
        assert!(
            controller
                .active_post_turn_evaluation_correlation()
                .is_none()
        );
        assert_eq!(
            controller.planning.worker_panel_history(),
            &accepted_history,
            "the current exact A completion must replace panel history"
        );

        let mut duplicate_execution =
            sample_post_turn_execution_for("thread-1", "turn-1", "/tmp/planning");
        duplicate_execution.planning_worker_panel_state = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshFailed,
            last_summary: Some("duplicate current A".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: current_correlation,
                execution: Box::new(duplicate_execution),
            },
        ));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
        assert_eq!(
            controller.planning.worker_panel_history(),
            &accepted_history,
            "a duplicate current A completion must preserve exact accepted history"
        );
    }

    #[test]
    fn lifecycle_only_aba_prunes_the_old_post_turn_lease_before_completion() {
        let mut controller = CoreController::new();

        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let mut first_request = post_turn_request(
            RuntimeProjection::invalid("refresh required"),
            Vec::new(),
            false,
            PlanningWorkerPanelState::default(),
        );
        first_request.context.planning_workspace_directory = "/tmp/planning".to_string();
        first_request.workspace_directory = "/tmp/turn".to_string();
        let first = controller.handle_input(CoreInput::Command(AppCommand::EvaluatePostTurn(
            Box::new(first_request),
        )));
        let first_correlation = post_turn_effect_correlation(&first);
        let first_permit = post_turn_effect_permit(&first);

        controller.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        apply_completed_turn(&mut controller, "thread-2", "turn-2");
        assert!(
            controller
                .active_post_turn_evaluation_correlation()
                .is_none(),
            "leaving the admitted lifecycle must permanently prune its lease"
        );
        assert!(!first_permit.is_current());
        assert!(
            first_permit.with_current(|| ()).is_none(),
            "the pruned worker must lose authority to make later background commits"
        );
        controller.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        apply_completed_turn(&mut controller, "thread-1", "turn-1");

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: first_correlation,
                execution: Box::new(sample_post_turn_execution_for(
                    "thread-1",
                    "turn-1",
                    "/tmp/turn",
                )),
            },
        ));
        assert!(stale.events.is_empty());
        assert!(stale.effects.is_empty());
        assert!(
            controller
                .active_post_turn_evaluation_correlation()
                .is_none()
        );

        let mut current_request = post_turn_request(
            RuntimeProjection::invalid("refresh required"),
            Vec::new(),
            false,
            PlanningWorkerPanelState::default(),
        );
        current_request.context.planning_workspace_directory = "/tmp/planning".to_string();
        current_request.workspace_directory = "/tmp/turn".to_string();
        let current = controller.handle_input(CoreInput::Command(AppCommand::EvaluatePostTurn(
            Box::new(current_request),
        )));
        let current_correlation = post_turn_effect_correlation(&current);
        let current_permit = post_turn_effect_permit(&current);
        assert!(
            current_permit.is_current(),
            "the newer Core-owned lease must remain current after pruning the old lifecycle"
        );
        assert_eq!(current_correlation.generation, 2);
        let exact = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: current_correlation,
                execution: Box::new(sample_post_turn_execution_for(
                    "thread-1",
                    "turn-1",
                    "/tmp/planning",
                )),
            },
        ));
        assert!(
            exact.events.iter().any(|event| matches!(
                event,
                AppEvent::PostTurnContinuationRoutingRequested { .. }
            ))
        );
    }

    #[test]
    fn forged_post_turn_payload_cannot_settle_the_exact_lease() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let started = start_post_turn_evaluation_for(
            &mut controller,
            "thread-1",
            "turn-1",
            "/tmp/turn",
            "/tmp/planning",
        );
        let correlation = post_turn_effect_correlation(&started);
        let unchanged_revision = started.snapshot.revision;

        let mut wrong_thread = sample_post_turn_execution_for("thread-1", "turn-1", "/tmp/turn");
        wrong_thread.thread_id = "forged-thread".to_string();
        let mut wrong_turn = sample_post_turn_execution_for("thread-1", "turn-1", "/tmp/turn");
        wrong_turn.completed_turn_id = "forged-turn".to_string();
        let wrong_workspace = sample_post_turn_execution_for("thread-1", "turn-1", "/tmp/forged");
        let mut wrong_provenance =
            sample_post_turn_execution_for("thread-1", "turn-1", "/tmp/turn");
        wrong_provenance.evaluation.provenance.completed_turn_id = "forged-turn".to_string();
        let mut wrong_receipt = sample_post_turn_execution_for("thread-1", "turn-1", "/tmp/turn");
        wrong_receipt.evaluation.provenance.queue_mutation_receipt =
            Some(crate::domain::planning::PlanningQueueMutationReceipt {
                completed_turn_id: "forged-turn".to_string(),
                planning_revision: 1,
                entries: Vec::new(),
            });

        for (label, execution) in [
            ("thread", wrong_thread),
            ("turn", wrong_turn),
            ("workspace", wrong_workspace),
            ("provenance", wrong_provenance),
            ("receipt", wrong_receipt),
        ] {
            let forged = controller.handle_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::PostTurnEvaluationCompleted {
                    correlation: correlation.clone(),
                    execution: Box::new(execution),
                },
            ));
            assert!(
                forged.events.is_empty() && forged.effects.is_empty(),
                "forged {label} payload must not publish"
            );
            assert_eq!(forged.snapshot.revision, unchanged_revision);
            assert_eq!(
                controller.active_post_turn_evaluation_correlation(),
                Some(&correlation),
                "forged {label} payload must leave the exact lease active"
            );
        }

        let exact = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation,
                execution: Box::new(sample_post_turn_execution_for(
                    "thread-1",
                    "turn-1",
                    "/tmp/planning",
                )),
            },
        ));
        assert!(
            exact.events.iter().any(|event| matches!(
                event,
                AppEvent::PostTurnContinuationRoutingRequested { .. }
            ))
        );
        assert!(
            controller
                .active_post_turn_evaluation_correlation()
                .is_none()
        );
    }

    #[test]
    #[should_panic(expected = "post-turn evaluation generation exhausted")]
    fn post_turn_evaluation_generation_exhaustion_fails_before_admission() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        controller
            .conversation_turn
            .exhaust_post_turn_generation_for_test();
        let _ = start_post_turn_evaluation(&mut controller, "turn-1");
    }

    #[test]
    fn post_turn_lease_is_pruned_or_settled_after_lifecycle_supersession() {
        let mut pruned = CoreController::new();
        apply_completed_turn(&mut pruned, "thread-1", "turn-1");
        assert!(
            !start_post_turn_evaluation(&mut pruned, "turn-1")
                .effects
                .is_empty()
        );
        pruned.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        apply_completed_turn(&mut pruned, "thread-1", "turn-2");

        let next = start_post_turn_evaluation(&mut pruned, "turn-2");
        assert!(matches!(
            next.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(_),
                AppEvent::PostTurnEvaluationStarted(_),
            ]
        ));
        let expected_correlation = PostTurnEvaluationCorrelation::new(
            2,
            "thread-1",
            "turn-2",
            "/tmp/workspace",
            "/tmp/workspace",
        );
        assert_eq!(
            pruned.active_post_turn_evaluation_correlation(),
            Some(&expected_correlation)
        );

        let mut settled = CoreController::new();
        apply_completed_turn(&mut settled, "thread-1", "turn-1");
        assert!(
            !start_post_turn_evaluation(&mut settled, "turn-1")
                .effects
                .is_empty()
        );
        settled.handle_input(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        apply_completed_turn(&mut settled, "thread-1", "turn-2");

        let stale_exact = settled.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            Box::new(sample_post_turn_execution()),
        )));
        assert!(stale_exact.events.is_empty());
        assert!(stale_exact.effects.is_empty());
        assert!(
            settled.active_post_turn_evaluation_correlation().is_none(),
            "an exact but stale completion must settle its obsolete lease"
        );
        assert!(
            !start_post_turn_evaluation(&mut settled, "turn-2")
                .effects
                .is_empty()
        );
    }

    #[test]
    fn resumed_turn_accepts_post_turn_completion_without_thread_prepared_event() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-1".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, "thread-1"),
                result: Ok(Box::new(sample_conversation_ready_snapshot())),
            },
        ));
        let turn_correlation = controller.begin_test_turn_submission();
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        ));
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::TurnTerminal {
                receipt: confirmed_terminal_receipt("thread-1", "turn-1", Vec::new()),
                execution_snapshot_capture: Some(TurnSnapshotCapture::capture_failed(
                    "/tmp/workspace",
                    "test capture skipped".to_string(),
                )),
            },
        ));
        let execution = Box::new(sample_post_turn_execution());
        assert!(
            !start_post_turn_evaluation(&mut controller, "turn-1")
                .effects
                .is_empty()
        );

        let outcome = controller.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            execution.clone(),
        )));

        assert!(outcome.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested {
                execution: routed_execution,
                ..
            } if routed_execution == &execution
        )));
    }

    #[test]
    fn accepted_post_turn_evaluation_updates_core_planning_projection() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        assert!(
            !start_post_turn_evaluation(&mut controller, "turn-1")
                .effects
                .is_empty()
        );
        let execution = Box::new(sample_post_turn_execution());
        let revision_before_completion = controller.snapshot().revision;

        let outcome = controller.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            execution.clone(),
        )));

        assert_eq!(
            outcome.snapshot.revision,
            revision_before_completion + 2,
            "projection and runtime authority must each advance the shared snapshot"
        );
        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            PlanningRuntimeProjection::invalid("planning blocked")
        );
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested {
                execution: routed_execution,
                ..
            } if routed_execution == &execution
        )));
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn post_turn_projection_cannot_complete_an_active_refresh_operation() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        assert!(
            !start_post_turn_evaluation(&mut controller, "turn-1")
                .effects
                .is_empty()
        );
        controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/workspace".to_string(),
        }));
        let execution = Box::new(sample_post_turn_execution());

        let outcome = controller.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            execution.clone(),
        )));

        assert!(outcome.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested {
                execution: routed_execution,
                ..
            } if routed_execution == &execution
        )));
        assert!(
            outcome
                .events
                .contains(&AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: planning_runtime_refresh_correlation(2, "/tmp/workspace"),
                })
        );
        assert!(
            outcome
                .events
                .contains(&AppEvent::PlanningRuntimeRefreshCancelled {
                    correlation: planning_runtime_refresh_correlation(1, "/tmp/workspace"),
                })
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadPlanningRuntime {
                correlation: planning_runtime_refresh_correlation(2, "/tmp/workspace"),
            }]
        );
        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: planning_runtime_refresh_correlation(1, "/tmp/workspace"),
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("stale read"),
                )),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(
            *stale.snapshot.planning_parallel.planning_runtime,
            PlanningRuntimeProjection::invalid("planning blocked")
        );
        let replacement_projection = PlanningRuntimeProjection::invalid("planning blocked");
        let replacement = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: planning_runtime_refresh_correlation(2, "/tmp/workspace"),
                result: Ok(planning_runtime_refresh_snapshot(
                    replacement_projection.clone(),
                )),
            },
        ));
        assert_eq!(
            replacement.events,
            vec![AppEvent::PlanningRuntimeRefreshed {
                correlation: planning_runtime_refresh_correlation(2, "/tmp/workspace"),
                result: Ok(planning_doctor_snapshot(&replacement_projection)),
            }]
        );
    }

    #[test]
    fn parallel_slot_post_turn_does_not_settle_the_root_planning_refresh() {
        let mut controller = CoreController::new();
        let root_projection = PlanningRuntimeProjection::ready(
            "root prompt".to_string(),
            "root queue".to_string(),
            None,
        );
        controller.handle_input(runtime_projection_changed(
            "/tmp/root",
            root_projection.clone(),
        ));
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let started = start_post_turn_evaluation_for(
            &mut controller,
            "thread-1",
            "turn-1",
            "/tmp/slot",
            "/tmp/root",
        );
        let post_turn_correlation = post_turn_effect_correlation(&started);
        controller.handle_input(CoreInput::Command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/root".to_string(),
        }));
        let mut execution = Box::new(sample_post_turn_execution());
        execution.runtime_projection_workspace_directory = "/tmp/slot".to_string();

        let slot_completion = controller.handle_input(CoreInput::EffectCompleted(
            post_turn_completion_for(post_turn_correlation, "/tmp/slot", execution.clone()),
        ));

        assert!(slot_completion.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested {
                execution: routed_execution,
                ..
            } if routed_execution == &execution
        )));
        assert_eq!(
            *slot_completion.snapshot.planning_parallel.planning_runtime,
            root_projection
        );
        assert_eq!(
            slot_completion
                .snapshot
                .planning_parallel
                .planning_runtime_workspace_directory
                .as_deref(),
            Some("/tmp/root")
        );

        let correlation = planning_runtime_refresh_correlation(1, "/tmp/root");
        let root_completion = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation: correlation.clone(),
                result: Ok(planning_runtime_refresh_snapshot(
                    PlanningRuntimeProjection::invalid("loaded root"),
                )),
            },
        ));
        assert_eq!(
            root_completion.events,
            vec![AppEvent::PlanningRuntimeRefreshed {
                correlation,
                result: Ok(planning_doctor_snapshot(
                    &PlanningRuntimeProjection::invalid("loaded root",)
                )),
            }]
        );
        assert_eq!(
            *root_completion.snapshot.planning_parallel.planning_runtime,
            PlanningRuntimeProjection::invalid("loaded root")
        );
    }

    #[test]
    fn matching_post_turn_projection_keeps_revision_and_delivers_completion() {
        let mut controller = CoreController::new();
        let projection = PlanningRuntimeProjection::invalid("planning blocked");
        controller.handle_input(runtime_projection_changed(
            "/tmp/workspace",
            projection.clone(),
        ));
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        assert!(
            !start_post_turn_evaluation(&mut controller, "turn-1")
                .effects
                .is_empty()
        );
        let execution = Box::new(sample_post_turn_execution());
        let revision_before_completion = controller.snapshot().revision;

        let outcome = controller.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            execution.clone(),
        )));

        assert_eq!(
            outcome.snapshot.revision,
            revision_before_completion + 1,
            "an unchanged projection must advance only runtime authority"
        );
        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            projection
        );
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested {
                execution: routed_execution,
                ..
            } if routed_execution == &execution
        )));
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn post_turn_completion_is_dropped_during_conversation_load() {
        let mut controller = CoreController::new();
        let current_projection = PlanningRuntimeProjection::ready(
            "current prompt".to_string(),
            "current summary".to_string(),
            None,
        );
        controller.handle_input(runtime_projection_changed(
            "/tmp/workspace",
            current_projection.clone(),
        ));
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        assert!(
            !start_post_turn_evaluation(&mut controller, "turn-1")
                .effects
                .is_empty()
        );
        let load = controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-2".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        let [CoreEffect::LoadConversation { correlation, .. }] = load.effects.as_slice() else {
            panic!("conversation load should emit one correlated effect");
        };
        let correlation = correlation.clone();
        let snapshot_before_completion = controller.snapshot();

        let dropped = controller.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            Box::new(sample_post_turn_execution()),
        )));

        assert_eq!(*dropped.snapshot, snapshot_before_completion);
        assert!(dropped.events.is_empty());
        assert!(dropped.effects.is_empty());

        let failed_load = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation,
                result: Err("load failed".to_string()),
            },
        ));
        assert_eq!(
            *failed_load.snapshot.planning_parallel.planning_runtime,
            current_projection
        );
    }

    #[test]
    fn stale_post_turn_evaluation_completion_is_dropped_in_core() {
        let mut controller = CoreController::new();
        let existing_projection = PlanningRuntimeProjection::ready(
            "existing prompt".to_string(),
            "existing summary".to_string(),
            None,
        );
        controller.handle_input(runtime_projection_changed(
            "/tmp/workspace",
            existing_projection,
        ));
        apply_completed_turn(&mut controller, "thread-1", "turn-2");
        let snapshot_before_stale_completion = controller.snapshot();
        let mut execution = sample_post_turn_execution();
        execution.completed_turn_id = "turn-1".to_string();
        execution.evaluation.provenance =
            crate::application::service::post_turn_evaluation::PostTurnEvaluationProvenance::new(
                "turn-1".to_string(),
            );

        let outcome = controller.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            Box::new(execution),
        )));

        assert_eq!(*outcome.snapshot, snapshot_before_stale_completion);
        assert!(outcome.events.is_empty());
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn duplicate_post_turn_evaluation_completion_is_dropped_in_core() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        assert!(
            !start_post_turn_evaluation(&mut controller, "turn-1")
                .effects
                .is_empty()
        );
        let execution = Box::new(sample_post_turn_execution());
        let mut duplicate_execution = (*execution).clone();
        duplicate_execution.evaluation.runtime_projection = PlanningRuntimeProjection::ready(
            "duplicate prompt".to_string(),
            "duplicate summary".to_string(),
            None,
        );

        let first = controller.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            execution,
        )));
        let duplicate = controller.handle_input(CoreInput::EffectCompleted(post_turn_completion(
            "/tmp/workspace",
            Box::new(duplicate_execution),
        )));

        assert!(matches!(
            first.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(_),
                AppEvent::PostTurnContinuationRoutingRequested { .. },
            ]
        ));
        assert_eq!(*duplicate.snapshot, *first.snapshot);
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn manual_prompt_preparation_completion_defers_projection_until_tui_accepts_correlation() {
        let mut controller = CoreController::new();
        let correlation = manual_prompt_correlation(1, "/tmp/workspace");
        let _ = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(manual_prompt_intent("/tmp/workspace", "ship it")),
        )));
        let result = Box::new(ManualPromptOutcome::Rejected {
            correlation,
            transcript_text: "ship it".to_string(),
            runtime_projection: Box::new(PlanningRuntimeProjection::invalid(
                "planning validation failed",
            )),
            reason: "blocked".to_string(),
        });

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(result.clone()),
        ));

        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
        assert_eq!(outcome.events, vec![AppEvent::ManualPromptPrepared(result)]);
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn planning_parallel_projection_changes_appear_in_app_snapshot() {
        let mut controller = CoreController::new();
        let planning_projection =
            PlanningRuntimeProjection::invalid("planning validation failed in projection");

        let outcome = controller.handle_input(runtime_projection_changed(
            "/tmp/workspace",
            planning_projection.clone(),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            planning_projection
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::SnapshotChanged(outcome.snapshot.clone())]
        );
        let [AppEvent::SnapshotChanged(event_snapshot)] = outcome.events.as_slice() else {
            panic!("planning projection change should publish its shared snapshot");
        };
        assert!(Arc::ptr_eq(event_snapshot, &outcome.snapshot));
        assert!(outcome.effects.is_empty());

        let readiness_snapshot = ParallelModeReadinessSnapshot::new(
            "/tmp/workspace",
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        );
        let outcome = controller.handle_input(CoreInput::ParallelModeReadinessProjectionChanged(
            Some(Box::new(readiness_snapshot.clone())),
        ));

        assert_eq!(outcome.snapshot.revision, 2);
        assert_eq!(
            outcome
                .snapshot
                .planning_parallel
                .parallel_mode
                .readiness
                .as_deref(),
            Some(&readiness_snapshot)
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::SnapshotChanged(outcome.snapshot.clone())]
        );
        let [AppEvent::SnapshotChanged(event_snapshot)] = outcome.events.as_slice() else {
            panic!("parallel readiness change should publish its shared snapshot");
        };
        assert!(Arc::ptr_eq(event_snapshot, &outcome.snapshot));
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn repeated_projection_input_does_not_advance_snapshot_revision() {
        let mut controller = CoreController::new();
        let planning_projection =
            PlanningRuntimeProjection::invalid("planning validation failed in projection");
        let first = controller.handle_input(runtime_projection_changed(
            "/tmp/workspace",
            planning_projection.clone(),
        ));

        let outcome = controller.handle_input(runtime_projection_changed(
            "/tmp/workspace",
            planning_projection,
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert!(Arc::ptr_eq(&first.snapshot, &outcome.snapshot));
        assert!(outcome.events.is_empty());
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_workspace_change_publishes_runtime_authority() {
        let mut controller = CoreController::new();
        let turn_correlation = controller.begin_test_turn_submission();

        let outcome = controller.handle_input(CoreInput::ConversationTurnWorkspaceChanged {
            correlation: turn_correlation,
            workspace_directory: "/tmp/slot-worktree".to_string(),
        });

        assert_eq!(
            outcome
                .snapshot
                .conversation_runtime
                .active_turn
                .as_ref()
                .map(|turn| turn.workspace_directory.as_str()),
            Some("/tmp/slot-worktree")
        );
        assert!(matches!(
            outcome.events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(_),
                AppEvent::ConversationTurnWorkspaceChanged {
                    workspace_directory,
                },
            ] if workspace_directory == "/tmp/slot-worktree"
        ));
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn parallel_supervisor_invalidation_passes_through_core_without_state_revision() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::ParallelModeSupervisorSnapshotInvalidated);

        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            outcome.events,
            vec![AppEvent::ParallelModeSupervisorSnapshotInvalidated]
        );
        assert!(outcome.effects.is_empty());
    }

    fn sample_startup_ready_snapshot() -> StartupReadySnapshot {
        StartupReadySnapshot {
            cwd: "/tmp/workspace".to_string(),
            workspace_path: "/tmp/workspace".to_string(),
            can_continue: true,
            codex_binary: StartupDiagnosticSnapshot {
                ok: true,
                detail: "/usr/bin/codex".to_string(),
            },
            workspace: StartupDiagnosticSnapshot {
                ok: true,
                detail: "git repo: /tmp/workspace".to_string(),
            },
            app_server_initialize: StartupDiagnosticSnapshot {
                ok: true,
                detail: "initialized".to_string(),
            },
            account: StartupDiagnosticSnapshot {
                ok: true,
                detail: "authenticated".to_string(),
            },
            attachment: StartupAttachmentSnapshot {
                mode_label: "provider-launched".to_string(),
                recovery_anchor_label: "provider-thread-id".to_string(),
            },
            warnings: vec!["non fatal".to_string()],
            schema_snapshot: "embedded schema".to_string(),
        }
    }

    fn sample_conversation_ready_snapshot() -> ConversationReadySnapshot {
        sample_conversation_ready_snapshot_for("thread-1")
    }

    fn sample_conversation_ready_snapshot_for(thread_id: &str) -> ConversationReadySnapshot {
        DomainConversationSnapshot {
            thread_id: thread_id.to_string(),
            title: "Core runtime".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: vec![ConversationMessage::new(
                ConversationMessageKind::Agent,
                "ready",
                None,
                None,
            )],
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        }
        .into()
    }

    fn github_review_poll_result(
        target: &GithubPullRequestTarget,
        latest_submitted_at: &str,
    ) -> Box<GithubPullRequestPollResult> {
        Box::new(GithubPullRequestPollResult {
            snapshot: GithubPullRequestActivitySnapshot {
                target: target.clone(),
                title: "Review poll".to_string(),
                url: "https://github.com/acme/widgets/pull/42".to_string(),
                head_branch: "feature".to_string(),
                base_branch: "prerelease".to_string(),
                events: Vec::new(),
            },
            changes: Vec::new(),
            next_state: GithubPullRequestPollState {
                latest_submitted_at: Some(latest_submitted_at.to_string()),
                seen_events_at_latest_timestamp: Vec::new(),
            },
        })
    }

    fn github_review_setup_request(
        workspace_directory: &str,
        target: GithubPullRequestTarget,
    ) -> GithubReviewPollingSetupRequest {
        GithubReviewPollingSetupRequest::new(
            workspace_directory,
            GithubReviewPollingSetupMode::Explicit { target },
        )
    }

    fn start_github_review_setup(
        controller: &mut CoreController,
        workspace_directory: &str,
        target: GithubPullRequestTarget,
    ) -> GithubReviewPollingSetupCorrelation {
        let outcome =
            controller.handle_input(CoreInput::Command(AppCommand::SetupGithubReviewPolling(
                github_review_setup_request(workspace_directory, target),
            )));
        let [CoreEffect::SetupGithubReviewPolling { correlation, .. }] = outcome.effects.as_slice()
        else {
            panic!("setup should emit one typed effect");
        };
        correlation.clone()
    }

    fn complete_github_review_setup(
        controller: &mut CoreController,
        correlation: GithubReviewPollingSetupCorrelation,
        target: GithubPullRequestTarget,
    ) {
        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                correlation,
                result: Ok(GithubReviewPollingSetupResult::Active { target }),
            },
        ));
        assert!(matches!(
            outcome.events.as_slice(),
            [AppEvent::GithubReviewPollingSetupCompleted {
                result: Ok(GithubReviewPollingSetupResult::Active { .. }),
                ..
            }]
        ));
    }

    fn activate_github_review_setup(
        controller: &mut CoreController,
        workspace_directory: &str,
        target: GithubPullRequestTarget,
    ) -> GithubReviewPollingSetupCorrelation {
        let correlation =
            start_github_review_setup(controller, workspace_directory, target.clone());
        complete_github_review_setup(controller, correlation.clone(), target);
        correlation
    }

    fn sample_session_summary(thread_id: &str, name: &str) -> SessionSummary {
        SessionSummary {
            id: thread_id.to_string(),
            name: Some(name.to_string()),
            preview: format!("{name} preview"),
            cwd: "/tmp/workspace".to_string(),
            source: "test".to_string(),
            model_provider: "test".to_string(),
            updated_at_epoch: 1,
            status_type: "ready".to_string(),
            path: format!("/tmp/workspace/{thread_id}"),
            git_branch: None,
        }
    }

    fn load_test_session_catalog(controller: &mut CoreController) {
        controller.handle_input(ensure_session_catalog_command(10, "/tmp/workspace"));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(1, 10, "/tmp/workspace"),
                result: Ok(SessionCatalogReadySnapshot::from_catalog(
                    RecentSessions {
                        items: vec![
                            sample_session_summary("thread-alpha", "Alpha"),
                            sample_session_summary("thread-beta", "Beta"),
                        ],
                        warnings: Vec::new(),
                        next_cursor: None,
                    }
                    .into(),
                )),
            },
        ));
    }

    fn load_test_conversation(controller: &mut CoreController, thread_id: &str) {
        controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: thread_id.to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_load_correlation(1, thread_id),
                result: Ok(Box::new(sample_conversation_ready_snapshot_for(thread_id))),
            },
        ));
    }

    fn confirmed_terminal_receipt(
        thread_id: &str,
        turn_id: &str,
        changed_planning_file_paths: Vec<String>,
    ) -> crate::domain::turn_terminal::ConversationTurnTerminalReceipt {
        crate::domain::turn_terminal::ConversationTurnTerminalReceipt::completed(
            thread_id,
            turn_id,
            changed_planning_file_paths,
        )
        .with_application_delivery(
            crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
        )
    }

    fn apply_completed_turn(
        controller: &mut CoreController,
        thread_id: &str,
        turn_id: &str,
    ) -> TurnSubmissionCorrelation {
        let turn_correlation = controller.begin_test_turn_submission();
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::ThreadPrepared {
                thread_id: thread_id.to_string(),
                title: "Core runtime".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::TurnStarted {
                turn_id: turn_id.to_string(),
                runtime_request: Box::default(),
            },
        ));
        controller.handle_input(test_turn_stream_input(
            turn_correlation,
            TurnStreamEvent::TurnTerminal {
                receipt: confirmed_terminal_receipt(thread_id, turn_id, Vec::new()),
                execution_snapshot_capture: Some(TurnSnapshotCapture::capture_failed(
                    "/tmp/workspace",
                    "test capture skipped".to_string(),
                )),
            },
        ));
        turn_correlation
    }

    fn test_turn_stream_input(
        correlation: TurnSubmissionCorrelation,
        event: TurnStreamEvent,
    ) -> CoreInput {
        CoreInput::ConversationStreamUpdated { correlation, event }
    }

    fn request_test_approval(
        controller: &mut CoreController,
        correlation: TurnSubmissionCorrelation,
        approval_id: &str,
    ) {
        controller.handle_input(test_turn_stream_input(
            correlation,
            TurnStreamEvent::ApprovalRequested {
                request: ConversationApprovalRequest {
                    approval_id: approval_id.to_string(),
                    server_request_id: format!("server-{approval_id}"),
                    method: "item/commandExecution/requestApproval".to_string(),
                    kind: ConversationApprovalRequestKind::CommandExecution,
                    summary: "Command execution requested.".to_string(),
                    details: Vec::new(),
                },
            },
        ));
    }

    fn approval_identity(approval_id: &str) -> ConversationApprovalRequestIdentity {
        ConversationApprovalRequestIdentity {
            approval_id: approval_id.to_string(),
            server_request_id: format!("server-{approval_id}"),
        }
    }

    fn conversation_preference_request(
        model: &str,
        thread_id: &str,
    ) -> ConversationPreferencePersistenceRequest {
        ConversationPreferencePersistenceRequest {
            global_workspace_directory: Some("/tmp/workspace".to_string()),
            active_thread: Some(ConversationPreferenceThreadTarget {
                workspace_directory: "/tmp/workspace".to_string(),
                thread_id: thread_id.to_string(),
            }),
            options: ConversationTurnOptions {
                model: Some(model.to_string()),
                reasoning_effort: Some(ConversationReasoningEffort::Medium),
            },
        }
    }

    fn test_approval_review(target_item_id: &str) -> ConversationApprovalReview {
        ConversationApprovalReview {
            target_item_id: target_item_id.to_string(),
            status: ConversationApprovalReviewStatus::Unknown("human_review_requested".to_string()),
            risk_level: Some("medium".to_string()),
            rationale: Some("operator review required".to_string()),
        }
    }

    fn test_turn_submission_request(thread_id: Option<&str>) -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: thread_id.map(str::to_string),
            image_paths: Vec::new(),
            prompt: "ship it".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        }
    }

    fn post_turn_request(
        current_runtime_projection: RuntimeProjection,
        changed_planning_file_paths: Vec<String>,
        planning_settlement_paused: bool,
        planning_worker_panel_state: PlanningWorkerPanelState,
    ) -> PostTurnRequest {
        PostTurnRequest {
            context: PostTurnContext {
                thread_id: "thread-1".to_string(),
                planning_workspace_directory: "/tmp/workspace".to_string(),
                latest_user_message: None,
                latest_main_reply: None,
                previous_handoff_task: None,
                current_runtime_projection,
                parallel_mode_enabled: false,
                parallel_automation_epoch_id: None,
                planning_settlement_paused,
                continuation_paused: false,
                can_queue_next: false,
                stop_keyword: ":stop".to_string(),
                stop_keyword_matched: false,
                no_file_changes_stop_matched: false,
                mode_label: "test".to_string(),
            },
            workspace_directory: "/tmp/workspace".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths,
            execution_snapshot_capture: None,
            planning_worker_panel_state,
            continuation_permit: PostTurnContinuationGate::default().capture(),
        }
    }

    fn start_post_turn_evaluation(
        controller: &mut CoreController,
        completed_turn_id: &str,
    ) -> CoreDispatchOutcome {
        start_post_turn_evaluation_for(
            controller,
            "thread-1",
            completed_turn_id,
            "/tmp/workspace",
            "/tmp/workspace",
        )
    }

    fn start_post_turn_evaluation_for(
        controller: &mut CoreController,
        thread_id: &str,
        completed_turn_id: &str,
        turn_workspace_directory: &str,
        planning_workspace_directory: &str,
    ) -> CoreDispatchOutcome {
        let mut request = post_turn_request(
            RuntimeProjection::invalid("refresh required"),
            Vec::new(),
            false,
            PlanningWorkerPanelState::default(),
        );
        request.context.thread_id = thread_id.to_string();
        request.context.planning_workspace_directory = planning_workspace_directory.to_string();
        request.workspace_directory = turn_workspace_directory.to_string();
        request.completed_turn_id = completed_turn_id.to_string();
        controller.handle_input(CoreInput::Command(AppCommand::EvaluatePostTurn(Box::new(
            request,
        ))))
    }

    fn post_turn_effect_correlation(
        outcome: &CoreDispatchOutcome,
    ) -> PostTurnEvaluationCorrelation {
        let [CoreEffect::EvaluatePostTurn { correlation, .. }] = outcome.effects.as_slice() else {
            panic!("test post-turn admission must dispatch one correlated effect");
        };
        correlation.clone()
    }

    fn post_turn_effect_permit(outcome: &CoreDispatchOutcome) -> PostTurnContinuationPermit {
        let [CoreEffect::EvaluatePostTurn { request, .. }] = outcome.effects.as_slice() else {
            panic!("test post-turn admission must dispatch one request-local permit");
        };
        request.continuation_permit.clone()
    }

    fn submit_test_turn(
        controller: &mut CoreController,
        thread_id: Option<&str>,
    ) -> TurnSubmissionCorrelation {
        let outcome = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            Box::new(test_turn_submission_request(thread_id)),
        )));
        let [CoreEffect::SubmitTurn { correlation, .. }] = outcome.effects.as_slice() else {
            panic!("test submission should produce one correlated effect");
        };
        *correlation
    }

    fn start_test_turn(
        controller: &mut CoreController,
        thread_id: &str,
        turn_id: &str,
    ) -> TurnSubmissionCorrelation {
        let correlation = submit_test_turn(controller, Some(thread_id));
        controller.handle_input(test_turn_stream_input(
            correlation,
            TurnStreamEvent::ThreadPrepared {
                thread_id: thread_id.to_string(),
                title: "Core runtime".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        ));
        controller.handle_input(test_turn_stream_input(
            correlation,
            TurnStreamEvent::TurnStarted {
                turn_id: turn_id.to_string(),
                runtime_request: Box::default(),
            },
        ));
        correlation
    }

    fn sample_post_turn_execution()
    -> crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution {
        use crate::application::service::post_turn_evaluation::{
            PlanningWorkerPanelState, PostTurnAutoFollowSkipReason, PostTurnContinuationAction,
            PostTurnEvaluationOutcome, PostTurnEvaluationProvenance,
        };

        crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution {
            thread_id: "thread-1".to_string(),
            completed_turn_id: "turn-1".to_string(),
            runtime_projection_workspace_directory: "/tmp/workspace".to_string(),
            evaluation: PostTurnEvaluationOutcome {
                provenance: PostTurnEvaluationProvenance::new("turn-1".to_string()),
                runtime_projection: PlanningRuntimeProjection::invalid("planning blocked"),
                planning_repair_state: None,
                runtime_notices: Vec::new(),
                action: PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
                },
                operator_alerts: Vec::new(),
            },
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
        }
    }

    fn sample_post_turn_execution_for(
        thread_id: &str,
        completed_turn_id: &str,
        runtime_projection_workspace_directory: &str,
    ) -> crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution {
        let mut execution = sample_post_turn_execution();
        execution.thread_id = thread_id.to_string();
        execution.completed_turn_id = completed_turn_id.to_string();
        execution.runtime_projection_workspace_directory =
            runtime_projection_workspace_directory.to_string();
        execution.evaluation.provenance.completed_turn_id = completed_turn_id.to_string();
        execution
    }
}
