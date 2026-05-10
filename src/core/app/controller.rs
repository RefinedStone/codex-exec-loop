use super::{AppCommand, AppEvent, AppSnapshot, AppState, CoreInput};

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
}
