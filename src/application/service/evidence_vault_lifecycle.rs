//! Local maintenance holds publication before the issue's binding transaction.
use anyhow::{Result, bail};
use std::sync::Arc;

use crate::application::port::inbound::vault_maintenance_port::VaultMaintenancePort;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultCleanupPolicy, VaultCleanupReport, VaultLifecycleAction,
    VaultRecordKey, VaultRecordLoadRequest, VaultRecordScope,
};
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::domain::issue_publish_intent::IssuePublishIntentState;
use crate::domain::issue_work::IssueReference;

pub(crate) struct LocalVaultMaintenance {
    pub(crate) workspace: String,
    pub(crate) vault: Arc<dyn EvidenceVaultStorePort>,
    pub(crate) journal: Arc<dyn IssuePublishIntentJournalPort>,
    pub(crate) key: VaultRecordKey,
}

impl VaultMaintenancePort for LocalVaultMaintenance {
    fn workspace(&self) -> &str {
        &self.workspace
    }

    fn maintain(
        &self,
        issue: IssueReference,
        action: VaultLifecycleAction,
        confirm: &mut dyn FnMut() -> Result<bool>,
    ) -> Result<Option<VaultCleanupReport>> {
        if issue.project_scope.workspace_dir != self.workspace {
            bail!("the Vault issue does not belong to the selected workspace");
        }
        let request = VaultRecordLoadRequest {
            scope: VaultRecordScope::current_version(issue.clone()),
            key: self.key.clone(),
        };
        let expected = self
            .vault
            .begin_binding(request.clone())?
            .maintenance_revision()?;
        if action == VaultLifecycleAction::DeleteIssue && !confirm()? {
            return Ok(None);
        }
        let workspace = &issue.project_scope.workspace_dir;
        let lease = self.journal.acquire_publication_lease(workspace)?;
        let mut disposable_intents = Vec::new();
        for record in self
            .journal
            .load(workspace)?
            .into_iter()
            .filter(|record| record.issue == issue)
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
        let binding = self.vault.begin_binding(request)?;
        lease.verify_identity()?;
        let policy = VaultCleanupPolicy {
            action,
            disposable_intents,
        };
        Ok(Some(binding.maintain(&expected, &policy)?))
    }
}
