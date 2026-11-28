/// Both operations leave unindexed legacy data alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultLifecycleAction {
    RetainRecent,
    DeleteIssue,
}

#[derive(Debug)]
pub struct VaultCleanupPolicy {
    pub action: VaultLifecycleAction,
    /// The publication journal proved these staging records have no remaining reader.
    pub disposable_intents: Vec<String>,
}

/// Counts apply only to the prospective inventory. Old opaque files are not
/// scanned, attributed, counted, or promised erased.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VaultCleanupReport {
    pub tombstoned: bool,
    pub removed: usize,
    pub retained: usize,
    pub cleanup_pending: usize,
    pub legacy_unverified: bool,
}
