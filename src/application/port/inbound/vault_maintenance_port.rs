use anyhow::Result;

use crate::domain::issue_work::IssueReference;

pub(crate) trait VaultMaintenancePort {
    fn workspace(&self) -> &str;

    fn maintain(
        &self,
        issue: IssueReference,
        action: VaultLifecycleAction,
        confirm: &mut dyn FnMut() -> Result<bool>,
    ) -> Result<Option<VaultCleanupReport>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultLifecycleAction {
    RetainRecent,
    DeleteIssue,
}

/// Counts cover only the prospective inventory, never unindexed files.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VaultCleanupReport {
    pub tombstoned: bool,
    pub removed: usize,
    pub retained: usize,
    pub cleanup_pending: usize,
    pub legacy_unverified: bool,
}
