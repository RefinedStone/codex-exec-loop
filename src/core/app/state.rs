use super::{AppSnapshot, StartupReadySnapshot, StartupState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    revision: u64,
    startup: StartupState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            revision: 0,
            startup: StartupState::Idle,
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            revision: self.revision,
            startup: self.startup.snapshot(),
        }
    }

    pub fn mark_startup_loading(&mut self) {
        self.startup = StartupState::Loading;
        self.advance_revision();
    }

    pub fn apply_startup_result(&mut self, result: Result<StartupReadySnapshot, String>) {
        self.startup = match result {
            Ok(ready) => StartupState::Ready(ready),
            Err(message) => StartupState::Failed(message),
        };
        self.advance_revision();
    }

    fn advance_revision(&mut self) {
        self.revision += 1;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::StartupSnapshot;

    #[test]
    fn new_state_projects_initial_snapshot() {
        assert_eq!(AppState::new().snapshot(), AppSnapshot::initial());
    }

    #[test]
    fn startup_loading_advances_revision() {
        let mut state = AppState::new();

        state.mark_startup_loading();

        assert_eq!(
            state.snapshot(),
            AppSnapshot {
                revision: 1,
                startup: StartupSnapshot::Loading,
            }
        );
    }
}
