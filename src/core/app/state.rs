use super::AppSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    revision: u64,
}

impl AppState {
    pub fn new() -> Self {
        Self { revision: 0 }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            revision: self.revision,
        }
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

    #[test]
    fn new_state_projects_initial_snapshot() {
        assert_eq!(AppState::new().snapshot(), AppSnapshot::initial());
    }
}
