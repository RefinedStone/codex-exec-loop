//! Local maintenance holds publication before the issue's binding transaction.
use anyhow::{Result, bail};
use std::sync::Arc;

use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultCleanupPolicy, VaultCleanupReport, VaultLifecycleAction,
    VaultRecordKey, VaultRecordLoadRequest,
};
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::domain::issue_publish_intent::IssuePublishIntentState;

pub(crate) struct LocalVaultMaintenance {
    pub(crate) workspace: String,
    pub(crate) vault: Arc<dyn EvidenceVaultStorePort>,
    pub(crate) journal: Arc<dyn IssuePublishIntentJournalPort>,
    pub(crate) key: VaultRecordKey,
}

pub(crate) fn maintain(
    stores: (
        &dyn EvidenceVaultStorePort,
        &dyn IssuePublishIntentJournalPort,
    ),
    request: VaultRecordLoadRequest,
    approval: impl FnOnce() -> Result<Option<VaultLifecycleAction>>,
) -> Result<Option<VaultCleanupReport>> {
    let (vault, journal) = stores;
    let expected = vault
        .begin_binding(request.clone())?
        .maintenance_revision()?;
    let Some(action) = approval()? else {
        return Ok(None);
    };
    let issue = &request.scope.issue;
    let workspace = &issue.project_scope.workspace_dir;
    let lease = journal.acquire_publication_lease(workspace)?;
    let mut disposable_intents = Vec::new();
    for record in journal
        .load(workspace)?
        .into_iter()
        .filter(|record| &record.issue == issue)
    {
        if record.state.is_unresolved()
            || record.state == IssuePublishIntentState::Held
            || (record.state == IssuePublishIntentState::Confirmed
                && !record.restore_link_finalized
                && !record.pending_vault_fields.is_empty())
            || (record.state == IssuePublishIntentState::Abandoned
                && record.write_attempted != Some(false))
        {
            bail!("this issue has an unresolved publication; the Vault was not changed");
        }
        if record.restore_link_finalized
            || (matches!(
                record.state,
                IssuePublishIntentState::Abandoned | IssuePublishIntentState::Conflict
            ) && record.write_attempted == Some(false))
        {
            disposable_intents.push(record.intent_id);
        }
    }
    lease.verify_identity()?;
    let binding = vault.begin_binding(request)?;
    lease.verify_identity()?;
    let policy = VaultCleanupPolicy {
        action,
        disposable_intents,
    };
    Ok(Some(binding.maintain(&expected, &policy)?))
}
