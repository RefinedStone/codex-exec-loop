use super::{
    AppCommand, AppEvent, AppSnapshot, AppState, ConversationLoadCorrelation, CoreEffect,
    CoreEffectCompletion, CoreInput, StartupCheckCorrelation, TurnStreamEvent, TurnStreamState,
    TurnStreamUpdate, TurnSubmissionCorrelation,
};
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleProjection;
use crate::domain::planning::ManualPromptCorrelation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDispatchOutcome {
    pub events: Vec<AppEvent>,
    pub effects: Vec<CoreEffect>,
    pub snapshot: AppSnapshot,
}

#[derive(Debug, Clone)]
pub struct CoreController {
    state: AppState,
    turn_stream_state: TurnStreamState,
    next_startup_check_generation: u64,
    in_flight_startup_check: Option<StartupCheckCorrelation>,
    next_conversation_load_generation: u64,
    in_flight_conversation_load: Option<ConversationLoadCorrelation>,
    in_flight_manual_prompt_preparation: Option<ManualPromptCorrelation>,
    next_turn_submission_generation: u64,
    active_turn_submission: Option<TurnSubmissionCorrelation>,
}

impl CoreController {
    pub fn new() -> Self {
        Self {
            state: AppState::new(),
            turn_stream_state: TurnStreamState::new(),
            next_startup_check_generation: 1,
            in_flight_startup_check: None,
            next_conversation_load_generation: 1,
            in_flight_conversation_load: None,
            in_flight_manual_prompt_preparation: None,
            next_turn_submission_generation: 1,
            active_turn_submission: None,
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
                self.state.mark_session_catalog_loading();
                self.session_catalog_changed_outcome(vec![CoreEffect::LoadSessionCatalog {
                    limit,
                    workspace_directory,
                }])
            }
            CoreInput::Command(AppCommand::LoadConversation {
                thread_id,
                fallback_workspace_directory,
            }) => {
                self.active_turn_submission = None;
                let correlation = ConversationLoadCorrelation::new(
                    take_generation(
                        &mut self.next_conversation_load_generation,
                        "conversation load",
                    ),
                    thread_id,
                );
                self.in_flight_conversation_load = Some(correlation.clone());
                self.state.mark_conversation_loading();
                self.conversation_changed_outcome(
                    Some(correlation.clone()),
                    vec![CoreEffect::LoadConversation {
                        correlation,
                        fallback_workspace_directory,
                    }],
                )
            }
            CoreInput::Command(AppCommand::InvalidateConversationLoad) => {
                self.in_flight_conversation_load = None;
                self.active_turn_submission = None;
                self.state.reset_conversation();
                self.turn_stream_state = TurnStreamState::new();
                self.conversation_changed_outcome(None, Vec::new())
            }
            CoreInput::Command(AppCommand::LoadParallelPeekConversation {
                request_id,
                thread_id,
            }) => CoreDispatchOutcome {
                events: Vec::new(),
                effects: vec![CoreEffect::LoadParallelPeekConversation {
                    request_id,
                    thread_id,
                }],
                snapshot: self.snapshot(),
            },
            CoreInput::Command(AppCommand::PrepareManualPrompt(request)) => {
                if self.in_flight_manual_prompt_preparation.is_some() {
                    return CoreDispatchOutcome {
                        events: Vec::new(),
                        effects: Vec::new(),
                        snapshot: self.snapshot(),
                    };
                }
                self.in_flight_manual_prompt_preparation = Some(request.correlation.clone());
                CoreDispatchOutcome {
                    events: Vec::new(),
                    effects: vec![CoreEffect::PrepareManualPrompt(request)],
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::Command(AppCommand::CancelManualPromptPreparation) => {
                self.in_flight_manual_prompt_preparation = None;
                self.unchanged_outcome()
            }
            CoreInput::Command(AppCommand::SubmitTurn(request)) => {
                if self.active_turn_submission.is_some() {
                    return self.unchanged_outcome();
                }
                let correlation = self.begin_turn_submission();
                CoreDispatchOutcome {
                    events: Vec::new(),
                    effects: vec![CoreEffect::SubmitTurn {
                        correlation,
                        request,
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
            CoreInput::EffectCompleted(CoreEffectCompletion::SessionCatalogLoaded(result)) => {
                self.state.apply_session_catalog_result(result);
                self.session_catalog_changed_outcome(Vec::new())
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
                request_id,
                thread_id,
                result,
            }) => CoreDispatchOutcome {
                events: vec![AppEvent::ParallelPeekConversationLoaded {
                    request_id,
                    thread_id,
                    result,
                }],
                effects: Vec::new(),
                snapshot: self.snapshot(),
            },
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
                let events = self
                    .turn_stream_state
                    .accept_post_turn_evaluation_completion(execution.as_ref())
                    .then_some(AppEvent::PostTurnEvaluationCompleted(execution))
                    .into_iter()
                    .collect();
                CoreDispatchOutcome {
                    events,
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
        self.active_turn_submission = Some(correlation);
        self.turn_stream_state.begin_submission();
        correlation
    }

    fn apply_correlated_turn_stream_event(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        event: TurnStreamEvent,
    ) -> CoreDispatchOutcome {
        if self.active_turn_submission != Some(correlation) {
            return self.unchanged_outcome();
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
        let mut events = vec![AppEvent::turn_stream_snapshot_changed(stream_snapshot)];

        if rejected_terminal {
            let failed = self
                .turn_stream_state
                .apply_stream_event(TurnStreamEvent::Failed {
                    message: "active turn returned a terminal receipt with mismatched identity"
                        .to_string(),
                });
            events.push(AppEvent::turn_stream_snapshot_changed(failed));
            self.active_turn_submission = None;
        } else if closes_submission {
            self.active_turn_submission = None;
        }

        CoreDispatchOutcome {
            events,
            effects: Vec::new(),
            snapshot: self.snapshot(),
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
    use crate::application::service::manual_prompt_preparation::{
        ManualPromptPreparationRequest, ManualPromptPreparationResult,
    };
    use crate::application::service::planning::PlanningRuntimeProjection;
    use crate::core::app::{
        ConversationReadySnapshot, ConversationSnapshot, CorePromptOrigin,
        SessionCatalogReadySnapshot, SessionCatalogSnapshot, TurnSubmissionRequest,
    };
    use crate::core::app::{
        StartupAttachmentSnapshot, StartupDiagnosticSnapshot, StartupReadySnapshot,
        StartupSnapshot, TurnStreamEvent, TurnStreamSnapshot, TurnStreamTerminalSnapshot,
        TurnStreamUpdate,
    };
    use crate::domain::conversation::{
        ConversationMessage, ConversationMessageKind,
        ConversationSnapshot as DomainConversationSnapshot,
    };
    use crate::domain::conversation_item_lifecycle::{
        ConversationItemKind, ConversationItemLifecycleConsistency,
        ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
        ConversationItemLifecycleSource, ConversationItemOutcome,
    };
    use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeReadinessState};
    use crate::domain::planning::TurnSnapshotCapture;
    use crate::domain::recent_sessions::RecentSessions;

    fn manual_prompt_correlation() -> crate::domain::planning::ManualPromptCorrelation {
        crate::domain::planning::ManualPromptCorrelation {
            request_id: 1,
            generation: 1,
            workspace_directory: "/tmp/workspace".to_string(),
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
                limit: 10,
                workspace_directory: "/tmp/workspace".to_string(),
            }]
        );
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
    fn parallel_peek_load_dispatches_effect_without_replacing_active_conversation() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::Command(
            AppCommand::LoadParallelPeekConversation {
                request_id: 7,
                thread_id: "thread-peek".to_string(),
            },
        ));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert!(outcome.events.is_empty());
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadParallelPeekConversation {
                request_id: 7,
                thread_id: "thread-peek".to_string(),
            }]
        );
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

        assert!(outcome.events.is_empty());
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
        assert!(second_outcome.events.is_empty());
        assert!(second_outcome.effects.is_empty());
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
        let request = ManualPromptPreparationRequest {
            correlation: manual_prompt_correlation(),
            raw_prompt: "ship it".to_string(),
            parent_thread_id: Some("thread-1".to_string()),
            parent_turn_id: Some("turn-1".to_string()),
        };

        let outcome = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(request.clone()),
        )));

        assert!(outcome.events.is_empty());
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::PrepareManualPrompt(Box::new(request))]
        );
        assert_eq!(outcome.snapshot, AppSnapshot::initial());
    }

    #[test]
    fn manual_prompt_preparation_dispatch_and_completion_are_exactly_once() {
        let mut controller = CoreController::new();
        let correlation = manual_prompt_correlation();
        let request = ManualPromptPreparationRequest {
            correlation: correlation.clone(),
            raw_prompt: "ship it".to_string(),
            parent_thread_id: None,
            parent_turn_id: None,
        };

        let first = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(request.clone()),
        )));
        let duplicate = controller.handle_input(CoreInput::Command(
            AppCommand::PrepareManualPrompt(Box::new(request)),
        ));
        let mut other_correlation = correlation.clone();
        other_correlation.request_id += 1;
        let overlapping = controller.handle_input(CoreInput::Command(
            AppCommand::PrepareManualPrompt(Box::new(ManualPromptPreparationRequest {
                correlation: other_correlation.clone(),
                raw_prompt: "other".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            })),
        ));

        assert_eq!(first.effects.len(), 1);
        assert!(duplicate.effects.is_empty());
        assert!(overlapping.effects.is_empty());

        let stale_result = Box::new(ManualPromptPreparationResult::Rejected {
            correlation: other_correlation.clone(),
            transcript_text: "other".to_string(),
            runtime_projection: Box::new(PlanningRuntimeProjection::invalid("stale")),
            reason: "stale".to_string(),
        });
        let stale = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(stale_result),
        ));
        assert!(stale.events.is_empty());

        let matching_result = Box::new(ManualPromptPreparationResult::Rejected {
            correlation,
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
            Box::new(ManualPromptPreparationRequest {
                correlation: other_correlation,
                raw_prompt: "other".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            }),
        )));
        assert_eq!(next.effects.len(), 1);
    }

    #[test]
    fn cancelling_manual_prompt_preparation_reopens_dispatch_and_drops_late_completion() {
        let mut controller = CoreController::new();
        let first_correlation = manual_prompt_correlation();
        let first_request = ManualPromptPreparationRequest {
            correlation: first_correlation.clone(),
            raw_prompt: "old workspace prompt".to_string(),
            parent_thread_id: None,
            parent_turn_id: None,
        };
        let _ = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(first_request),
        )));

        let cancelled = controller.handle_input(CoreInput::Command(
            AppCommand::CancelManualPromptPreparation,
        ));
        assert!(cancelled.events.is_empty());
        assert!(cancelled.effects.is_empty());
        assert!(controller.in_flight_manual_prompt_preparation.is_none());

        let mut second_correlation = first_correlation.clone();
        second_correlation.request_id += 1;
        second_correlation.generation += 1;
        second_correlation.workspace_directory = "/tmp/other-workspace".to_string();
        let second = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(ManualPromptPreparationRequest {
                correlation: second_correlation.clone(),
                raw_prompt: "new workspace prompt".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            }),
        )));
        assert_eq!(second.effects.len(), 1);

        let late = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(Box::new(
                ManualPromptPreparationResult::Rejected {
                    correlation: first_correlation,
                    transcript_text: "old workspace prompt".to_string(),
                    runtime_projection: Box::new(PlanningRuntimeProjection::invalid("stale")),
                    reason: "late completion".to_string(),
                },
            )),
        ));
        assert!(late.events.is_empty());
        assert_eq!(
            controller.in_flight_manual_prompt_preparation,
            Some(second_correlation.clone())
        );

        let current = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(Box::new(
                ManualPromptPreparationResult::Rejected {
                    correlation: second_correlation,
                    transcript_text: "new workspace prompt".to_string(),
                    runtime_projection: Box::new(PlanningRuntimeProjection::invalid("blocked")),
                    reason: "current completion".to_string(),
                },
            )),
        ));
        assert!(matches!(
            current.events.as_slice(),
            [AppEvent::ManualPromptPrepared(_)]
        ));
        assert!(controller.in_flight_manual_prompt_preparation.is_none());
    }

    #[test]
    fn session_catalog_completion_marks_ready() {
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

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded(Ok(ready.clone())),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            outcome.snapshot.session_catalog,
            SessionCatalogSnapshot::Ready(ready.clone())
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::SessionCatalogChanged(
                SessionCatalogSnapshot::Ready(ready)
            )]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn session_catalog_completion_marks_failed() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded(Err("catalog unavailable".to_string())),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
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
    fn parallel_peek_completion_passes_through_without_mutating_core_state() {
        let mut controller = CoreController::new();
        let ready = sample_conversation_ready_snapshot();

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                request_id: 7,
                thread_id: "thread-peek".to_string(),
                result: Ok(Box::new(ready.clone())),
            },
        ));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            outcome.events,
            vec![AppEvent::ParallelPeekConversationLoaded {
                request_id: 7,
                thread_id: "thread-peek".to_string(),
                result: Ok(Box::new(ready)),
            }]
        );
        assert!(outcome.effects.is_empty());
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
    fn post_turn_evaluation_completion_passes_through_core_without_state_revision() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let execution = Box::new(sample_post_turn_execution());

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(execution.clone()),
        ));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            outcome.events,
            vec![AppEvent::PostTurnEvaluationCompleted(execution)]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn stale_post_turn_evaluation_completion_is_dropped_in_core() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-2");
        let mut execution = sample_post_turn_execution();
        execution.completed_turn_id = "turn-1".to_string();
        execution.evaluation.provenance =
            crate::application::service::post_turn_evaluation::PostTurnEvaluationProvenance::new(
                "turn-1".to_string(),
            );

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(Box::new(execution)),
        ));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert!(outcome.events.is_empty());
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn duplicate_post_turn_evaluation_completion_is_dropped_in_core() {
        let mut controller = CoreController::new();
        apply_completed_turn(&mut controller, "thread-1", "turn-1");
        let execution = Box::new(sample_post_turn_execution());

        let first = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(execution.clone()),
        ));
        let duplicate = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(execution),
        ));

        assert_eq!(first.events.len(), 1);
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn manual_prompt_preparation_completion_defers_projection_until_tui_accepts_correlation() {
        let mut controller = CoreController::new();
        let correlation = manual_prompt_correlation();
        let _ = controller.handle_input(CoreInput::Command(AppCommand::PrepareManualPrompt(
            Box::new(ManualPromptPreparationRequest {
                correlation: correlation.clone(),
                raw_prompt: "ship it".to_string(),
                parent_thread_id: None,
                parent_turn_id: None,
            }),
        )));
        let result = Box::new(ManualPromptPreparationResult::Rejected {
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
