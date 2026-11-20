//! A new issue has a repository and a durable local intent before it has an issue ID.
//! Sent is committed before the sole remote mutation. Recovery only reads or binds.
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use crate::application::port::inbound::issue_creation_draft::{
    IssueCreationRestartRequest, IssueCreationTarget,
};
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultCreationRecordScope, VaultCreationSaveRequest, VaultRecordField,
    VaultRecordKey,
};
use crate::application::port::outbound::issue_create_port::{
    IssueCreatePort, IssueCreateRequest, IssueRepositoryResolveRequest,
};
use crate::application::port::outbound::issue_creation_journal_port::{
    IssueCreationJournalPort, IssueCreationJournalTransaction,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest, IssueSnapshotResponse,
};
use crate::application::service::issue_publish_journal::{
    IssueJournaledPublishService, IssueRestorableField,
};
use crate::domain::issue_creation::{
    IssueCreatedLocation, IssueCreationApproval, IssueCreationApprovedCopy,
    IssueCreationIntentRecord, IssueCreationIntentState, IssueRepositoryReference,
};
use crate::domain::issue_publish_intent::IssuePublishVaultField;
use crate::domain::issue_validation::IssueValidationPassed;
use crate::domain::issue_work::{IssueFieldHash, IssueLocator, IssueProjectScope, IssueProvider};

