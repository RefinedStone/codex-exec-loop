use super::{
    AppCommand, AppEvent, AppSnapshot, AppState, CoreEffect, CoreEffectCompletion, CoreInput,
    TurnStreamState,
};

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
}

impl CoreController {
    pub fn new() -> Self {
        Self {
            state: AppState::new(),
            turn_stream_state: TurnStreamState::new(),
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
                self.state.mark_startup_loading();
                self.startup_changed_outcome(vec![CoreEffect::RunStartupChecks])
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
            CoreInput::Command(AppCommand::LoadConversation { thread_id }) => {
                self.state.mark_conversation_loading();
                self.conversation_changed_outcome(vec![CoreEffect::LoadConversation { thread_id }])
            }
            CoreInput::Command(AppCommand::SubmitTurn(request)) => CoreDispatchOutcome {
                events: Vec::new(),
                effects: vec![CoreEffect::SubmitTurn(request)],
                snapshot: self.snapshot(),
            },
            CoreInput::EffectCompleted(CoreEffectCompletion::StartupChecksLoaded(result)) => {
                self.state.apply_startup_result(result);
                self.startup_changed_outcome(Vec::new())
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::SessionCatalogLoaded(result)) => {
                self.state.apply_session_catalog_result(result);
                self.session_catalog_changed_outcome(Vec::new())
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::ConversationLoaded(result)) => {
                self.state.apply_conversation_result(result);
                self.turn_stream_state = TurnStreamState::new();
                self.conversation_changed_outcome(Vec::new())
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::PostTurnEvaluationCompleted(
                completion,
            )) => CoreDispatchOutcome {
                events: vec![AppEvent::PostTurnEvaluationCompleted(completion)],
                effects: Vec::new(),
                snapshot: self.snapshot(),
            },
            CoreInput::ConversationStreamUpdated(event) => {
                let stream_snapshot = self.turn_stream_state.apply_stream_event(event);
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnStreamSnapshotChanged(stream_snapshot)],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::ConversationTurnCompleted {
                turn_id,
                changed_planning_file_paths,
                execution_snapshot_capture,
            } => {
                let stream_snapshot = self.turn_stream_state.apply_turn_completed(
                    turn_id,
                    changed_planning_file_paths,
                    execution_snapshot_capture,
                );
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnStreamSnapshotChanged(stream_snapshot)],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::ConversationRuntimeNotice(notice) => {
                let stream_snapshot = self.turn_stream_state.apply_runtime_notice(notice);
                CoreDispatchOutcome {
                    events: vec![AppEvent::TurnStreamSnapshotChanged(stream_snapshot)],
                    effects: Vec::new(),
                    snapshot: self.snapshot(),
                }
            }
            CoreInput::ConversationTurnWorkspaceChanged {
                workspace_directory,
            } => CoreDispatchOutcome {
                events: vec![AppEvent::ConversationTurnWorkspaceChanged {
                    workspace_directory,
                }],
                effects: Vec::new(),
                snapshot: self.snapshot(),
            },
            CoreInput::ParallelModeSupervisorSnapshotInvalidated => CoreDispatchOutcome {
                events: vec![AppEvent::ParallelModeSupervisorSnapshotInvalidated],
                effects: Vec::new(),
                snapshot: self.snapshot(),
            },
            CoreInput::PlanningRuntimeProjectionChanged(snapshot) => {
                let changed = self.state.apply_planning_runtime_projection(snapshot);
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

    fn startup_changed_outcome(&self, effects: Vec<CoreEffect>) -> CoreDispatchOutcome {
        let snapshot = self.snapshot();
        CoreDispatchOutcome {
            events: vec![AppEvent::StartupChanged(snapshot.startup.clone())],
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

    fn conversation_changed_outcome(&self, effects: Vec<CoreEffect>) -> CoreDispatchOutcome {
        let snapshot = self.snapshot();
        CoreDispatchOutcome {
            events: vec![AppEvent::ConversationChanged(snapshot.conversation.clone())],
            effects,
            snapshot,
        }
    }
}

impl Default for CoreController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::service::planning::PlanningRuntimeSnapshot;
    use crate::core::app::{
        ConversationReadySnapshot, ConversationSnapshot, SessionCatalogReadySnapshot,
        SessionCatalogSnapshot,
    };
    use crate::core::app::{
        StartupAttachmentSnapshot, StartupDiagnosticSnapshot, StartupReadySnapshot,
        StartupSnapshot, TurnStreamUpdate,
    };
    use crate::domain::conversation::{
        ConversationMessage, ConversationMessageKind,
        ConversationSnapshot as DomainConversationSnapshot,
    };
    use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeReadinessState};
    use crate::domain::recent_sessions::RecentSessions;

    #[test]
    fn new_controller_exposes_initial_snapshot() {
        let controller = CoreController::new();

        assert_eq!(controller.snapshot(), AppSnapshot::initial());
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
            vec![AppEvent::StartupChanged(StartupSnapshot::Loading)]
        );
        assert_eq!(outcome.effects, vec![CoreEffect::RunStartupChecks]);
    }

    #[test]
    fn startup_completion_marks_startup_ready() {
        let mut controller = CoreController::new();
        let ready_snapshot = sample_startup_ready_snapshot();

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded(Ok(Box::new(ready_snapshot.clone()))),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            outcome.snapshot.startup,
            StartupSnapshot::Ready(Box::new(ready_snapshot.clone()))
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged(StartupSnapshot::Ready(Box::new(
                ready_snapshot
            )))]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn startup_completion_marks_startup_failed() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded(Err("codex missing".to_string())),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            outcome.snapshot.startup,
            StartupSnapshot::Failed {
                message: "codex missing".to_string()
            }
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged(StartupSnapshot::Failed {
                message: "codex missing".to_string()
            })]
        );
        assert!(outcome.effects.is_empty());
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
        }));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(outcome.snapshot.conversation, ConversationSnapshot::Loading);
        assert_eq!(
            outcome.events,
            vec![AppEvent::ConversationChanged(ConversationSnapshot::Loading)]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadConversation {
                thread_id: "thread-1".to_string()
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
            slot_lease_handoff: None,
        };

        let outcome =
            controller.handle_input(CoreInput::Command(AppCommand::SubmitTurn(request.clone())));

        assert!(outcome.events.is_empty());
        assert_eq!(outcome.effects, vec![CoreEffect::SubmitTurn(request)]);
        assert_eq!(outcome.snapshot, AppSnapshot::initial());
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

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded(Ok(Box::new(ready.clone()))),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            outcome.snapshot.conversation,
            ConversationSnapshot::Ready(Box::new(ready.clone()))
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::ConversationChanged(ConversationSnapshot::Ready(
                Box::new(ready)
            ))]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_completion_marks_failed() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded(Err("thread unavailable".to_string())),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            outcome.snapshot.conversation,
            ConversationSnapshot::Failed {
                message: "thread unavailable".to_string()
            }
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::ConversationChanged(
                ConversationSnapshot::Failed {
                    message: "thread unavailable".to_string()
                }
            )]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_stream_event_reduces_to_core_snapshot_without_state_revision() {
        let mut controller = CoreController::new();
        let stream_event =
            crate::application::service::conversation_runtime_event::ConversationStreamEvent::StatusUpdated {
            text: "thinking".to_string(),
        };

        let outcome = controller.handle_input(CoreInput::ConversationStreamUpdated(stream_event));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        let [AppEvent::TurnStreamSnapshotChanged(stream_snapshot)] = outcome.events.as_slice()
        else {
            panic!("stream event should emit a turn stream snapshot");
        };
        assert_eq!(stream_snapshot.revision, 1);
        assert_eq!(
            stream_snapshot.update,
            TurnStreamUpdate::StatusUpdated {
                text: "thinking".to_string()
            }
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_turn_completion_reduces_to_core_snapshot_without_state_revision() {
        let mut controller = CoreController::new();
        let execution_snapshot_capture =
            crate::application::service::planning::PlanningTurnExecutionSnapshotCapture::capture_failed(
                "/tmp/workspace",
                "planning capture failed".to_string(),
            );

        let outcome = controller.handle_input(CoreInput::ConversationTurnCompleted {
            turn_id: "turn-1".to_string(),
            changed_planning_file_paths: vec!["new/docs/plan.md".to_string()],
            execution_snapshot_capture: execution_snapshot_capture.clone(),
        });

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        let [AppEvent::TurnStreamSnapshotChanged(stream_snapshot)] = outcome.events.as_slice()
        else {
            panic!("turn completion should emit a turn stream snapshot");
        };
        assert_eq!(
            stream_snapshot.update,
            TurnStreamUpdate::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec!["new/docs/plan.md".to_string()],
                execution_snapshot_capture: Some(execution_snapshot_capture),
                status_text: "turn completed".to_string(),
            }
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_runtime_notice_reduces_to_core_snapshot_without_state_revision() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::ConversationRuntimeNotice(
            "reattached runtime".to_string(),
        ));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        let [AppEvent::TurnStreamSnapshotChanged(stream_snapshot)] = outcome.events.as_slice()
        else {
            panic!("runtime notice should emit a turn stream snapshot");
        };
        assert_eq!(
            stream_snapshot.update,
            TurnStreamUpdate::RuntimeNotice {
                notice: "reattached runtime".to_string()
            }
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_load_resets_previous_turn_stream_identity() {
        let mut controller = CoreController::new();
        controller.handle_input(CoreInput::ConversationStreamUpdated(
            crate::application::service::conversation_runtime_event::ConversationStreamEvent::ThreadPrepared {
                thread_id: "old-thread".to_string(),
                title: "Old Thread".to_string(),
                cwd: "/tmp/old".to_string(),
            },
        ));
        controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ConversationLoaded(Ok(Box::new(
                sample_conversation_ready_snapshot(),
            ))),
        ));

        let outcome = controller.handle_input(CoreInput::ConversationRuntimeNotice(
            "runtime reattached".to_string(),
        ));

        let [AppEvent::TurnStreamSnapshotChanged(stream_snapshot)] = outcome.events.as_slice()
        else {
            panic!("runtime notice should emit a fresh turn stream snapshot");
        };
        assert_eq!(stream_snapshot.revision, 1);
        assert_eq!(stream_snapshot.thread_id, None);
        assert_eq!(stream_snapshot.title, None);
        assert_eq!(stream_snapshot.cwd, None);
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn post_turn_evaluation_completion_passes_through_core_without_state_revision() {
        let mut controller = CoreController::new();
        let completion = crate::core::app::PostTurnEvaluationCompletion {
            thread_id: "thread-1".to_string(),
            completed_turn_id: "turn-1".to_string(),
        };

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted(completion.clone()),
        ));

        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            outcome.events,
            vec![AppEvent::PostTurnEvaluationCompleted(completion)]
        );
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn planning_parallel_projection_changes_appear_in_app_snapshot() {
        let mut controller = CoreController::new();
        let planning_snapshot =
            PlanningRuntimeSnapshot::invalid("planning validation failed in projection");

        let outcome = controller.handle_input(CoreInput::PlanningRuntimeProjectionChanged(
            Box::new(planning_snapshot.clone()),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            planning_snapshot
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
        let planning_snapshot =
            PlanningRuntimeSnapshot::invalid("planning validation failed in projection");
        controller.handle_input(CoreInput::PlanningRuntimeProjectionChanged(Box::new(
            planning_snapshot.clone(),
        )));

        let outcome = controller.handle_input(CoreInput::PlanningRuntimeProjectionChanged(
            Box::new(planning_snapshot),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert!(outcome.events.is_empty());
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn conversation_workspace_change_passes_through_core_without_state_revision() {
        let mut controller = CoreController::new();

        let outcome = controller.handle_input(CoreInput::ConversationTurnWorkspaceChanged {
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
        DomainConversationSnapshot {
            thread_id: "thread-1".to_string(),
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
        }
        .into()
    }
}
