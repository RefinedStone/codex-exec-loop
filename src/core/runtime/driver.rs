use std::sync::mpsc::Receiver;

use crate::core::app::{
    AppCommand, AppSnapshot, CoreController, CoreDispatchOutcome, CoreEffect, CoreInput,
};

/*
 * CoreRuntime is the headless command loop around CoreController. Inbound
 * adapters can submit commands, while background workers send CoreInput back
 * through the queue so all app state changes still pass through the controller.
 */
pub struct CoreRuntime<E> {
    controller: CoreController,
    effect_executor: E,
    input_receiver: Receiver<CoreInput>,
}

pub trait CoreEffectExecutor {
    fn run_effect(&self, effect: CoreEffect);
}

impl<E> CoreRuntime<E>
where
    E: CoreEffectExecutor,
{
    pub fn new(effect_executor: E, input_receiver: Receiver<CoreInput>) -> Self {
        Self::from_parts(CoreController::new(), effect_executor, input_receiver)
    }

    pub fn from_parts(
        controller: CoreController,
        effect_executor: E,
        input_receiver: Receiver<CoreInput>,
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

    pub fn drain_pending_inputs(&mut self, max_inputs: usize) -> Vec<CoreDispatchOutcome> {
        let mut outcomes = Vec::new();

        for _ in 0..max_inputs {
            let Ok(input) = self.input_receiver.try_recv() else {
                break;
            };
            outcomes.push(self.dispatch_input(input));
        }

        outcomes
    }

    fn dispatch_input(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        let outcome = self.controller.handle_input(input);
        self.run_effects(&outcome);
        outcome
    }

    fn run_effects(&self, outcome: &CoreDispatchOutcome) {
        for effect in outcome.effects.iter().cloned() {
            self.effect_executor.run_effect(effect);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc;

    use super::*;
    use crate::core::app::{
        AppEvent, CoreEffectCompletion, SessionCatalogReadySnapshot, SessionCatalogSnapshot,
        StartupAttachmentSnapshot, StartupDiagnosticSnapshot, StartupReadySnapshot,
        StartupSnapshot,
    };
    use crate::domain::recent_sessions::RecentSessions;

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
        fn run_effect(&self, effect: CoreEffect) {
            self.effects.borrow_mut().push(effect);
        }
    }

    #[test]
    fn dispatch_command_updates_state_and_runs_returned_effects() {
        let (_tx, rx) = mpsc::channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);

        let outcome = runtime.dispatch_command(AppCommand::RunStartupChecks);

        assert_eq!(outcome.snapshot.startup, StartupSnapshot::Loading);
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged(StartupSnapshot::Loading)]
        );
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::RunStartupChecks]
        );
        assert_eq!(runtime.snapshot().startup, StartupSnapshot::Loading);
    }

    #[test]
    fn drain_pending_inputs_reenters_completions_through_controller() {
        let (tx, rx) = mpsc::channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);
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
            CoreEffectCompletion::StartupChecksLoaded(Ok(Box::new(ready.clone()))),
        ))
        .unwrap();

        let outcomes = runtime.drain_pending_inputs(8);

        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].events,
            vec![AppEvent::StartupChanged(StartupSnapshot::Ready(Box::new(
                ready.clone()
            )))]
        );
        assert_eq!(
            runtime.snapshot().startup,
            StartupSnapshot::Ready(Box::new(ready))
        );
        assert!(effects.recorded_effects().is_empty());
    }

    #[test]
    fn drain_pending_inputs_respects_batch_limit() {
        let (tx, rx) = mpsc::channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects, rx);

        tx.send(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded(Ok(SessionCatalogReadySnapshot {
                catalog: Box::new(
                    RecentSessions {
                        items: Vec::new(),
                        warnings: Vec::new(),
                        next_cursor: None,
                    }
                    .into(),
                ),
                tier_label: "provider-backed catalog".to_string(),
                item_count: 0,
                warnings: Vec::new(),
            })),
        ))
        .unwrap();
        tx.send(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded(Err("second result".to_string())),
        ))
        .unwrap();

        let first_batch = runtime.drain_pending_inputs(1);

        assert_eq!(first_batch.len(), 1);
        assert_eq!(
            runtime.snapshot().session_catalog,
            SessionCatalogSnapshot::Ready(SessionCatalogReadySnapshot {
                catalog: Box::new(
                    RecentSessions {
                        items: Vec::new(),
                        warnings: Vec::new(),
                        next_cursor: None,
                    }
                    .into(),
                ),
                tier_label: "provider-backed catalog".to_string(),
                item_count: 0,
                warnings: Vec::new(),
            })
        );

        let second_batch = runtime.drain_pending_inputs(1);

        assert_eq!(second_batch.len(), 1);
        assert_eq!(
            runtime.snapshot().session_catalog,
            SessionCatalogSnapshot::Failed {
                message: "second result".to_string()
            }
        );
    }
}
