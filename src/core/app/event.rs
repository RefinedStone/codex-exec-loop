use super::{AppSnapshot, StartupReadySnapshot, StartupSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreInput {
    Command(super::AppCommand),
    EffectCompleted(CoreEffectCompletion),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffectCompletion {
    StartupChecksLoaded(Result<StartupReadySnapshot, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /*
     * SnapshotChanged is the neutral output event for the skeleton. Concrete
     * slices can add narrower events such as StartupChanged while preserving the
     * same core-to-inbound adapter direction.
     */
    SnapshotChanged(AppSnapshot),
    StartupChanged(StartupSnapshot),
}