pub struct IssueCreationApprovedRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
    pub validated: IssueValidationPassed,
    pub approval: IssueCreationApproval,
    pub policy_version: String,
    pub template_version: Option<String>,
    pub restorable: Vec<IssueRestorableField>,
    pub vault_key: Option<VaultRecordKey>,
    pub shared_copy_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCreationOutcome {
    Intent(Box<IssueCreationIntentRecord>),
    Refused(&'static str),
}

pub struct IssueCreationService {
    create: Arc<dyn IssueCreatePort>,
    snapshot: Arc<dyn IssueSnapshotPort>,
    journal: Arc<dyn IssueCreationJournalPort>,
    vault: Arc<dyn EvidenceVaultStorePort>,
    publisher: Arc<IssueJournaledPublishService>,
}

impl IssueCreationService {
    pub fn new(
        create: Arc<dyn IssueCreatePort>,
        snapshot: Arc<dyn IssueSnapshotPort>,
        journal: Arc<dyn IssueCreationJournalPort>,
        vault: Arc<dyn EvidenceVaultStorePort>,
        publisher: Arc<IssueJournaledPublishService>,
    ) -> Self {
        Self {
            create,
            snapshot,
            journal,
            vault,
            publisher,
        }
    }

    /// Opening another window or restarting the process reuses even a completed intent.
    /// Only an explicit restart can create another local attempt.
    pub fn open(&self, target: &IssueCreationTarget) -> Result<IssueCreationIntentRecord> {
        let repository = self.repository(target)?;
        let mut transaction = self
            .journal
            .begin(&repository.project_scope.workspace_dir)?;
        if let Some(record) = transaction
            .records()?
            .into_iter()
            .rfind(|r| r.repository == repository)
        {
            return Ok(record);
        }
        let record = prepared(repository, None);
        transaction.save(&record)?;
        Ok(record)
    }

    /// Preview needs no remote read. Publication resolves the repository again.
    pub fn intent(
        &self,
        target: &IssueCreationTarget,
        intent_id: &str,
    ) -> Result<IssueCreationIntentRecord> {
        let workspace = self.journal.canonical_workspace(&target.cwd)?;
        let mut transaction = self.journal.begin(&workspace)?;
        find_intent(&mut *transaction, &workspace, intent_id)
    }

    pub fn restart(
        &self,
        request: &IssueCreationRestartRequest,
    ) -> Result<IssueCreationIntentRecord> {
        let repository = self.repository(&request.target)?;
        let mut transaction = self
            .journal
            .begin(&repository.project_scope.workspace_dir)?;
        let mut previous = transaction
            .records()?
            .into_iter()
            .rfind(|r| r.repository == repository)
            .ok_or_else(|| anyhow!("there is no creation intent to restart"))?;
        if previous.intent_id != request.previous_intent_id {
            bail!("the creation intent changed; reopen the draft")
        }
        if matches!(
            previous.state,
            IssueCreationIntentState::Sent | IssueCreationIntentState::NeedsConfirmation
        ) && !request.acknowledge_uncertain
        {
            bail!("the previous attempt may already exist; acknowledge it before starting another")
        }
        if matches!(
            previous.state,
            IssueCreationIntentState::Prepared
                | IssueCreationIntentState::Held
                | IssueCreationIntentState::Recorded
        ) {
            previous.state = IssueCreationIntentState::Abandoned;
            transaction.save(&previous)?;
        }
        let record = prepared(repository, Some(previous.intent_id));
        transaction.save(&record)?;
        Ok(record)
    }

    pub fn publish(&self, request: IssueCreationApprovedRequest) -> Result<IssueCreationOutcome> {
        let copy = request.validated.shared_copy();
        if request.approval.intent_id != request.intent_id
            || request.approval.shared_copy_hash
                != IssueFieldHash::of_fields(&copy.title, &copy.body)
            || request.approval.policy_version != request.policy_version
            || request.approval.template_version != request.template_version
        {
            return Ok(IssueCreationOutcome::Refused("approval_mismatch"));
        }
        let workspace = self.journal.canonical_workspace(&request.target.cwd)?;
        let mut transaction = self.journal.begin(&workspace)?;
        let mut record = find_intent(&mut *transaction, &workspace, &request.intent_id)?;
        if record.repository != request.approval.repository {
            return Ok(IssueCreationOutcome::Refused("approval_mismatch"));
        }
        if record.state == IssueCreationIntentState::Abandoned {
            return Ok(IssueCreationOutcome::Refused("intent_abandoned"));
        }
        if transaction
            .records()?
            .iter()
            .rfind(|r| r.repository == record.repository)
            .is_none_or(|r| r.intent_id != record.intent_id)
        {
            return Ok(IssueCreationOutcome::Refused("intent_not_current"));
        }
        if let Some(frozen) = &record.approved
            && (frozen.approval != request.approval || frozen.shared_copy != *copy)
        {
            return Ok(IssueCreationOutcome::Refused("frozen_approval_changed"));
        }
        if record.state.may_have_been_sent() {
            record = self.restore_link(&mut *transaction, record, request.vault_key)?;
            return Ok(IssueCreationOutcome::Intent(Box::new(record)));
        }
        // Names are locators, not authorization. A transferred/replaced repository cannot
        // receive a copy approved for the old stable repository ID.
        if self.repository(&request.target)? != record.repository {
            return Ok(IssueCreationOutcome::Refused("repository_changed"));
        }
        if record.state == IssueCreationIntentState::Prepared {
            record.approved = Some(IssueCreationApprovedCopy {
                approval: request.approval,
                shared_copy: copy.clone(),
            });
            record.shared_copy_only = request.shared_copy_only;
            if !request.shared_copy_only {
                for item in &request.restorable {
                    if record.pending_vault_fields.contains(&item.field) {
                        bail!("duplicate restorable creation field")
                    }
                    record.pending_vault_fields.push(item.field);
                }
            }
            // Freeze before staging. An interrupted/partially written mapping is never
            // restaged with a different original that masks to the same shared copy.
            record.state = IssueCreationIntentState::Held;
            transaction.save(&record)?;
            if !request.shared_copy_only
                && !request.restorable.is_empty()
                && !self.stage(&record, request.restorable, request.vault_key.clone())
            {
                return Ok(IssueCreationOutcome::Intent(Box::new(record)));
            }
        } else if record.state == IssueCreationIntentState::Held {
            if !request.shared_copy_only {
                return Ok(IssueCreationOutcome::Intent(Box::new(record)));
            }
            record.shared_copy_only = true;
            record.pending_vault_fields.clear();
        }
        if request.shared_copy_only {
            record.shared_copy_only = true;
            record.pending_vault_fields.clear();
        }
        record.state = IssueCreationIntentState::Recorded;
        transaction.save(&record)?;
        let approved = record
            .approved
            .as_ref()
            .ok_or_else(|| anyhow!("creation approval is missing"))?;
        let write = IssueCreateRequest {
            repository: record.repository.clone(),
            intent_id: record.intent_id.clone(),
            approval: approved.approval.clone(),
            shared_copy: approved.shared_copy.clone(),
        };
        write.ensure_authorized()?;
        record.state = IssueCreationIntentState::Sent;
        transaction.save(&record)?;
        let response = self.create.create_issue(write);
        record.state = IssueCreationIntentState::NeedsConfirmation;
        if let Ok(response) = response {
            let hash = IssueFieldHash::of_fields(
                &response.published_copy.title,
                &response.published_copy.body,
            );
            record.write_response_mismatch = hash != approved_hash(&record)?
                || !valid_location(&record.repository, &response.created);
            // Persist a trustworthy stable ID even when the returned copy contradicts
            // approval. Manual confirmation may recheck that exact issue, never another.
            if valid_location(&record.repository, &response.created) {
                record.created = Some(response.created);
            }
        }
        // Commit candidate/contradiction before a second network request.
        transaction.save(&record)?;
        record = self.observe(record);
        transaction.save(&record)?;
        record = self.restore_link(&mut *transaction, record, request.vault_key)?;
        Ok(IssueCreationOutcome::Intent(Box::new(record)))
    }

    pub fn recheck(
        &self,
        target: &IssueCreationTarget,
        intent_id: &str,
        key: Option<VaultRecordKey>,
    ) -> Result<IssueCreationIntentRecord> {
        let workspace = self.journal.canonical_workspace(&target.cwd)?;
        let mut transaction = self.journal.begin(&workspace)?;
        let record = find_intent(&mut *transaction, &workspace, intent_id)?;
        let next = self.observe(record.clone());
        if next != record {
            transaction.save(&next)?;
        }
        self.restore_link(&mut *transaction, next, key)
    }

    /// User-supplied URLs are resolved and reread by stable ID. Matching titles,
    /// search results, or an empty search are never evidence of creation.
    pub fn confirm(
        &self,
        target: &IssueCreationTarget,
        intent_id: &str,
        url: &str,
        key: Option<VaultRecordKey>,
    ) -> Result<IssueCreationIntentRecord> {
        let locator = github_issue_locator(url)?;
        let workspace = self.journal.canonical_workspace(&target.cwd)?;
        let mut transaction = self.journal.begin(&workspace)?;
        let mut record = find_intent(&mut *transaction, &workspace, intent_id)?;
        if record.state == IssueCreationIntentState::Confirmed {
            if record
                .created
                .as_ref()
                .is_none_or(|known| known.url != url || known.locator != locator)
                || self.repository(target)? != record.repository
            {
                bail!("the URL does not match the confirmed creation")
            }
            return self.restore_link(&mut *transaction, record, key);
        }
        if !matches!(
            record.state,
            IssueCreationIntentState::Sent | IssueCreationIntentState::NeedsConfirmation
        ) {
            bail!("only a sent creation attempt can be confirmed with a URL")
        }
        if self.repository(target)? != record.repository {
            bail!("the approved repository changed")
        }
        let resolved = self.snapshot.resolve(IssueSnapshotResolveRequest {
            locator: locator.clone(),
            project_scope: record.repository.project_scope.clone(),
        })?;
        if !record.repository.contains(&resolved.issue)
            || resolved.issue.issue_id.is_empty()
            || snapshot_hash(&resolved) != Some(approved_hash(&record)?)
            || record
                .created
                .as_ref()
                .is_some_and(|known| known.issue != resolved.issue)
        {
            bail!("the selected issue does not match the approved creation attempt")
        }
        let latest = self.snapshot.read(IssueSnapshotReadRequest {
            issue: resolved.issue.clone(),
        })?;
        if latest.issue != resolved.issue || snapshot_hash(&latest) != Some(approved_hash(&record)?)
        {
            bail!("the selected issue changed while confirming")
        }
        record.created = Some(IssueCreatedLocation {
            issue: resolved.issue,
            locator,
            url: url.to_string(),
        });
        record.observed_hash = Some(approved_hash(&record)?);
        record.write_response_mismatch = false;
        record.state = IssueCreationIntentState::Confirmed;
        transaction.save(&record)?;
        self.restore_link(&mut *transaction, record, key)
    }

    fn repository(&self, target: &IssueCreationTarget) -> Result<IssueRepositoryReference> {
        let workspace_dir = self.journal.canonical_workspace(&target.cwd)?;
        let repository = self
            .create
            .resolve_repository(IssueRepositoryResolveRequest {
                owner: target.owner.clone(),
                repository: target.repository.clone(),
                project_scope: IssueProjectScope {
                    workspace_dir: workspace_dir.clone(),
                },
            })?;
        if repository.project_scope.workspace_dir != workspace_dir
            || repository.provider != IssueProvider::GitHub
            || repository.server_id != "github.com"
            || repository.repository_id.is_empty()
        {
            bail!("invalid creation repository reference")
        }
        Ok(repository)
    }

    fn stage(
        &self,
        record: &IssueCreationIntentRecord,
        fields: Vec<IssueRestorableField>,
        key: Option<VaultRecordKey>,
    ) -> bool {
        let Some(key) = key else { return false };
        let Ok(approved_hash) = approved_hash(record) else {
            return false;
        };
        fields.into_iter().all(|field| {
            self.vault
                .save_creation(VaultCreationSaveRequest {
                    scope: VaultCreationRecordScope {
                        repository: record.repository.clone(),
                        intent_id: record.intent_id.clone(),
                        approved_hash: approved_hash.clone(),
                        field: match field.field {
                            IssuePublishVaultField::Title => VaultRecordField::Title,
                            IssuePublishVaultField::Body => VaultRecordField::Body,
                        },
                    },
                    key: key.clone(),
                    plaintext: field.plaintext,
                })
                .is_ok()
        })
    }

    fn observe(&self, mut record: IssueCreationIntentRecord) -> IssueCreationIntentRecord {
        if !matches!(
            record.state,
            IssueCreationIntentState::Sent | IssueCreationIntentState::NeedsConfirmation
        ) {
            return record;
        }
        record.state = IssueCreationIntentState::NeedsConfirmation;
        record.observed_hash = record.created.as_ref().and_then(|created| {
            let snapshot = self
                .snapshot
                .read(IssueSnapshotReadRequest {
                    issue: created.issue.clone(),
                })
                .ok()?;
            (snapshot.issue == created.issue)
                .then(|| snapshot_hash(&snapshot))
                .flatten()
        });
        if !record.write_response_mismatch
            && record.observed_hash.is_some()
            && record.observed_hash == approved_hash(&record).ok()
        {
            record.state = IssueCreationIntentState::Confirmed;
        }
        record
    }

    fn restore_link(
        &self,
        transaction: &mut dyn IssueCreationJournalTransaction,
        mut record: IssueCreationIntentRecord,
        key: Option<VaultRecordKey>,
    ) -> Result<IssueCreationIntentRecord> {
        if record.state != IssueCreationIntentState::Confirmed || record.restore_link_finalized {
            return Ok(record);
        }
        if record.pending_vault_fields.is_empty() {
            record.restore_link_finalized = true;
        } else if let Some(key) = key {
            let current_matches = record.created.as_ref().is_some_and(|created| {
                self.snapshot
                    .read(IssueSnapshotReadRequest {
                        issue: created.issue.clone(),
                    })
                    .is_ok_and(|snapshot| {
                        snapshot.issue == created.issue
                            && snapshot_hash(&snapshot) == approved_hash(&record).ok()
                    })
            });
            if !current_matches {
                return Ok(record);
            }
            // A confirmed remote creation stays confirmed if local restoration fails.
            // The ordinary publisher owns ordering and CurrentVersion; reuse that owner.
            if let Ok(adopted) = self
                .publisher
                .adopt_created_issue(&record, Some(key.clone()))
            {
                record.restore_link_finalized = self
                    .publisher
                    .finalize_restore_link(
                        &record.repository.project_scope.workspace_dir,
                        &adopted.intent_id,
                        key,
                    )
                    .is_ok_and(|result| result.restore_link_finalized);
            }
        }
        if record.restore_link_finalized {
            transaction.save(&record)?;
        }
        Ok(record)
    }
}

fn prepared(
    repository: IssueRepositoryReference,
    predecessor: Option<String>,
) -> IssueCreationIntentRecord {
    IssueCreationIntentRecord {
        intent_id: format!("{:032x}", rand::random::<u128>()),
        repository,
        state: IssueCreationIntentState::Prepared,
        approved: None,
        pending_vault_fields: Vec::new(),
        created: None,
        observed_hash: None,
        write_response_mismatch: false,
        restore_link_finalized: false,
        shared_copy_only: false,
        predecessor_intent_id: predecessor,
    }
}

fn find_intent(
    transaction: &mut dyn IssueCreationJournalTransaction,
    workspace: &str,
    intent_id: &str,
) -> Result<IssueCreationIntentRecord> {
    transaction
        .records()?
        .into_iter()
        .find(|r| r.intent_id == intent_id && r.repository.project_scope.workspace_dir == workspace)
        .ok_or_else(|| anyhow!("the journal has no creation intent in this workspace"))
}

fn approved_hash(record: &IssueCreationIntentRecord) -> Result<IssueFieldHash> {
    record
        .approved
        .as_ref()
        .map(|a| a.approval.shared_copy_hash.clone())
        .ok_or_else(|| anyhow!("creation approval is missing"))
}

fn snapshot_hash(snapshot: &IssueSnapshotResponse) -> Option<IssueFieldHash> {
    let hash = snapshot.raw_fields.field_hash();
    (hash == snapshot.comparison_basis.field_hash).then_some(hash)
}

fn valid_location(repository: &IssueRepositoryReference, created: &IssueCreatedLocation) -> bool {
    repository.contains(&created.issue)
        && !created.issue.issue_id.is_empty()
        && github_issue_locator(&created.url).is_ok_and(|locator| locator == created.locator)
}

fn github_issue_locator(url: &str) -> Result<IssueLocator> {
    let path = url
        .strip_prefix("https://github.com/")
        .ok_or_else(|| anyhow!("select a canonical https://github.com issue URL"))?;
    let parts: Vec<_> = path.split('/').collect();
    let valid_name = |value: &str| {
        !value.is_empty()
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    if parts.len() != 4 || parts[2] != "issues" || !valid_name(parts[0]) || !valid_name(parts[1]) {
        bail!("select a canonical GitHub issue URL")
    }
    let number: u64 = parts[3]
        .parse()
        .map_err(|_| anyhow!("invalid issue number"))?;
    if number == 0 || number.to_string() != parts[3] {
        bail!("invalid issue number")
    }
    Ok(IssueLocator {
        owner: parts[0].to_string(),
        repository: parts[1].to_string(),
        number,
    })
}

#[cfg(test)]
mod tests;
