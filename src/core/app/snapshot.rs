use super::StartupSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSnapshot {
    pub revision: u64,
    pub startup: StartupSnapshot,
}

impl AppSnapshot {
    pub fn initial() -> Self {
        Self {
            revision: 0,
            startup: StartupSnapshot::Idle,
        }
    }
}
