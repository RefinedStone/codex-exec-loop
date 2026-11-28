//! Adopt a verified creation into the existing publication/version owner.
//! No remote mutation occurs here, and a late adoption cannot overtake an update.
use super::*;
use crate::application::port::outbound::evidence_vault_port::{
    VaultCreationLoadRequest, VaultCreationRecordScope, VaultStoreError,
};
use crate::domain::issue_creation::{IssueCreationIntentRecord, IssueCreationIntentState};

impl IssueJournaledPublishService {
    pub fn adopt_created_issue(
        &self,
        creation: &IssueCreationIntentRecord,
        key: Option<VaultRecordKey>,
    ) -> Result<IssuePublishIntentRecord> {
        let _publishing = self.lock();
        let approved = creation
            .approved
            .as_ref()
            .ok_or_else(|| anyhow!("creation approval is missing"))?;
        let created = creation
            .created
            .as_ref()
            .ok_or_else(|| anyhow!("created issue identity is missing"))?;
        if creation.state != IssueCreationIntentState::Confirmed
            || !creation.repository.contains(&created.issue)
            || creation.observed_hash.as_ref() != Some(&approved.approval.shared_copy_hash)
            || creation.write_response_mismatch
        {
            bail!("only a verified creation can own a restore link")
        }
        let workspace = &creation.repository.project_scope.workspace_dir;
        let lease = self.journal.acquire_publication_lease(workspace)?;
        lease.verify_identity()?;
        let intent_id = creation.adoption_intent_id();
        let records = self.journal.load(workspace)?;
        if let Some(existing) = records.iter().find(|r| r.intent_id == intent_id) {
            if existing.issue != created.issue
                || existing.approved_hash != approved.approval.shared_copy_hash
                || existing.pending_vault_fields != creation.pending_vault_fields
                || existing.state != IssuePublishIntentState::Confirmed
                || existing.write_attempted != Some(true)
            {
                bail!("the creation adoption conflicts with an existing publish intent")
            }
            return Ok(existing.clone());
        }
        // Assigning a fresh sequence after a later update would make an old original
        // appear newest, even when both originals mask to the same public text.
        if records
            .iter()
            .any(|r| r.issue.same_remote_issue(&created.issue))
        {
            bail!("another publish intent already owns this issue")
        }
        let key =
            key.ok_or_else(|| anyhow!("the vault must be unlocked before adopting a creation"))?;
        for field in &creation.pending_vault_fields {
            let plaintext = self.vault.load_creation(VaultCreationLoadRequest {
                scope: VaultCreationRecordScope {
                    repository: creation.repository.clone(),
                    intent_id: creation.intent_id.clone(),
                    approved_hash: approved.approval.shared_copy_hash.clone(),
                    field: vault_field(*field),
                },
                key: key.clone(),
            })?;
            // Re-encrypt under the real stable issue ID; renaming ciphertext would
            // retain the wrong authenticated scope.
            lease.verify_identity()?;
            self.vault.save(VaultRecordSaveRequest {
                scope: VaultRecordScope {
                    issue: created.issue.clone(),
                    field: vault_field(*field),
                    binding: VaultRecordBinding::PublishIntent(intent_id.clone()),
                },
                key: key.clone(),
                plaintext,
            })?;
        }
        // Staging is not visible to restore. Only after it is complete do we take
        // the binding lease and atomically reserve the absence observed below until
        // the publication journal records ownership. Never take these leases in reverse.
        let binding = self.vault.begin_binding(VaultRecordLoadRequest {
            scope: VaultRecordScope::current_version(created.issue.clone()),
            key: key.clone(),
        })?;
        match &binding.snapshot().state {
            VaultBindingState::Missing => {}
            VaultBindingState::Deleted => return Err(VaultStoreError::IssueDeleted.into()),
            VaultBindingState::Readable { .. } => {
                bail!("an existing restore version already owns this issue")
            }
            VaultBindingState::Unreadable { error } => return Err(error.clone().into()),
        }
        let latest = self.snapshot.read(IssueSnapshotReadRequest {
            issue: created.issue.clone(),
        })?;
        if latest.issue != created.issue
            || latest.comparison_basis.field_hash != approved.approval.shared_copy_hash
            || latest.raw_fields.field_hash() != approved.approval.shared_copy_hash
        {
            bail!("the created issue changed before its restore link was bound")
        }
        let sequence = records
            .iter()
            .map(|r| r.sequence)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow!("publish sequence exhausted"))?;
        let record = IssuePublishIntentRecord {
            intent_id,
            issue: created.issue.clone(),
            previous_hash: approved.approval.shared_copy_hash.clone(),
            approved_hash: approved.approval.shared_copy_hash.clone(),
            pending_vault_fields: creation.pending_vault_fields.clone(),
            state: IssuePublishIntentState::Confirmed,
            observed_hash: creation.observed_hash.clone(),
            restore_link_finalized: false,
            restore_binding_attempt: Some(IssuePublishRestoreBindingAttempt {
                expected_ciphertext_sha256: binding.snapshot().token.ciphertext_sha256(),
            }),
            write_response_mismatch: false,
            write_attempted: Some(true),
            sequence,
        };
        lease.verify_identity()?;
        self.journal.save(workspace, &record)?;
        Ok(record)
    }
}
