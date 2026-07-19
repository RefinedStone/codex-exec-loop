use super::{
    AppCommand, AppEvent, AppSnapshot, AppState, ApprovalDecisionAdmission,
    ApprovalDecisionCorrelation, ConversationLoadCorrelation, CoreEffect, CoreEffectCompletion,
    CoreInput, GithubReviewPollCorrelation, ManualPromptPreparationAdmission,
    ManualPromptPreparationIntent, ParallelPeekLoadCorrelation, QueueAuthorityLoadCorrelation,
    QueueMutationCorrelation, ReviewCenterLoadCorrelation, SessionCatalogLoadCorrelation,
    SessionRenameAcceptedSnapshot, SessionRenameCorrelation, StartupCheckCorrelation,
    StopRequestAdmission, StopRequestAttempt, StopRequestCorrelation, TurnSteerAdmission,
    TurnSteerCorrelation, TurnStreamEvent, TurnStreamState, TurnStreamUpdate,
    TurnSubmissionAdmission, TurnSubmissionCorrelation,
};
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleProjection;
use crate::domain::github_review::{GithubPullRequestPollState, GithubPullRequestTarget};
use crate::domain::planning::{ManualPromptCorrelation, ManualPromptRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDispatchOutcome {
    pub events: Vec<AppEvent>,
    pub effects: Vec<CoreEffect>,
    pub snapshot: AppSnapshot,
}

#[derive(Debug, Clone)]
struct ActiveTurnSteer {
    correlation: TurnSteerCorrelation,
    expected_turn_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalDecisionPhase {
    Submitting,
    Submitted,
}

#[derive(Debug, Clone)]
struct ActiveApprovalDecision {
    correlation: ApprovalDecisionCorrelation,
    phase: ApprovalDecisionPhase,
}

#[derive(Debug, Clone, Copy)]
struct ActiveStopRequest {
    correlation: StopRequestCorrelation,
    pending_attempt: Option<StopRequestAttempt>,
    synchronize_after_turn_started: bool,
}

#[derive(Debug, Clone)]
pub struct CoreController {
    state: AppState,
    turn_stream_state: TurnStreamState,
    next_startup_check_generation: u64,
    in_flight_startup_check: Option<StartupCheckCorrelation>,
    next_session_catalog_load_generation: u64,
    in_flight_session_catalog_load: Option<SessionCatalogLoadCorrelation>,
    next_session_rename_generation: u64,
    in_flight_session_rename: Option<SessionRenameCorrelation>,
    guarded_session_rename_stream: Option<(TurnSubmissionCorrelation, SessionRenameCorrelation)>,
    deferred_session_catalog_load: Option<(usize, String)>,
    next_conversation_load_generation: u64,
    in_flight_conversation_load: Option<ConversationLoadCorrelation>,
    deferred_conversation_load: Option<(String, String)>,
    next_parallel_peek_load_generation: u64,
    active_parallel_peek_load: Option<ParallelPeekLoadCorrelation>,
    next_review_center_load_generation: u64,
    active_review_center_load: Option<ReviewCenterLoadCorrelation>,
    next_queue_authority_load_generation: u64,
    active_queue_authority_load: Option<QueueAuthorityLoadCorrelation>,
    next_queue_mutation_generation: u64,
    active_queue_mutation: Option<QueueMutationCorrelation>,
    next_manual_prompt_preparation_generation: u64,
    in_flight_manual_prompt_preparation: Option<ManualPromptCorrelation>,
    next_turn_submission_generation: u64,
    active_turn_submission: Option<TurnSubmissionCorrelation>,
    next_stop_request_generation: u64,
    active_stop_request: Option<ActiveStopRequest>,
    next_turn_steer_generation: u64,
    active_turn_steer: Option<ActiveTurnSteer>,
    next_approval_decision_generation: u64,
    active_approval_decision: Option<ActiveApprovalDecision>,
    github_review_poll_target: Option<GithubPullRequestTarget>,
    github_review_poll_cursor: Option<(GithubPullRequestTarget, GithubPullRequestPollState)>,
    next_github_review_poll_generation: u64,
    active_github_review_poll: Option<GithubReviewPollCorrelation>,
}

impl CoreController {
    pub fn new() -> Self {
        Self {
            state: AppState::new(),
            turn_stream_state: TurnStreamState::new(),
            next_startup_check_generation: 1,
            in_flight_startup_check: None,
            next_session_catalog_load_generation: 1,
            in_flight_session_catalog_load: None,
            next_session_rename_generation: 1,
            in_flight_session_rename: None,
            guarded_session_rename_stream: None,
            deferred_session_catalog_load: None,
            next_conversation_load_generation: 1,
            in_flight_conversation_load: None,
            deferred_conversation_load: None,
            next_parallel_peek_load_generation: 1,
            active_parallel_peek_load: None,
            next_review_center_load_generation: 1,
            active_review_center_load: None,
            next_queue_authority_load_generation: 1,
            active_queue_authority_load: None,
            next_queue_mutation_generation: 1,
            active_queue_mutation: None,
            next_manual_prompt_preparation_generation: 1,
            in_flight_manual_prompt_preparation: None,
            next_turn_submission_generation: 1,
            active_turn_submission: None,
            next_stop_request_generation: 1,
            active_stop_request: None,
            next_turn_steer_generation: 1,
            active_turn_steer: None,
            next_approval_decision_generation: 1,
            active_approval_decision: None,
            github_review_poll_target: None,
            github_review_poll_cursor: None,
            next_github_review_poll_generation: 1,
            active_github_review_poll: None,
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        self.state.snapshot()
    }

    pub fn handle_input(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        match input {
            CoreInput::Command(AppCommand::Noop) => CoreDispatchOutcome {
                events: Vec::new(),
                effects: Vec::new(),
                snapshot: self.snapshot(),
            },
            CoreInput::Command(AppCommand::RunStartupChecks) => {
                let correlation = StartupCheckCorrelation::new(take_generation(
                    &mut self.next_startup_check_generation,
                    "startup check",
                ));
                self.in_flight_startup_check = Some(correlation);
                self.state.mark_startup_loading();
                self.startup_changed_outcome(
                    correlation,
                    vec![CoreEffect::RunStartupChecks { correlation }],
                )
            }
            CoreInput::Command(AppCommand::LoadSessionCatalog {
                limit,
                workspace_directory,
            }) => {
                if self.in_flight_session_rename.is_some() {
                    self.deferred_session_catalog_load = Some((limit, workspace_directory));
                    return self.unchanged_outcome();
                }
                self.start_session_catalog_load(limit, workspace_directory)
            }
            CoreInput::Command(AppCommand::RenameSession(request)) => {
                if self.in_flight_session_rename.is_some() {
                    return self.unchanged_outcome();
                }
                let correlation = SessionRenameCorrelation::new(
                    take_generation(&mut self.next_session_rename_generation, "session rename"),
                    request,
                );
                if self.in_flight_session_catalog_load.is_some() {
                    return self.session_rename_rejected_outcome(
                        correlation,
                        "session rename is unavailable while the session catalog is loading",
                    );
                }
                if self
                    .in_flight_conversation_load
                    .as_ref()
                    .is_some_and(|load| load.requested_thread_id == correlation.request.thread_id)
                {
                    return self.session_rename_rejected_outcome(
                        correlation,
                        "session rename is unavailable while that conversation is loading",
                    );
                }
                self.in_flight_session_rename = Some(correlation.clone());
                CoreDispatchOutcome {
                    events: Vec::new(),
                    effects: vec![CoreEffect::RenameSession { correlation }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::LoadConversation {
                thread_id,
                fallback_workspace_directory,
            }) => {
                if self
                    .in_flight_session_rename
                    .as_ref()
                    .is_some_and(|rename| rename.request.thread_id == thread_id)
                {
                    self.deferred_conversation_load =
                        Some((thread_id, fallback_workspace_directory));
                    return self.unchanged_outcome();
                }
                self.deferred_conversation_load = None;
                self.start_conversation_load(thread_id, fallback_workspace_directory)
            }
            CoreInput::Command(AppCommand::InvalidateConversationLoad) => {
                self.deferred_conversation_load = None;
                self.in_flight_conversation_load = None;
                self.active_turn_submission = None;
                self.active_stop_request = None;
                self.active_turn_steer = None;
                self.active_approval_decision = None;
                self.guarded_session_rename_stream = None;
                self.state.reset_conversation();
                self.turn_stream_state = TurnStreamState::new();
                self.conversation_changed_outcome(None, Vec::new())
            }
            CoreInput::Command(AppCommand::LoadParallelPeekConversation { thread_id }) => {
                let correlation = ParallelPeekLoadCorrelation::new(
                    take_generation(
                        &mut self.next_parallel_peek_load_generation,
                        "parallel peek load",
                    ),
                    thread_id,
                );
                self.active_parallel_peek_load = Some(correlation.clone());
                CoreDispatchOutcome {
                    events: Vec::new(),
                    effects: vec![CoreEffect::LoadParallelPeekConversation { correlation }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::LoadReviewCenter {
                workspace_directory,
                active_thread_id,
            }) => {
                let correlation = ReviewCenterLoadCorrelation::new(
                    take_generation(
                        &mut self.next_review_center_load_generation,
                        "review center load",
                    ),
                    workspace_directory,
                    active_thread_id,
                );
                self.active_review_center_load = Some(correlation.clone());
                CoreDispatchOutcome {
                    events: vec![AppEvent::ReviewCenterLoadStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::LoadReviewCenter { correlation }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::LoadQueueAuthority {
                workspace_directory,
                active_thread_id,
            }) => {
                let correlation = QueueAuthorityLoadCorrelation::new(
                    take_generation(
                        &mut self.next_queue_authority_load_generation,
                        "queue authority load",
                    ),
                    workspace_directory,
                    active_thread_id,
                );
                self.active_queue_authority_load = Some(correlation.clone());
                CoreDispatchOutcome {
                    events: vec![AppEvent::QueueAuthorityLoadStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::LoadQueueAuthority { correlation }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::SubmitQueueMutation(intent)) => {
                if self.active_queue_mutation.is_some() {
                    return self.unchanged_outcome();
                }
                let correlation = QueueMutationCorrelation::new(
                    take_generation(&mut self.next_queue_mutation_generation, "queue mutation"),
                    *intent,
                );
                self.active_queue_mutation = Some(correlation.clone());
                CoreDispatchOutcome {
                    events: vec![AppEvent::QueueMutationStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::ExecuteQueueMutation { correlation }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::PrepareManualPrompt(intent)) => {
                if let Some(active_correlation) = &self.in_flight_manual_prompt_preparation {
                    return CoreDispatchOutcome {
                        events: vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                            ManualPromptPreparationAdmission::RejectedActive {
                                active_correlation: active_correlation.clone(),
                            },
                        )],
                        effects: Vec::new(),
                        snapshot: self.snapshot(),
                    };
                }
                let generation = take_generation(
                    &mut self.next_manual_prompt_preparation_generation,
                    "manual prompt preparation",
                );
                let ManualPromptPreparationIntent {
                    workspace_directory,
                    raw_prompt,
                    parent_thread_id,
                    parent_turn_id,
                } = *intent;
                let correlation = ManualPromptCorrelation {
                    request_id: generation,
                    generation,
                    workspace_directory,
                };
                let request = ManualPromptRequest {
                    correlation: correlation.clone(),
                    raw_prompt,
                    parent_thread_id,
                    parent_turn_id,
                };
                self.in_flight_manual_prompt_preparation = Some(correlation.clone());
                CoreDispatchOutcome {
                    events: vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                        ManualPromptPreparationAdmission::Accepted { correlation },
                    )],
                    effects: vec![CoreEffect::PrepareManualPrompt(Box::new(request))],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::CancelManualPromptPreparation) => {
                self.in_flight_manual_prompt_preparation = None;
                self.unchanged_outcome()
            }
            CoreInput::Command(AppCommand::SubmitTurn(request)) => {
                if let Some(active_correlation) = self.active_turn_submission {
                    return CoreDispatchOutcome {
                        events: vec![AppEvent::TurnSubmissionAdmissionResolved(
                            TurnSubmissionAdmission::RejectedActive { active_correlation },
                        )],
                        effects: Vec::new(),
                        snapshot: self.snapshot(),
                    };
                }
                let correlation = self.begin_turn_submission();
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnSubmissionAdmissionResolved(
                        TurnSubmissionAdmission::Accepted { correlation },
                    )],
                    effects: vec![CoreEffect::SubmitTurn {
                        correlation,
                        request,
                    }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::RequestStopAllSessions) => {
                if let Some(active) = self.active_stop_request {
                    return CoreDispatchOutcome {
                        events: vec![AppEvent::StopRequestAdmissionResolved(
                            StopRequestAdmission::RejectedActive {
                                active_correlation: active.correlation,
                            },
                        )],
                        effects: Vec::new(),
                        snapshot: self.snapshot(),
                    };
                }
                let correlation = StopRequestCorrelation::new(
                    take_generation(
                        &mut self.next_stop_request_generation,
                        "runtime stop request",
                    ),
                    self.active_turn_submission,
                );
                self.active_stop_request = Some(ActiveStopRequest {
                    correlation,
                    pending_attempt: Some(StopRequestAttempt::Initial),
                    synchronize_after_turn_started: correlation.turn_submission.is_some()
                        && !self.turn_stream_state.has_active_turn(),
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::StopRequestAdmissionResolved(
                        StopRequestAdmission::Accepted { correlation },
                    )],
                    effects: vec![CoreEffect::RequestStopAllSessions {
                        correlation,
                        attempt: StopRequestAttempt::Initial,
                    }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::SteerTurn(request)) => {
                if let Some(active) = &self.active_turn_steer {
                    return CoreDispatchOutcome {
                        events: vec![AppEvent::TurnSteerAdmissionResolved(
                            TurnSteerAdmission::RejectedActive {
                                active_correlation: active.correlation,
                            },
                        )],
                        effects: Vec::new(),
                        snapshot: self.snapshot(),
                    };
                }
                let Some(turn_submission) = self.active_turn_submission else {
                    return self.turn_steer_unavailable_outcome();
                };
                if !self
                    .turn_stream_state
                    .matches_active_turn(&request.thread_id, &request.expected_turn_id)
                {
                    return self.turn_steer_unavailable_outcome();
                }
                let correlation = TurnSteerCorrelation::new(
                    take_generation(&mut self.next_turn_steer_generation, "active turn steer"),
                    turn_submission,
                );
                self.active_turn_steer = Some(ActiveTurnSteer {
                    correlation,
                    expected_turn_id: request.expected_turn_id.clone(),
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnSteerAdmissionResolved(
                        TurnSteerAdmission::Accepted { correlation },
                    )],
                    effects: vec![CoreEffect::SteerTurn {
                        correlation,
                        request,
                    }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::SubmitApprovalDecision {
                approval_id,
                decision,
            }) => {
                if let Some(active) = &self.active_approval_decision {
                    return CoreDispatchOutcome {
                        events: vec![AppEvent::ApprovalDecisionAdmissionResolved(
                            ApprovalDecisionAdmission::RejectedActive {
                                active_correlation: active.correlation.clone(),
                            },
                        )],
                        effects: Vec::new(),
                        snapshot: self.snapshot(),
                    };
                }
                let Some(turn_submission) = self.active_turn_submission else {
                    return self.approval_decision_unavailable_outcome();
                };
                if !self
                    .turn_stream_state
                    .matches_pending_approval(&approval_id)
                {
                    return self.approval_decision_unavailable_outcome();
                }
                let correlation = ApprovalDecisionCorrelation::new(
                    take_generation(
                        &mut self.next_approval_decision_generation,
                        "approval decision",
                    ),
                    turn_submission,
                    approval_id,
                    decision,
                );
                self.active_approval_decision = Some(ActiveApprovalDecision {
                    correlation: correlation.clone(),
                    phase: ApprovalDecisionPhase::Submitting,
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::ApprovalDecisionAdmissionResolved(
                        ApprovalDecisionAdmission::Accepted {
                            correlation: correlation.clone(),
                        },
                    )],
                    effects: vec![CoreEffect::SubmitApprovalDecision { correlation }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::ConfigureGithubReviewPolling { target }) => {
                self.github_review_poll_target = target;
                self.github_review_poll_cursor = None;
                self.active_github_review_poll = None;
                self.unchanged_outcome()
            }
            CoreInput::Command(AppCommand::PollGithubReview) => {
                let Some(target) = self.github_review_poll_target.clone() else {
                    return self.unchanged_outcome();
                };
                if self.active_github_review_poll.is_some() {
                    return self.unchanged_outcome();
                }
                let correlation = GithubReviewPollCorrelation::new(
                    take_generation(
                        &mut self.next_github_review_poll_generation,
                        "GitHub review poll",
                    ),
                    target.clone(),
                );
                let previous_state = self
                    .github_review_poll_cursor
                    .as_ref()
                    .filter(|(cursor_target, _)| cursor_target == &target)
                    .map(|(_, state)| state.clone());
                self.active_github_review_poll = Some(correlation.clone());
                CoreDispatchOutcome {
                    events: vec![AppEvent::GithubReviewPollStarted {
                        correlation: correlation.clone(),
                    }],
                    effects: vec![CoreEffect::PollGithubReview {
                        correlation,
                        previous_state,
                    }],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::EvaluatePostTurn(request)) => CoreDispatchOutcome {
                events: Vec::new(),
                effects: vec![CoreEffect::EvaluatePostTurn(request)],
                snapshot: self.snapshot(),
            },
            CoreInput::EffectCompleted(CoreEffectCompletion::StartupChecksLoaded {
                correlation,
                result,
            }) => {
                if self.in_flight_startup_check != Some(correlation) {
                    return self.unchanged_outcome();
                }
                self.in_flight_startup_check = None;
                self.state.apply_startup_result(result);
                self.startup_changed_outcome(correlation, Vec::new())
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::SessionCatalogLoaded {
                correlation,
                result,
            }) => {
                if self.in_flight_session_catalog_load != Some(correlation) {
                    return self.unchanged_outcome();
                }
                self.in_flight_session_catalog_load = None;
                self.state.apply_session_catalog_result(result);
                self.session_catalog_changed_outcome(Vec::new())
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::SessionRenamed {
                correlation,
                result,
            }) => {
                if self.in_flight_session_rename.as_ref() != Some(&correlation) {
                    return self.unchanged_outcome();
                }
                self.in_flight_session_rename = None;
                let result = result.map(|()| {
                    self.state.apply_session_rename(&correlation.request);
                    if self
                        .turn_stream_state
                        .matches_thread(&correlation.request.thread_id)
                        && let Some(turn_correlation) = self.active_turn_submission
                    {
                        self.guarded_session_rename_stream =
                            Some((turn_correlation, correlation.clone()));
                    }
                    SessionRenameAcceptedSnapshot {
                        session_catalog: self.snapshot().session_catalog,
                        turn_stream: self
                            .turn_stream_state
                            .apply_session_rename(
                                &correlation.request.thread_id,
                                &correlation.request.name,
                            )
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
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ConversationLoaded {
                correlation,
                mut result,
            }) => {
                if self.in_flight_conversation_load.as_ref() != Some(&correlation) {
                    return self.unchanged_outcome();
                }
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
                    (Some(ready), Some(Ok(item_lifecycle))) => Some((
                        ready.thread_id.clone(),
                        ready.title.clone(),
                        ready.workspace_directory.clone(),
                        item_lifecycle,
                    )),
                    _ => None,
                };
                self.in_flight_conversation_load = None;
                self.active_turn_submission = None;
                self.active_stop_request = None;
                self.active_turn_steer = None;
                self.active_approval_decision = None;
                self.guarded_session_rename_stream = None;
                self.state.apply_conversation_result(result);
                self.turn_stream_state = TurnStreamState::new();
                if let Some((thread_id, title, cwd, item_lifecycle)) = loaded_stream_identity {
                    self.turn_stream_state.seed_loaded_thread_projection(
                        thread_id,
                        title,
                        cwd,
                        item_lifecycle,
                    );
                }
                self.conversation_changed_outcome(Some(correlation), Vec::new())
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation,
                result,
            }) => {
                if self.active_parallel_peek_load.as_ref() != Some(&correlation) {
                    return self.unchanged_outcome();
                }
                self.active_parallel_peek_load = None;
                CoreDispatchOutcome {
                    events: vec![AppEvent::ParallelPeekConversationLoaded {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ReviewCenterLoaded {
                correlation,
                snapshot,
            }) => {
                if self.active_review_center_load.as_ref() != Some(&correlation) {
                    return self.unchanged_outcome();
                }
                self.active_review_center_load = None;
                CoreDispatchOutcome {
                    events: vec![AppEvent::ReviewCenterLoaded {
                        correlation,
                        snapshot,
                    }],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::QueueAuthorityLoaded {
                correlation,
                result,
            }) => {
                if self.active_queue_authority_load.as_ref() != Some(&correlation) {
                    return self.unchanged_outcome();
                }
                self.active_queue_authority_load = None;
                CoreDispatchOutcome {
                    events: vec![AppEvent::QueueAuthorityLoaded {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::QueueMutationCompleted {
                correlation,
                result,
            }) => {
                if self.active_queue_mutation.as_ref() != Some(&correlation) {
                    return self.unchanged_outcome();
                }
                self.active_queue_mutation = None;
                CoreDispatchOutcome {
                    events: vec![AppEvent::QueueMutationCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt,
                result,
            }) => {
                if self.active_stop_request.is_none_or(|active| {
                    active.correlation != correlation || active.pending_attempt != Some(attempt)
                }) {
                    return self.unchanged_outcome();
                }
                self.active_stop_request
                    .as_mut()
                    .expect("exact active stop request must remain present")
                    .pending_attempt = None;
                let failed = result.is_err();
                if failed || correlation.turn_submission.is_none() {
                    self.active_stop_request = None;
                }
                let mut effects = Vec::new();
                if !failed {
                    self.schedule_stop_synchronization_after_turn_started(&mut effects);
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::StopRequestAttemptCompleted {
                        correlation,
                        attempt,
                        result,
                    }],
                    effects,
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::TurnSteered {
                correlation,
                result,
            }) => {
                if self
                    .active_turn_steer
                    .as_ref()
                    .is_none_or(|active| active.correlation != correlation)
                {
                    return self.unchanged_outcome();
                }
                let active = self
                    .active_turn_steer
                    .take()
                    .expect("exact active turn steer must remain present");
                let result = result.and_then(|receipt| {
                    if receipt.turn_id == active.expected_turn_id {
                        Ok(receipt)
                    } else {
                        Err("turn steer provider returned a different turn".to_string())
                    }
                });
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnSteerCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation,
                result,
            }) => {
                if self.active_approval_decision.as_ref().is_none_or(|active| {
                    active.correlation != correlation
                        || active.phase != ApprovalDecisionPhase::Submitting
                }) {
                    return self.unchanged_outcome();
                }
                if result.is_ok() {
                    self.active_approval_decision
                        .as_mut()
                        .expect("exact active approval decision must remain present")
                        .phase = ApprovalDecisionPhase::Submitted;
                } else {
                    self.active_approval_decision = None;
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::ApprovalDecisionSubmissionCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::GithubReviewPollCompleted {
                correlation,
                mut result,
            }) => {
                if self.active_github_review_poll.as_ref() != Some(&correlation) {
                    return self.unchanged_outcome();
                }
                self.active_github_review_poll = None;
                if result
                    .as_ref()
                    .is_ok_and(|poll| poll.snapshot.target != correlation.target)
                {
                    result =
                        Err("GitHub review poll provider returned a different target".to_string());
                }
                if let Ok(poll) = &result {
                    self.github_review_poll_cursor =
                        Some((correlation.target.clone(), poll.next_state.clone()));
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::GithubReviewPollCompleted {
                        correlation,
                        result,
                    }],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ManualPromptPrepared(result)) => {
                if self.in_flight_manual_prompt_preparation.as_ref() != Some(result.correlation()) {
                    return CoreDispatchOutcome {
                        events: Vec::new(),
                        effects: Vec::new(),
                        snapshot: self.snapshot(),
                    };
                }
                self.in_flight_manual_prompt_preparation = None;
                let snapshot = self.snapshot();
                CoreDispatchOutcome {
                    events: vec![AppEvent::ManualPromptPrepared(result)],
                    effects: Vec::new(),
                    snapshot,
                }
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PostTurnEvaluationCompleted(
                execution,
            )) => {
                if self.in_flight_conversation_load.is_some() {
                    return self.unchanged_outcome();
                }
                let accepted = self
                    .turn_stream_state
                    .accept_post_turn_evaluation_completion(execution.as_ref());
                if !accepted {
                    return self.unchanged_outcome();
                }
                self.state.apply_planning_runtime_projection(Box::new(
                    execution.evaluation.runtime_projection.clone(),
                ));
                CoreDispatchOutcome {
                    events: vec![AppEvent::PostTurnEvaluationCompleted(execution)],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::ConversationStreamUpdated { correlation, event } => {
                self.apply_correlated_turn_stream_event(correlation, event)
            }
            CoreInput::ConversationRuntimeNotice(notice) => {
                let stream_snapshot = self.turn_stream_state.apply_runtime_notice(notice);
                CoreDispatchOutcome {
                    events: vec![AppEvent::turn_stream_snapshot_changed(stream_snapshot)],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::ConversationTurnRuntimeNotice {
                correlation,
                notice,
            } => {
                if self.active_turn_submission != Some(correlation) {
                    return self.unchanged_outcome();
                }
                let stream_snapshot = self.turn_stream_state.apply_runtime_notice(notice);
                CoreDispatchOutcome {
                    events: vec![AppEvent::turn_stream_snapshot_changed(stream_snapshot)],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::ConversationTurnWorkspaceChanged {
                correlation,
                workspace_directory,
            } => {
                if self.active_turn_submission != Some(correlation) {
                    return self.unchanged_outcome();
                }
                CoreDispatchOutcome {
                    events: vec![AppEvent::ConversationTurnWorkspaceChanged {
                        workspace_directory,
                    }],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::ParallelModeSupervisorSnapshotInvalidated => CoreDispatchOutcome {
                events: vec![AppEvent::ParallelModeSupervisorSnapshotInvalidated],
                effects: Vec::new(),
                snapshot: self.snapshot(),
            },
            CoreInput::RuntimeProjectionChanged(projection) => {
                let changed = self.state.apply_planning_runtime_projection(projection);
                self.snapshot_changed_outcome(changed)
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

    fn start_session_catalog_load(
        &mut self,
        limit: usize,
        workspace_directory: String,
    ) -> CoreDispatchOutcome {
        let correlation = SessionCatalogLoadCorrelation::new(take_generation(
            &mut self.next_session_catalog_load_generation,
            "session catalog load",
        ));
        self.in_flight_session_catalog_load = Some(correlation);
        self.state.mark_session_catalog_loading();
        self.session_catalog_changed_outcome(vec![CoreEffect::LoadSessionCatalog {
            correlation,
            limit,
            workspace_directory,
        }])
    }

    fn start_conversation_load(
        &mut self,
        thread_id: String,
        fallback_workspace_directory: String,
    ) -> CoreDispatchOutcome {
        self.active_turn_submission = None;
        self.active_stop_request = None;
        self.active_turn_steer = None;
        self.active_approval_decision = None;
        self.guarded_session_rename_stream = None;
        let correlation = ConversationLoadCorrelation::new(
            take_generation(
                &mut self.next_conversation_load_generation,
                "conversation load",
            ),
            thread_id,
        );
        self.in_flight_conversation_load = Some(correlation.clone());
        self.state.mark_conversation_loading();
        self.turn_stream_state = TurnStreamState::new();
        self.conversation_changed_outcome(
            Some(correlation.clone()),
            vec![CoreEffect::LoadConversation {
                correlation,
                fallback_workspace_directory,
            }],
        )
    }

    fn start_deferred_session_reads(
        &mut self,
        events: &mut Vec<AppEvent>,
        effects: &mut Vec<CoreEffect>,
    ) {
        if let Some((limit, workspace_directory)) = self.deferred_session_catalog_load.take() {
            let outcome = self.start_session_catalog_load(limit, workspace_directory);
            events.extend(outcome.events);
            effects.extend(outcome.effects);
        }
        if let Some((thread_id, fallback_workspace_directory)) =
            self.deferred_conversation_load.take()
        {
            let outcome = self.start_conversation_load(thread_id, fallback_workspace_directory);
            events.extend(outcome.events);
            effects.extend(outcome.effects);
        }
    }

    fn snapshot_changed_outcome(&self, changed: bool) -> CoreDispatchOutcome {
        let snapshot = self.snapshot();
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

    fn begin_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        let correlation = TurnSubmissionCorrelation::new(take_generation(
            &mut self.next_turn_submission_generation,
            "turn submission",
        ));
        self.guarded_session_rename_stream = None;
        self.active_stop_request = None;
        self.active_approval_decision = None;
        self.active_turn_submission = Some(correlation);
        self.turn_stream_state.begin_submission();
        correlation
    }

    fn turn_steer_unavailable_outcome(&self) -> CoreDispatchOutcome {
        CoreDispatchOutcome {
            events: vec![AppEvent::TurnSteerAdmissionResolved(
                TurnSteerAdmission::RejectedUnavailable,
            )],
            effects: Vec::new(),
            snapshot: self.snapshot(),
        }
    }

    fn approval_decision_unavailable_outcome(&self) -> CoreDispatchOutcome {
        CoreDispatchOutcome {
            events: vec![AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedUnavailable,
            )],
            effects: Vec::new(),
            snapshot: self.snapshot(),
        }
    }

    fn apply_correlated_turn_stream_event(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        mut event: TurnStreamEvent,
    ) -> CoreDispatchOutcome {
        if self.active_turn_submission != Some(correlation) {
            return self.unchanged_outcome();
        }

        if let TurnStreamEvent::ThreadPrepared {
            thread_id, title, ..
        } = &mut event
            && let Some(guarded_rename) = self
                .guarded_session_rename_stream
                .as_ref()
                .filter(|(turn, _)| *turn == correlation)
                .map(|(_, rename)| rename.clone())
        {
            if thread_id == &guarded_rename.request.thread_id {
                *title = guarded_rename.request.name;
            }
            self.guarded_session_rename_stream = None;
        }
        let stream_snapshot = self.turn_stream_state.apply_stream_event(event);
        let closes_submission = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnCompleted { .. }
                | TurnStreamUpdate::TurnTerminal { .. }
                | TurnStreamUpdate::Failed { .. }
        );
        let rejected_terminal = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnTerminalIgnored { .. }
        );
        let turn_started = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnStarted { .. }
        );
        let retry_reopens_stop = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnRetrying {
                correlation_failure: None,
                ..
            }
        );
        let mut events = vec![AppEvent::turn_stream_snapshot_changed(stream_snapshot)];
        let mut effects = Vec::new();

        if rejected_terminal {
            let failed = self
                .turn_stream_state
                .apply_stream_event(TurnStreamEvent::Failed {
                    message: "active turn returned a terminal receipt with mismatched identity"
                        .to_string(),
                });
            events.push(AppEvent::turn_stream_snapshot_changed(failed));
            self.clear_stop_request_for_turn(correlation);
            self.active_turn_submission = None;
            self.guarded_session_rename_stream = None;
        } else if closes_submission {
            self.clear_stop_request_for_turn(correlation);
            self.active_turn_submission = None;
            self.guarded_session_rename_stream = None;
        } else if retry_reopens_stop {
            self.clear_stop_request_for_turn(correlation);
        } else if turn_started {
            self.schedule_stop_synchronization_after_turn_started(&mut effects);
        }
        if self
            .active_approval_decision
            .as_ref()
            .is_some_and(|active| {
                self.active_turn_submission != Some(active.correlation.turn_submission)
                    || !self
                        .turn_stream_state
                        .matches_pending_approval(&active.correlation.approval_id)
            })
        {
            self.active_approval_decision = None;
        }

        CoreDispatchOutcome {
            events,
            effects,
            snapshot: self.snapshot(),
        }
    }

    fn schedule_stop_synchronization_after_turn_started(&mut self, effects: &mut Vec<CoreEffect>) {
        let Some(active) = self.active_stop_request.as_mut() else {
            return;
        };
        if !active.synchronize_after_turn_started
            || active.pending_attempt.is_some()
            || active.correlation.turn_submission != self.active_turn_submission
            || !self.turn_stream_state.has_active_turn()
        {
            return;
        }
        active.synchronize_after_turn_started = false;
        active.pending_attempt = Some(StopRequestAttempt::AfterTurnStarted);
        effects.push(CoreEffect::RequestStopAllSessions {
            correlation: active.correlation,
            attempt: StopRequestAttempt::AfterTurnStarted,
        });
    }

    fn clear_stop_request_for_turn(&mut self, correlation: TurnSubmissionCorrelation) {
        if self
            .active_stop_request
            .is_some_and(|active| active.correlation.turn_submission == Some(correlation))
        {
            self.active_stop_request = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        assert!(
            self.active_turn_submission.is_none(),
            "test turn submission must not supersede an active generation"
        );
        self.begin_turn_submission()
    }

    fn unchanged_outcome(&self) -> CoreDispatchOutcome {
        CoreDispatchOutcome {
            events: Vec::new(),
            effects: Vec::new(),
            snapshot: self.snapshot(),
        }
    }

    fn startup_changed_outcome(
        &self,
        correlation: StartupCheckCorrelation,
        effects: Vec<CoreEffect>,
    ) -> CoreDispatchOutcome {
        let snapshot = self.snapshot();
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
        let snapshot = self.snapshot();
        CoreDispatchOutcome {
            events: vec![AppEvent::SessionCatalogChanged(
                snapshot.session_catalog.clone(),
            )],
            effects,
            snapshot,
        }
    }

    fn session_rename_rejected_outcome(
        &self,
        correlation: SessionRenameCorrelation,
        message: &str,
    ) -> CoreDispatchOutcome {
        CoreDispatchOutcome {
            events: vec![AppEvent::SessionRenameCompleted {
                correlation,
                result: Err(message.to_string()),
            }],
            effects: Vec::new(),
            snapshot: self.snapshot(),
        }
    }

    fn conversation_changed_outcome(
        &self,
        correlation: Option<ConversationLoadCorrelation>,
        effects: Vec<CoreEffect>,
    ) -> CoreDispatchOutcome {
        let snapshot = self.snapshot();
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

fn take_generation(next_generation: &mut u64, operation: &str) -> u64 {
    let generation = *next_generation;
    *next_generation = generation
        .checked_add(1)
        .unwrap_or_else(|| panic!("{operation} generation exhausted"));
    generation
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
        ConversationReadySnapshot, ConversationSnapshot, CorePromptOrigin, QueueAuthorityLoadError,
        QueueAuthoritySnapshot, QueueMutationCommitSnapshot, QueueMutationIntent,
        QueueMutationKind, QueueMutationResult, QueueMutationTarget, ReviewCenterSnapshot,
        SessionCatalogReadySnapshot, SessionCatalogSnapshot, TurnSubmissionRequest,
    };
    use crate::core::app::{
        StartupAttachmentSnapshot, StartupDiagnosticSnapshot, StartupReadySnapshot,
        StartupSnapshot, TurnStreamEvent, TurnStreamSnapshot, TurnStreamTerminalSnapshot,
        TurnStreamUpdate,
    };
    use crate::domain::conversation::{
        ConversationApprovalDecision, ConversationApprovalRequest, ConversationApprovalRequestKind,
        ConversationApprovalResolution, ConversationMessage, ConversationMessageKind,
        ConversationSnapshot as DomainConversationSnapshot, ConversationTurnSteerRequest,
    };
    use crate::domain::conversation_item_lifecycle::{
        ConversationItemKind, ConversationItemLifecycleConsistency,
        ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
        ConversationItemLifecycleSource, ConversationItemOutcome,
    };
    use crate::domain::github_review::{
        GithubPullRequestActivitySnapshot, GithubPullRequestPollResult,
    };
    use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeReadinessState};
    use crate::domain::planning::{
        ManualPromptOutcome, ManualPromptRequest, RuntimeProjection, TaskStatus,
        TurnSnapshotCapture,
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
        StartupCheckCorrelation::new(generation)
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

        let outcome = controller.handle_input(CoreInput::Command(AppCommand::Noop));

        assert!(outcome.events.is_empty());
        assert!(outcome.effects.is_empty());
        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert_eq!(controller.snapshot(), AppSnapshot::initial());
    }

    #[test]
    fn run_startup_checks_marks_startup_loading() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::Command(AppCommand::RunStartupChecks));

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
        controller.handle_input(CoreInput::Command(AppCommand::RunStartupChecks));

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
        controller.handle_input(CoreInput::Command(AppCommand::RunStartupChecks));

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
        stale_success.handle_input(CoreInput::Command(AppCommand::RunStartupChecks));
        stale_success.handle_input(CoreInput::Command(AppCommand::RunStartupChecks));

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
        stale_failure.handle_input(CoreInput::Command(AppCommand::RunStartupChecks));
        stale_failure.handle_input(CoreInput::Command(AppCommand::RunStartupChecks));
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
    fn load_session_catalog_marks_session_loading() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::Command(AppCommand::LoadSessionCatalog {
            limit: 10,
            workspace_directory: "/tmp/workspace".to_string(),
        }));

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
                correlation: SessionCatalogLoadCorrelation::new(1),
                limit: 10,
                workspace_directory: "/tmp/workspace".to_string(),
            }]
        );
    }

    #[test]
    fn rename_session_dispatches_core_effect_without_changing_snapshot() {
        let mut controller = CoreController::new();
        let request = SessionRenameRequest::new("thread-1", "Renamed");

        let outcome =
            controller.handle_input(CoreInput::Command(AppCommand::RenameSession(request)));

        assert!(outcome.events.is_empty());
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::RenameSession {
                correlation: session_rename_correlation(1, "thread-1", "Renamed"),
            }]
        );
        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: correlation.clone(),
                result: Ok(()),
            },
        ));

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

        let ConversationSnapshot::Ready(conversation) = outcome.snapshot.conversation else {
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
        assert!(duplicate_command.events.is_empty());
        assert!(duplicate_command.effects.is_empty());

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: session_rename_correlation(99, "thread-alpha", "Renamed"),
                result: Ok(()),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(stale.snapshot, snapshot_before_rename);

        let failed = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionRenamed {
                correlation: correlation.clone(),
                result: Err("provider unavailable".to_string()),
            },
        ));
        assert_eq!(failed.snapshot, snapshot_before_rename);
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
        assert_eq!(duplicate_completion.snapshot, snapshot_before_rename);
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

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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
        catalog_loading.handle_input(CoreInput::Command(AppCommand::LoadSessionCatalog {
            limit: 10,
            workspace_directory: "/tmp/workspace".to_string(),
        }));
        let rejected = catalog_loading.handle_input(CoreInput::Command(AppCommand::RenameSession(
            SessionRenameRequest::new("thread-1", "Renamed"),
        )));
        assert!(rejected.effects.is_empty());
        assert!(matches!(
            rejected.events.as_slice(),
            [AppEvent::SessionRenameCompleted {
                result: Err(message),
                ..
            }] if message.contains("catalog is loading")
        ));

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
            [AppEvent::SessionRenameCompleted {
                result: Err(message),
                ..
            }] if message.contains("conversation is loading")
        ));

        let accepted = same_thread_loading.handle_input(CoreInput::Command(
            AppCommand::RenameSession(SessionRenameRequest::new("thread-2", "Other renamed")),
        ));
        assert!(matches!(
            accepted.effects.as_slice(),
            [CoreEffect::RenameSession { correlation }]
                if correlation.request.thread_id == "thread-2"
        ));
    }

    #[test]
    fn active_session_rename_defers_conflicting_reads_and_allows_other_thread_load() {
        let mut catalog = CoreController::new();
        let catalog_rename = session_rename_correlation(1, "thread-1", "Renamed");
        catalog.handle_input(CoreInput::Command(AppCommand::RenameSession(
            catalog_rename.request.clone(),
        )));
        let deferred = catalog.handle_input(CoreInput::Command(AppCommand::LoadSessionCatalog {
            limit: 10,
            workspace_directory: "/tmp/workspace".to_string(),
        }));
        assert!(deferred.events.is_empty());
        assert!(deferred.effects.is_empty());
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
                correlation: SessionCatalogLoadCorrelation { generation: 1 },
                limit: 10,
                workspace_directory,
            }] if workspace_directory == "/tmp/workspace"
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
        assert_eq!(deferred.snapshot, snapshot_before_rename);
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

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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
        controller.next_queue_mutation_generation = u64::MAX;

        controller.handle_input(CoreInput::Command(AppCommand::SubmitQueueMutation(
            Box::new(queue_mutation_intent(
                "/tmp/workspace",
                Some("thread-1"),
                QueueMutationKind::RemoveSelected,
            )),
        )));
    }

    #[test]
    fn submit_turn_returns_core_effect_without_state_revision() {
        let mut controller = CoreController::new();
        let request = crate::core::app::TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            prompt: "ship it".to_string(),
            prompt_origin: crate::core::app::CorePromptOrigin::Manual,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        };

        let outcome =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(request.clone())));

        assert_eq!(
            outcome.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::Accepted {
                    correlation: TurnSubmissionCorrelation::new(1),
                },
            )]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::SubmitTurn {
                correlation: TurnSubmissionCorrelation::new(1),
                request,
            }]
        );
        assert_eq!(outcome.snapshot, AppSnapshot::initial());
    }

    #[test]
    fn active_turn_submission_rejects_a_second_submit_effect() {
        let mut controller = CoreController::new();
        let first = test_turn_submission_request(Some("thread-1"));
        let second = test_turn_submission_request(Some("thread-1"));

        let first_outcome =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(first)));
        let second_outcome =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(second)));

        assert_eq!(first_outcome.effects.len(), 1);
        assert_eq!(
            first_outcome.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::Accepted {
                    correlation: TurnSubmissionCorrelation::new(1),
                },
            )]
        );
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
        assert_eq!(
            accepted.events,
            vec![AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::Accepted { correlation },
            )]
        );
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
        controller.next_stop_request_generation = u64::MAX;

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
            controller
                .active_turn_steer
                .as_ref()
                .map(|active| active.correlation),
            Some(correlation)
        );

        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::TurnTerminal {
                receipt: confirmed_terminal_receipt("thread-1", "turn-1", Vec::new()),
                execution_snapshot_capture: None,
            },
        ));
        let next_submit = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            test_turn_submission_request(Some("thread-1")),
        )));
        assert_eq!(
            next_submit.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::Accepted {
                    correlation: TurnSubmissionCorrelation::new(2),
                },
            )]
        );
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
        assert!(controller.active_turn_steer.is_none());
    }

    #[test]
    fn approval_decision_is_single_flight_until_the_exact_request_resolves() {
        let mut controller = CoreController::new();
        let unavailable =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                approval_id: "approval-1".to_string(),
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
                approval_id: "approval-other".to_string(),
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
            "approval-1",
            ConversationApprovalDecision::Accept,
        );
        let accepted =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                approval_id: "approval-1".to_string(),
                decision: ConversationApprovalDecision::Accept,
            }));
        assert_eq!(
            accepted.events,
            vec![AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::Accepted {
                    correlation: correlation.clone(),
                },
            )]
        );
        assert_eq!(
            accepted.effects,
            vec![CoreEffect::SubmitApprovalDecision {
                correlation: correlation.clone(),
            }]
        );

        let duplicate =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                approval_id: "approval-1".to_string(),
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
        assert_eq!(
            completed.events,
            vec![AppEvent::ApprovalDecisionSubmissionCompleted {
                correlation: correlation.clone(),
                result: Ok(()),
            }]
        );
        assert_eq!(
            controller
                .active_approval_decision
                .as_ref()
                .map(|active| active.phase),
            Some(ApprovalDecisionPhase::Submitted)
        );

        controller.handle_input(test_turn_stream_input(
            turn_submission,
            TurnStreamEvent::ApprovalResolved {
                approval_id: "approval-stale".to_string(),
                resolution: ConversationApprovalResolution::Declined,
            },
        ));
        let waiting =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                approval_id: "approval-1".to_string(),
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
                approval_id: "approval-1".to_string(),
                resolution: ConversationApprovalResolution::Accepted,
            },
        ));
        assert!(controller.active_approval_decision.is_none());
        let after_resolution =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                approval_id: "approval-1".to_string(),
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
            "approval-1",
            ConversationApprovalDecision::Accept,
        );
        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-1".to_string(),
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
            [AppEvent::ApprovalDecisionSubmissionCompleted {
                result: Err(message),
                ..
            }] if message == "runtime unavailable"
        ));
        assert!(controller.active_approval_decision.is_none());

        let retried =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
                approval_id: "approval-1".to_string(),
                decision: ConversationApprovalDecision::Decline,
            }));
        assert!(matches!(
            retried.effects.as_slice(),
            [CoreEffect::SubmitApprovalDecision { correlation }]
                if correlation.generation == 2
                    && correlation.turn_submission == turn_submission
                    && correlation.approval_id == "approval-1"
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
            "approval-a",
            ConversationApprovalDecision::Accept,
        );
        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-a".to_string(),
            decision: ConversationApprovalDecision::Accept,
        }));

        request_test_approval(&mut controller, turn_submission, "approval-b");
        assert!(controller.active_approval_decision.is_none());
        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-b".to_string(),
            decision: ConversationApprovalDecision::Accept,
        }));
        request_test_approval(&mut controller, turn_submission, "approval-a");
        assert!(controller.active_approval_decision.is_none());
        let current = ApprovalDecisionCorrelation::new(
            3,
            turn_submission,
            "approval-a",
            ConversationApprovalDecision::Accept,
        );
        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-a".to_string(),
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
                .active_approval_decision
                .as_ref()
                .map(|active| &active.correlation),
            Some(&current)
        );

        let accepted = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation: current.clone(),
                result: Ok(()),
            },
        ));
        assert_eq!(
            accepted.events,
            vec![AppEvent::ApprovalDecisionSubmissionCompleted {
                correlation: current,
                result: Ok(()),
            }]
        );
    }

    #[test]
    fn terminal_and_conversation_invalidation_drop_approval_completions() {
        let mut terminal = CoreController::new();
        let terminal_turn = start_test_turn(&mut terminal, "thread-1", "turn-1");
        request_test_approval(&mut terminal, terminal_turn, "approval-terminal");
        let terminal_correlation = ApprovalDecisionCorrelation::new(
            1,
            terminal_turn,
            "approval-terminal",
            ConversationApprovalDecision::Accept,
        );
        terminal.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-terminal".to_string(),
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
        assert!(terminal.active_approval_decision.is_none());

        let mut invalidated = CoreController::new();
        let invalidated_turn = start_test_turn(&mut invalidated, "thread-1", "turn-1");
        request_test_approval(&mut invalidated, invalidated_turn, "approval-invalidated");
        let invalidated_correlation = ApprovalDecisionCorrelation::new(
            1,
            invalidated_turn,
            "approval-invalidated",
            ConversationApprovalDecision::Decline,
        );
        invalidated.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-invalidated".to_string(),
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
        assert!(invalidated.active_approval_decision.is_none());
    }

    #[test]
    #[should_panic(expected = "approval decision generation exhausted")]
    fn approval_decision_generation_panics_before_it_can_wrap() {
        let mut controller = CoreController::new();
        let turn_submission = start_test_turn(&mut controller, "thread-1", "turn-1");
        request_test_approval(&mut controller, turn_submission, "approval-1");
        controller.next_approval_decision_generation = u64::MAX;

        controller.handle_input(CoreInput::Command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-1".to_string(),
            decision: ConversationApprovalDecision::Accept,
        }));
    }

    #[test]
    fn github_review_poll_runs_one_generation_for_the_configured_target() {
        let mut controller = CoreController::new();
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let configured = controller.handle_input(CoreInput::Command(
            AppCommand::ConfigureGithubReviewPolling {
                target: Some(target.clone()),
            },
        ));
        assert!(configured.events.is_empty());
        assert!(configured.effects.is_empty());

        let correlation = GithubReviewPollCorrelation::new(1, target);
        let started = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert_eq!(
            started.events,
            vec![AppEvent::GithubReviewPollStarted {
                correlation: correlation.clone(),
            }]
        );
        assert_eq!(
            started.effects,
            vec![CoreEffect::PollGithubReview {
                correlation,
                previous_state: None,
            }]
        );

        let duplicate = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn github_review_poll_success_advances_cursor_and_failure_preserves_it() {
        let mut controller = CoreController::new();
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        controller.handle_input(CoreInput::Command(
            AppCommand::ConfigureGithubReviewPolling {
                target: Some(target.clone()),
            },
        ));
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
    fn github_review_poll_reconfigure_starts_a_new_same_target_epoch() {
        let mut controller = CoreController::new();
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        controller.handle_input(CoreInput::Command(
            AppCommand::ConfigureGithubReviewPolling {
                target: Some(target.clone()),
            },
        ));
        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        let first = GithubReviewPollCorrelation::new(1, target.clone());
        let cursor = github_review_poll_result(&target, "2026-07-19T10:00:00Z");
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: first,
                result: Ok(cursor),
            },
        ));
        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        let superseded = GithubReviewPollCorrelation::new(2, target.clone());

        controller.handle_input(CoreInput::Command(
            AppCommand::ConfigureGithubReviewPolling {
                target: Some(target.clone()),
            },
        ));
        let current = GithubReviewPollCorrelation::new(3, target.clone());
        let restarted = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert_eq!(
            restarted.effects,
            vec![CoreEffect::PollGithubReview {
                correlation: current.clone(),
                previous_state: None,
            }]
        );

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: superseded,
                result: Ok(github_review_poll_result(&target, "2026-07-19T10:01:00Z")),
            },
        ));
        assert!(stale.events.is_empty());

        controller.handle_input(CoreInput::Command(
            AppCommand::ConfigureGithubReviewPolling { target: None },
        ));
        let disabled = controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
        assert!(disabled.events.is_empty());
        assert!(disabled.effects.is_empty());
        let late = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::GithubReviewPollCompleted {
                correlation: current,
                result: Ok(github_review_poll_result(&target, "2026-07-19T10:02:00Z")),
            },
        ));
        assert!(late.events.is_empty());
    }

    #[test]
    fn github_review_poll_rejects_a_wrong_provider_target_without_losing_cursor() {
        let mut controller = CoreController::new();
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        controller.handle_input(CoreInput::Command(
            AppCommand::ConfigureGithubReviewPolling {
                target: Some(target.clone()),
            },
        ));
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
                correlation: GithubReviewPollCorrelation::new(3, target),
                previous_state: Some(previous.next_state),
            }]
        );
    }

    #[test]
    #[should_panic(expected = "GitHub review poll generation exhausted")]
    fn github_review_poll_generation_panics_before_it_can_wrap() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(
            AppCommand::ConfigureGithubReviewPolling {
                target: Some(GithubPullRequestTarget::new("acme/widgets", 42)),
            },
        ));
        controller.next_github_review_poll_generation = u64::MAX;

        controller.handle_input(CoreInput::Command(AppCommand::PollGithubReview));
    }

    #[test]
    fn next_submission_recovers_pre_start_failure_and_ignores_stale_worker_inputs() {
        let mut controller = CoreController::new();
        let old_correlation = apply_completed_turn(&mut controller, "thread-1", "turn-1");
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
        assert!(matches!(
            current_failure.events.as_slice(),
            [AppEvent::TurnStreamSnapshotChanged(snapshot)]
                if matches!(snapshot.update, TurnStreamUpdate::Failed { .. })
        ));

        let next = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            test_turn_submission_request(Some("thread-1")),
        )));
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
            [AppEvent::TurnStreamSnapshotChanged(snapshot)]
                if matches!(
                    &snapshot.update,
                    TurnStreamUpdate::TurnTerminal { receipt: projected, status_text, .. }
                        if projected.as_ref() == &receipt && status_text == "turn recovery pending"
                )
        ));
        assert!(controller.active_turn_submission.is_none());
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
        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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
    fn cancelling_manual_prompt_preparation_reopens_dispatch_and_drops_late_completion() {
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
        assert!(cancelled.effects.is_empty());
        assert!(controller.in_flight_manual_prompt_preparation.is_none());

        let second_correlation = manual_prompt_correlation(2, "/tmp/other-workspace");
        let second = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(manual_prompt_intent(
                "/tmp/other-workspace",
                "new workspace prompt",
            )),
        )));
        assert_eq!(
            second.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted {
                    correlation: second_correlation.clone(),
                },
            )]
        );
        assert!(matches!(
            second.effects.as_slice(),
            [CoreEffect::PrepareManualPrompt(request)]
                if request.correlation == second_correlation
        ));

        let late = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(Box::new(ManualPromptOutcome::Rejected {
                correlation: first_correlation,
                transcript_text: "old workspace prompt".to_string(),
                runtime_projection: Box::new(PlanningRuntimeProjection::invalid("stale")),
                reason: "late completion".to_string(),
            })),
        ));
        assert!(late.events.is_empty());
        assert_eq!(
            controller.in_flight_manual_prompt_preparation,
            Some(second_correlation.clone())
        );

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
        assert!(controller.in_flight_manual_prompt_preparation.is_none());
    }

    #[test]
    fn session_catalog_completion_marks_ready_and_drops_older_results() {
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

        for workspace_directory in ["/tmp/older", "/tmp/current"] {
            controller.handle_input(CoreInput::Command(AppCommand::LoadSessionCatalog {
                limit: 10,
                workspace_directory: workspace_directory.to_string(),
            }));
        }
        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: SessionCatalogLoadCorrelation::new(2),
                result: Ok(ready.clone()),
            },
        ));

        assert_eq!(outcome.snapshot.revision, 3);
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

        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: SessionCatalogLoadCorrelation::new(1),
                result: Err("stale catalog".to_string()),
            },
        ));
        assert!(stale.events.is_empty());
        assert_eq!(
            stale.snapshot.session_catalog,
            SessionCatalogSnapshot::Ready(ready)
        );
    }

    #[test]
    fn session_catalog_completion_marks_failed() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::Command(AppCommand::LoadSessionCatalog {
            limit: 10,
            workspace_directory: "/tmp/workspace".to_string(),
        }));

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: SessionCatalogLoadCorrelation::new(1),
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

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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
        assert_eq!(accepted.snapshot, AppSnapshot::initial());

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
            controller.active_queue_mutation,
            Some(first_correlation.clone())
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
            controller.active_queue_mutation,
            Some(second_correlation.clone())
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
    fn conversation_stream_event_reduces_to_core_snapshot_without_state_revision() {
        let mut controller = CoreController::new();
        let turn_correlation = controller.begin_test_turn_submission();
        let stream_event = TurnStreamEvent::StatusUpdated {
            text: "thinking".to_string(),
        };

        let outcome =
            controller.handle_input(test_turn_stream_input(turn_correlation, stream_event));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        let [AppEvent::TurnStreamSnapshotChanged(snapshot)] = outcome.events.as_slice() else {
            panic!("typed completion should produce one stream snapshot");
        };
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

        let outcome = controller.handle_input(CoreInput::ConversationRuntimeNotice(
            "reattached runtime".to_string(),
        ));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(execution.clone()),
        ));

        assert_eq!(
            outcome.events,
            vec![AppEvent::PostTurnEvaluationCompleted(execution)]
        );
    }

    #[test]
    fn accepted_post_turn_evaluation_updates_core_planning_projection() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let execution = Box::new(sample_post_turn_execution());

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(execution.clone()),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            PlanningRuntimeProjection::invalid("planning blocked")
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::PostTurnEvaluationCompleted(execution)]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn matching_post_turn_projection_keeps_revision_and_delivers_completion() {
        let mut controller = CoreController::new();
        let projection = PlanningRuntimeProjection::invalid("planning blocked");
        controller.handle_input(CoreInput::RuntimeProjectionChanged(Box::new(
            projection.clone(),
        )));
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let execution = Box::new(sample_post_turn_execution());

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(execution.clone()),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            projection
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::PostTurnEvaluationCompleted(execution)]
        );
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
        controller.handle_input(CoreInput::RuntimeProjectionChanged(Box::new(
            current_projection.clone(),
        )));
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let load = controller.handle_input(CoreInput::Command(AppCommand::LoadConversation {
            thread_id: "thread-2".to_string(),
            fallback_workspace_directory: "/tmp/workspace".to_string(),
        }));
        let [CoreEffect::LoadConversation { correlation, .. }] = load.effects.as_slice() else {
            panic!("conversation load should emit one correlated effect");
        };
        let correlation = correlation.clone();
        let snapshot_before_completion = controller.snapshot();

        let dropped = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(Box::new(
                sample_post_turn_execution(),
            )),
        ));

        assert_eq!(dropped.snapshot, snapshot_before_completion);
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
        controller.handle_input(CoreInput::RuntimeProjectionChanged(Box::new(
            existing_projection,
        )));
        apply_completed_turn(&mut controller, "thread-1", "turn-2");
        let snapshot_before_stale_completion = controller.snapshot();
        let mut execution = sample_post_turn_execution();
        execution.completed_turn_id = "turn-1".to_string();
        execution.evaluation.provenance =
            crate::application::service::post_turn_evaluation::PostTurnEvaluationProvenance::new(
                "turn-1".to_string(),
            );

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(Box::new(execution)),
        ));

        assert_eq!(outcome.snapshot, snapshot_before_stale_completion);
        assert!(outcome.events.is_empty());
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn duplicate_post_turn_evaluation_completion_is_dropped_in_core() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let execution = Box::new(sample_post_turn_execution());
        let mut duplicate_execution = (*execution).clone();
        duplicate_execution.evaluation.runtime_projection = PlanningRuntimeProjection::ready(
            "duplicate prompt".to_string(),
            "duplicate summary".to_string(),
            None,
        );

        let first = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(execution),
        ));
        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(Box::new(duplicate_execution)),
        ));

        assert_eq!(first.events.len(), 1);
        assert_eq!(duplicate.snapshot, first.snapshot);
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

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert_eq!(outcome.events, vec![AppEvent::ManualPromptPrepared(result)]);
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn planning_parallel_projection_changes_appear_in_app_snapshot() {
        let mut controller = CoreController::new();
        let planning_projection =
            PlanningRuntimeProjection::invalid("planning validation failed in projection");

        let outcome = controller.handle_input(CoreInput::RuntimeProjectionChanged(Box::new(
            planning_projection.clone(),
        )));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            planning_projection
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::SnapshotChanged(outcome.snapshot.clone())]
        );
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
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn repeated_projection_input_does_not_advance_snapshot_revision() {
        let mut controller = CoreController::new();
        let planning_projection =
            PlanningRuntimeProjection::invalid("planning validation failed in projection");
        controller.handle_input(CoreInput::RuntimeProjectionChanged(Box::new(
            planning_projection.clone(),
        )));

        let outcome = controller.handle_input(CoreInput::RuntimeProjectionChanged(Box::new(
            planning_projection,
        )));

        assert_eq!(outcome.snapshot.revision, 1);
        assert!(outcome.events.is_empty());
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_workspace_change_passes_through_core_without_state_revision() {
        let mut controller = CoreController::new();
        let turn_correlation = controller.begin_test_turn_submission();

        let outcome = controller.handle_input(CoreInput::ConversationTurnWorkspaceChanged {
            correlation: turn_correlation,
            workspace_directory: "/tmp/slot-worktree".to_string(),
        });

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            outcome.events,
            vec![AppEvent::ConversationTurnWorkspaceChanged {
                workspace_directory: "/tmp/slot-worktree".to_string()
            }]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn parallel_supervisor_invalidation_passes_through_core_without_state_revision() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::ParallelModeSupervisorSnapshotInvalidated);

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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
        controller.handle_input(CoreInput::Command(AppCommand::LoadSessionCatalog {
            limit: 10,
            workspace_directory: "/tmp/workspace".to_string(),
        }));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: SessionCatalogLoadCorrelation::new(1),
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

    fn test_turn_submission_request(thread_id: Option<&str>) -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: thread_id.map(str::to_string),
            prompt: "ship it".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        }
    }

    fn submit_test_turn(
        controller: &mut CoreController,
        thread_id: Option<&str>,
    ) -> TurnSubmissionCorrelation {
        let outcome = controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(
            test_turn_submission_request(thread_id),
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
}
