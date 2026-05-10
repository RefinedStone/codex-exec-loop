use super::{AppCommand, AppEvent, AppSnapshot, AppState, CoreEffectCompletion, CoreInput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDispatchOutcome {
    pub events: Vec<AppEvent>,
    pub snapshot: AppSnapshot,
}

#[derive(Debug, Clone)]
pub struct CoreController {
    state: AppState,
}

impl CoreController {
    pub fn new() -> Self {
        Self {
            state: AppState::new(),
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        self.state.snapshot()
    }

    pub fn handle_input(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        match input {
            CoreInput::Command(AppCommand::Noop) => CoreDispatchOutcome {
                events: Vec::new(),
                snapshot: self.snapshot(),
            },
            CoreInput::Command(AppCommand::RunStartupChecks) => {
                self.state.mark_startup_loading();
                self.startup_changed_outcome()
            }
            CoreInput::EffectCompleted(CoreEffectCompletion::StartupChecksLoaded(result)) => {
                self.state.apply_startup_result(result);
                self.startup_changed_outcome()
            }
        }
    }

    fn startup_changed_outcome(&self) -> CoreDispatchOutcome {
        let snapshot = self.snapshot();
        CoreDispatchOutcome {
            events: vec![AppEvent::StartupChanged(snapshot.startup.clone())],
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
    use crate::core::app::{StartupReadySnapshot, StartupSnapshot};

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
    }

    #[test]
    fn startup_completion_marks_startup_ready() {
        let mut controller = CoreController::new();
        let ready_snapshot = StartupReadySnapshot {
            workspace_path: "/tmp/workspace".to_string(),
            can_continue: true,
            warnings: vec!["non fatal".to_string()],
        };

        let outcome = controller.handle_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded(Ok(ready_snapshot.clone())),
        ));

        assert_eq!(outcome.snapshot.revision, 1);
        assert_eq!(
            outcome.snapshot.startup,
            StartupSnapshot::Ready(ready_snapshot.clone())
        );
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged(StartupSnapshot::Ready(
                ready_snapshot
            ))]
        );
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
    }
}
