use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, Transaction, params};
use sha2::{Digest, Sha256};

use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget,
};
use crate::domain::issue_automation::collection::StoredIssueCollection;
use crate::domain::issue_automation::source::{
    IssueSourceContent, IssueSourceObservation, IssueSourceRead, IssueSourceRepository,
};
use crate::domain::issue_automation::{
    IssueAutomationBinding, IssueAutomationBindingId, IssueAutomationFlag, TrackedIssue,
};
use crate::domain::issue_work::{IssueComparisonBasis, IssueProvider, IssueReference};

use super::super::{
    enum_name, insert_issue_row, next_revision, read_binding, read_issue, read_revision,
    sql_revision, update_issue_row, validate_binding_scope, validate_reference_scope,
};
use super::queries;

const MAX_OBSERVATION_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct StoredObservation {
    pub binding_id: IssueAutomationBindingId,
    pub current: IssueSourceRead,
}

pub(super) fn reference(read: &IssueSourceRead) -> &IssueReference {
    match read {
        IssueSourceRead::Observed(observation) => &observation.issue,
        IssueSourceRead::Unknown { issue, .. } => issue,
    }
}

pub(super) fn validate_page(
    workspace: &str,
    request: &IssueDiscoveryRequest,
    page: &IssueDiscoveryPage,
) -> Result<()> {
    request.validate()?;
    page.repository.validate()?;
    if page.repository.project_scope.workspace_dir != workspace {
        bail!("the issue discovery page belongs to another project scope")
    }
    match &request.target {
        IssueDiscoveryTarget::Repository(repository) if repository != &page.repository => {
            bail!("the issue discovery response changed its stable repository identity")
        }
        IssueDiscoveryTarget::GitHubLocator {
            project_scope,
            expected_repository_id,
            ..
        } if page.repository.provider != IssueProvider::GitHub
            || page.repository.server_id != "github.com"
            || page.repository.project_scope != *project_scope
            || expected_repository_id
                .as_ref()
                .is_some_and(|expected| expected != &page.repository.repository_id) =>
        {
            bail!("the issue discovery response changed its expected repository identity")
        }
        _ => {}
    }
    if page.scanned_count > request.page_size
        || page.observations.len() > usize::from(page.scanned_count)
    {
        bail!("the issue discovery page exceeds its requested bound")
    }
    if let Some(cursor) = &page.next_cursor {
        cursor.validate_for(
            &page.repository,
            &request.selection,
            request.page_size,
            &request.disclosure,
        )?;
        if request.cursor.as_ref() == Some(cursor) {
            bail!("the issue discovery page did not advance its cursor")
        }
    }
    let mut seen = BTreeSet::new();
    for observation in &page.observations {
        let policy_version = match &observation.content {
            IssueSourceContent::Shared(snapshot) => &snapshot.policy_version,
            IssueSourceContent::Withheld { policy_version, .. } => policy_version,
        };
        if policy_version != &request.disclosure.policy_version {
            bail!("the issue discovery observation used a different disclosure policy")
        }
        if !page.repository.matches_reference(&observation.issue)
            || !seen.insert(queries::key(&observation.issue)?)
        {
            bail!("the issue discovery page contains a mismatched or duplicate stable identity")
        }
    }
    Ok(())
}

