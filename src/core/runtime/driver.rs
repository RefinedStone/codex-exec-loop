use crate::core::app::{
    AppCommand, AppSnapshot, CoreController, CoreDispatchOutcome, CoreEffect, CoreInput,
};

use super::input_mailbox::CoreInputReceiver;

/*
 * CoreRuntime is the headless command loop around CoreController. Inbound
 * adapters can submit commands, while background workers send CoreInput back
 * through the queue so all app state changes still pass through the controller.
 */
pub struct CoreRuntime<E> {
    controller: CoreController,
    effect_executor: E,
    input_receiver: CoreInputReceiver,
}

pub trait CoreEffectExecutor {
    // Bounded local effects may return an immediate completion when their mutation must remain
    // serialized with the command dispatch. Long-running provider work returns None and reports
    // completion through CoreInputSender instead.
    fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput>;
}

impl<E> CoreRuntime<E>
where
    E: CoreEffectExecutor,
{
    pub fn new(effect_executor: E, input_receiver: CoreInputReceiver) -> Self {
        Self::from_parts(CoreController::new(), effect_executor, input_receiver)
    }

    pub fn from_parts(
        controller: CoreController,
        effect_executor: E,
        input_receiver: CoreInputReceiver,
    ) -> Self {
        Self {
            controller,
            effect_executor,
            input_receiver,
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        self.controller.snapshot()
    }

    pub fn dispatch_command(&mut self, command: AppCommand) -> CoreDispatchOutcome {
        self.dispatch_input(CoreInput::Command(command))
    }

    #[cfg(test)]
    pub(crate) fn drain_pending_inputs(&mut self, max_inputs: usize) -> Vec<CoreDispatchOutcome> {
        let mut outcomes = Vec::new();

        for _ in 0..max_inputs {
            let Some(outcome) = self.poll_pending_input() else {
                break;
            };
            outcomes.push(outcome);
        }

        outcomes
    }

    pub fn poll_pending_input(&mut self) -> Option<CoreDispatchOutcome> {
        let input = self.input_receiver.try_recv().ok()?;
        Some(self.dispatch_input(input))
    }

    pub fn dispatch_input(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        let mut outcome = self.controller.handle_input(input);
        let effects = outcome.effects.clone();
        for effect in effects {
            let Some(immediate_input) = self.effect_executor.run_effect(effect) else {
                continue;
            };
            let immediate_outcome = self.dispatch_input(immediate_input);
            outcome.events.extend(immediate_outcome.events);
            outcome.effects.extend(immediate_outcome.effects);
            outcome.snapshot = immediate_outcome.snapshot;
        }
        outcome
    }

    #[cfg(test)]
    pub(crate) fn begin_test_turn_submission(
        &mut self,
    ) -> crate::core::app::TurnSubmissionCorrelation {
        self.controller.begin_test_turn_submission()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc::TrySendError;

    use super::*;
    use crate::application::service::manual_prompt_preparation::ManualPromptPreparationRequest;
    use crate::core::app::{
        AppEvent, CoreEffectCompletion, CorePromptOrigin, StartupAttachmentSnapshot,
        StartupCheckCorrelation, StartupDiagnosticSnapshot, StartupReadySnapshot, StartupSnapshot,
        TurnStreamEvent, TurnSubmissionAdmission, TurnSubmissionRequest,
    };
    use crate::core::runtime::input_mailbox::{CORE_INPUT_CHANNEL_CAPACITY, core_input_channel};

    #[test]
    fn core_input_channel_applies_backpressure_and_reports_disconnect() {
        let (sender, receiver) = core_input_channel();
        for _ in 0..CORE_INPUT_CHANNEL_CAPACITY {
            sender
                .try_send(CoreInput::Command(AppCommand::Noop))
                .expect("inputs within the fixed capacity should be admitted");
        }
        assert!(matches!(
            sender.try_send(CoreInput::Command(AppCommand::Noop)),
            Err(TrySendError::Full(_))
        ));
        drop(receiver);
        assert!(sender.send(CoreInput::Command(AppCommand::Noop)).is_err());
    }

    #[derive(Clone, Default)]
    struct RecordingEffectExecutor {
        effects: Rc<RefCell<Vec<CoreEffect>>>,
    }

    impl RecordingEffectExecutor {
        fn recorded_effects(&self) -> Vec<CoreEffect> {
            self.effects.borrow().clone()
        }
    }

    impl CoreEffectExecutor for RecordingEffectExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            self.effects.borrow_mut().push(effect);
            None
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateManualPromptExecutor;

    impl CoreEffectExecutor for ImmediateManualPromptExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::PrepareManualPrompt(request) = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::ManualPromptPrepared(Box::new(
                    crate::domain::planning::ManualPromptOutcome::Rejected {
                        correlation: request.correlation,
                        transcript_text: request.raw_prompt,
                        runtime_projection: Box::new(
                            crate::domain::planning::RuntimeProjection::invalid("blocked"),
                        ),
                        reason: "blocked".to_string(),
                    },
                )),
            ))
        }
    }

    #[test]
    fn dispatch_command_updates_state_and_runs_returned_effects() {
        let (_tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);

        let outcome = runtime.dispatch_command(AppCommand::RunStartupChecks);

        assert_eq!(outcome.snapshot.startup, StartupSnapshot::Loading);
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged {
                correlation: StartupCheckCorrelation::new(1),
                snapshot: StartupSnapshot::Loading,
            }]
        );
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::RunStartupChecks {
                correlation: StartupCheckCorrelation::new(1),
            }]
        );
        assert_eq!(runtime.snapshot().startup, StartupSnapshot::Loading);
    }

    #[test]
    fn submit_turn_admission_runs_exactly_one_worker_until_the_active_turn_closes() {
        let (_tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);
        let request = TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            prompt: "ship it".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        };

        let first = runtime.dispatch_command(AppCommand::SubmitTurn(request.clone()));
        let rejected = runtime.dispatch_command(AppCommand::SubmitTurn(request.clone()));

        assert_eq!(
            first.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::Accepted {
                    correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
                },
            )]
        );
        assert_eq!(
            rejected.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedActive {
                    active_correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
                },
            )]
        );
        assert!(rejected.effects.is_empty());
        assert_eq!(first.snapshot, AppSnapshot::initial());
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::SubmitTurn {
                correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
                request: request.clone(),
            }]
        );

        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
            event: TurnStreamEvent::Failed {
                message: "worker stopped before turn/start".to_string(),
            },
        });
        let retried = runtime.dispatch_command(AppCommand::SubmitTurn(request.clone()));

        assert_eq!(
            retried.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::Accepted {
                    correlation: crate::core::app::TurnSubmissionCorrelation::new(2),
                },
            )]
        );
        assert_eq!(
            effects.recorded_effects(),
            vec![
                CoreEffect::SubmitTurn {
                    correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
                    request: request.clone(),
                },
                CoreEffect::SubmitTurn {
                    correlation: crate::core::app::TurnSubmissionCorrelation::new(2),
                    request,
                },
            ]
        );
    }

    #[test]
    fn prepare_manual_prompt_command_runs_prepare_effect_without_snapshot_change() {
        let (_tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);
        let request = ManualPromptPreparationRequest {
            correlation: crate::domain::planning::ManualPromptCorrelation {
                request_id: 1,
                generation: 1,
                workspace_directory: "/tmp/workspace".to_string(),
            },
            raw_prompt: "ship it".to_string(),
            parent_thread_id: Some("thread-1".to_string()),
            parent_turn_id: None,
        };

        let outcome =
            runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(request.clone())));

        assert!(outcome.events.is_empty());
        assert_eq!(outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::PrepareManualPrompt(Box::new(request))]
        );
    }

    #[test]
    fn immediate_manual_prompt_effect_is_accepted_before_dispatch_returns() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateManualPromptExecutor, rx);
        let request = ManualPromptPreparationRequest {
            correlation: crate::domain::planning::ManualPromptCorrelation {
                request_id: 1,
                generation: 1,
                workspace_directory: "/tmp/workspace".to_string(),
            },
            raw_prompt: "ship it".to_string(),
            parent_thread_id: None,
            parent_turn_id: None,
        };

        let outcome =
            runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(request.clone())));

        assert_eq!(
            outcome.effects,
            vec![CoreEffect::PrepareManualPrompt(Box::new(request.clone()))]
        );
        assert!(matches!(
            outcome.events.as_slice(),
            [AppEvent::ManualPromptPrepared(result)]
                if result.correlation() == &request.correlation
        ));
        let second = runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(request)));
        assert_eq!(second.events.len(), 1);
    }

    #[test]
    fn drain_pending_inputs_reenters_completions_through_controller() {
        let (tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);
        runtime.dispatch_command(AppCommand::RunStartupChecks);
        let ready = StartupReadySnapshot {
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
            warnings: Vec::new(),
            schema_snapshot: "embedded schema".to_string(),
        };

        tx.send(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: StartupCheckCorrelation::new(1),
                result: Ok(Box::new(ready.clone())),
            },
        ))
        .unwrap();

        let outcomes = runtime.drain_pending_inputs(8);

        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].events,
            vec![AppEvent::StartupChanged {
                correlation: StartupCheckCorrelation::new(1),
                snapshot: StartupSnapshot::Ready(Box::new(ready.clone())),
            }]
        );
        assert_eq!(
            runtime.snapshot().startup,
            StartupSnapshot::Ready(Box::new(ready))
        );
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::RunStartupChecks {
                correlation: StartupCheckCorrelation::new(1),
            }]
        );
    }

    #[test]
    fn drain_pending_inputs_respects_batch_limit() {
        let (tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects, rx);

        tx.send(CoreInput::Command(AppCommand::Noop)).unwrap();
        tx.send(CoreInput::Command(AppCommand::Noop)).unwrap();

        let first_batch = runtime.drain_pending_inputs(1);

        assert_eq!(first_batch.len(), 1);

        let second_batch = runtime.drain_pending_inputs(1);

        assert_eq!(second_batch.len(), 1);
        assert!(runtime.drain_pending_inputs(1).is_empty());
    }
}