pub(super) fn resolve_binding(
    transaction: &Transaction<'_>,
    workspace: &str,
    control: &StoredIssueCollection,
    repository: &IssueSourceRepository,
) -> Result<IssueAutomationBinding> {
    if let Some(id) = &control.binding_id {
        let binding = read_binding(transaction, workspace, id)?
            .context("the issue collection binding is missing")?
            .binding;
        validate_repository(&binding, repository)?;
        return Ok(binding);
    }
    let existing: Option<String> = {
        let mut statement = transaction.prepare(
            "SELECT binding_id FROM issue_automation_bindings
             WHERE provider = ?1 AND server_id = ?2 AND repository_id = ?3 AND workspace_dir = ?4
             ORDER BY binding_id COLLATE BINARY LIMIT 1",
        )?;
        let mut rows = statement.query(params![
            enum_name(&repository.provider)?,
            repository.server_id,
            repository.repository_id,
            workspace,
        ])?;
        rows.next()?.map(|row| row.get(0)).transpose()?
    };
    if let Some(id) = existing {
        let binding = read_binding(transaction, workspace, &IssueAutomationBindingId::new(id)?)?
            .context("the issue collection binding disappeared")?
            .binding;
        validate_repository(&binding, repository)?;
        return Ok(binding);
    }
    // This identifies a local binding, never an issue. Remote issue IDs always come from the provider.
    let digest = Sha256::digest(serde_json::to_vec(repository)?);
    let binding = IssueAutomationBinding::new(
        IssueAutomationBindingId::new(format!("issue-source-{digest:x}"))?,
        repository.provider,
        repository.server_id.clone(),
        repository.repository_id.clone(),
        repository.project_scope.clone(),
    )?;
    validate_binding_scope(workspace, &binding)?;
    transaction.execute(
        "INSERT INTO issue_automation_bindings
             (binding_id, provider, server_id, repository_id, workspace_dir, autonomy, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
        params![
            binding.id().as_str(),
            enum_name(&binding.provider())?,
            binding.server_id(),
            binding.repository_id(),
            workspace,
            enum_name(&binding.autonomy())?,
        ],
    )?;
    Ok(binding)
}

fn validate_repository(
    binding: &IssueAutomationBinding,
    repository: &IssueSourceRepository,
) -> Result<()> {
    if binding.provider() != repository.provider
        || binding.server_id() != repository.server_id
        || binding.repository_id() != repository.repository_id
        || binding.project_scope() != &repository.project_scope
    {
        bail!("the issue collection is already bound to a different stable repository")
    }
    Ok(())
}

pub(super) fn apply(
    transaction: &Transaction<'_>,
    workspace: &str,
    default_binding: &IssueAutomationBinding,
    value: &IssueSourceRead,
    collection_revision: u64,
) -> Result<bool> {
    let reference = reference(value);
    validate_reference_scope(workspace, reference)?;
    let previous = read(transaction, workspace, reference)?;
    let tracked = read_issue(transaction, workspace, reference)?;
    if let (Some(observation), Some(tracked)) = (&previous, &tracked)
        && observation.binding_id != *tracked.binding_id()
    {
        bail!("the current issue observation and workflow row have different bindings")
    }
    let binding_id = tracked
        .as_ref()
        .map(|issue| issue.binding_id())
        .or_else(|| previous.as_ref().map(|item| &item.binding_id))
        .unwrap_or_else(|| default_binding.id());
    let binding = read_binding(transaction, workspace, binding_id)?
        .context("the source observation binding is missing")?
        .binding;
    validate_read(workspace, &binding, value)?;
    let newly_tracked = previous.is_none() && tracked.is_none();
    match tracked {
        Some(current) => {
            let mut candidate = current.clone();
            if let IssueSourceRead::Observed(observation) = value
                && let IssueSourceContent::Shared(snapshot) = &observation.content
            {
                candidate.replace_snapshot(snapshot.clone())?;
            }
            candidate.set_flag(
                IssueAutomationFlag::Unknown,
                matches!(value, IssueSourceRead::Unknown { .. }),
            );
            if !current.same_content(&candidate) {
                current.validate_replacement(&candidate)?;
                let mut record = candidate.to_record();
                record.revision = next_revision(current.revision())?;
                TrackedIssue::hydrate(record.clone())?;
                update_issue_row(transaction, current.revision(), &record)?;
            }
        }
        None => {
            if let IssueSourceRead::Observed(observation) = value
                && let IssueSourceContent::Shared(snapshot) = &observation.content
            {
                let issue =
                    TrackedIssue::discovered(reference.clone(), &binding, snapshot.clone())?;
                let mut record = issue.to_record();
                record.revision = 1;
                TrackedIssue::hydrate(record.clone())?;
                insert_issue_row(transaction, &record)?;
            }
        }
    }
    let encoded = serde_json::to_string(value)?;
    if encoded.len() > MAX_OBSERVATION_BYTES {
        bail!("the shared issue observation exceeds its storage bound")
    }
    transaction.execute(
        "INSERT INTO issue_source_observations
             (provider, server_id, repository_id, issue_id, workspace_dir, binding_id, current_read, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (provider, server_id, repository_id, issue_id, workspace_dir)
         DO UPDATE SET current_read = excluded.current_read, revision = excluded.revision",
        params![
            enum_name(&reference.provider)?, reference.server_id, reference.repository_id,
            reference.issue_id, workspace, binding.id().as_str(), encoded, sql_revision(collection_revision)?,
        ],
    )?;
    Ok(newly_tracked)
}

pub(super) fn read(
    connection: &Connection,
    workspace: &str,
    reference: &IssueReference,
) -> Result<Option<StoredObservation>> {
    let mut statement = connection.prepare(
        "SELECT binding_id, current_read, revision FROM issue_source_observations
         WHERE provider = ?1 AND server_id = ?2 AND repository_id = ?3 AND issue_id = ?4 AND workspace_dir = ?5",
    )?;
    let mut rows = statement.query(params![
        enum_name(&reference.provider)?,
        reference.server_id,
        reference.repository_id,
        reference.issue_id,
        workspace,
    ])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let binding_id = IssueAutomationBindingId::new(row.get::<_, String>(0)?)?;
    let encoded: String = row.get(1)?;
    read_revision(row, 2)?;
    if encoded.len() > MAX_OBSERVATION_BYTES {
        bail!("the stored issue observation exceeds its storage bound")
    }
    let current: IssueSourceRead =
        serde_json::from_str(&encoded).context("the stored issue observation is malformed")?;
    if self::reference(&current) != reference {
        bail!("the stored issue observation does not match its stable key")
    }
    let binding = read_binding(connection, workspace, &binding_id)?
        .context("the stored issue observation has no binding")?
        .binding;
    validate_read(workspace, &binding, &current)?;
    Ok(Some(StoredObservation {
        binding_id,
        current,
    }))
}

fn validate_read(
    workspace: &str,
    binding: &IssueAutomationBinding,
    value: &IssueSourceRead,
) -> Result<()> {
    let reference = reference(value);
    validate_reference_scope(workspace, reference)?;
    for text in [
        &reference.server_id,
        &reference.repository_id,
        &reference.issue_id,
    ] {
        validate_text(text)?;
    }
    if !binding.matches_reference(reference) {
        bail!("the issue source observation belongs to another binding")
    }
    if let IssueSourceRead::Observed(observation) = value {
        validate_observation(binding, observation)?;
    }
    Ok(())
}

fn validate_observation(
    binding: &IssueAutomationBinding,
    observation: &IssueSourceObservation,
) -> Result<()> {
    if let Some(web) = &observation.web {
        web.validate_for(&observation.issue)?;
    }
    match &observation.content {
        IssueSourceContent::Shared(snapshot) => {
            TrackedIssue::discovered(observation.issue.clone(), binding, snapshot.clone())?;
        }
        IssueSourceContent::Withheld {
            basis,
            policy_version,
            ..
        } => {
            validate_basis(basis)?;
            validate_text(policy_version)?;
        }
    }
    for label in &observation.labels.items {
        validate_text(&label.id)?;
        validate_text(&label.name)?;
    }
    for actor in observation
        .assignees
        .items
        .iter()
        .chain(observation.author.iter())
    {
        validate_text(&actor.id)?;
        validate_text(&actor.login)?;
    }
    for attribution in &observation.label_actors.items {
        validate_text(&attribution.label.id)?;
        validate_text(&attribution.label.name)?;
        validate_text(&attribution.event_id)?;
        validate_text(&attribution.created_at)?;
        if let Some(actor) = &attribution.actor {
            validate_text(&actor.id)?;
            validate_text(&actor.login)?;
        }
    }
    Ok(())
}

fn validate_basis(basis: &IssueComparisonBasis) -> Result<()> {
    let hash = &basis.field_hash;
    if hash.version == 0
        || hash.sha256.len() != 64
        || !hash
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("the withheld issue source fingerprint is invalid")
    }
    validate_text(&basis.provider_updated_at)
}

fn validate_text(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.contains('\0') {
        bail!("the issue source observation contains an invalid value")
    }
    Ok(())
}
