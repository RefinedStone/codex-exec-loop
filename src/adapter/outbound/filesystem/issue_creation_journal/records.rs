//! Shared bounded journal format and validation for Unix and Windows.
use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::domain::issue_creation::{
    IssueCreationIntentRecord, IssueCreationIntentState, IssueRepositoryReference,
};
use crate::domain::issue_work::{ISSUE_FIELD_HASH_VERSION, IssueFieldHash, IssueProvider};

pub(super) const MAX_JOURNAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;

pub(super) fn decode_records(bytes: &[u8]) -> Result<Vec<IssueCreationIntentRecord>> {
    if bytes.len() > MAX_JOURNAL_BYTES {
        bail!("the issue creation journal exceeds its byte limit");
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| anyhow!("the issue creation journal is not UTF-8"))?;
    let mut records = Vec::new();
    let mut ids = HashSet::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if records.len() >= MAX_RECORDS {
            bail!("the issue creation journal contains too many records")
        }
        // Do not include source text in the parse error.
        let record: IssueCreationIntentRecord = serde_json::from_str(line)
            .map_err(|_| anyhow!("the issue creation journal contains an invalid record"))?;
        validate_record(&record)?;
        if !ids.insert(record.intent_id.clone()) {
            bail!("the issue creation journal contains duplicate intent IDs")
        }
        records.push(record);
    }
    Ok(records)
}

pub(super) fn upsert(
    records: &mut Vec<IssueCreationIntentRecord>,
    workspace: &str,
    record: &IssueCreationIntentRecord,
) -> Result<()> {
    validate_record(record)?;
    if record.repository.project_scope.workspace_dir != workspace {
        bail!("the creation intent belongs to a different workspace")
    }
    match records
        .iter_mut()
        .find(|existing| existing.intent_id == record.intent_id)
    {
        Some(existing) => {
            if existing.repository != record.repository
                || existing.predecessor_intent_id != record.predecessor_intent_id
                || (existing.approved.is_some() && existing.approved != record.approved)
            {
                bail!("an existing creation intent or approved copy cannot be replaced")
            }
            *existing = record.clone();
        }
        None => {
            if records.len() >= MAX_RECORDS {
                bail!("the issue creation journal contains too many records")
            }
            records.push(record.clone());
        }
    }
    Ok(())
}

pub(super) fn encode_records(records: &[IssueCreationIntentRecord]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for record in records {
        serde_json::to_writer(&mut body, record)
            .context("the issue creation journal could not be encoded")?;
        body.push(b'\n');
        if body.len() > MAX_JOURNAL_BYTES {
            bail!("the issue creation journal is full")
        }
    }
    Ok(body)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-=".contains(&byte))
}

fn valid_hash(hash: &IssueFieldHash) -> bool {
    hash.version == ISSUE_FIELD_HASH_VERSION
        && hash.sha256.len() == 64
        && hash
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_repository(repository: &IssueRepositoryReference) -> Result<()> {
    if !valid_id(&repository.repository_id)
        || repository.server_id != "github.com"
        || repository.provider != IssueProvider::GitHub
        || !Path::new(&repository.project_scope.workspace_dir).is_absolute()
    {
        bail!("the issue creation journal contains an invalid repository")
    }
    Ok(())
}

fn validate_record(record: &IssueCreationIntentRecord) -> Result<()> {
    validate_repository(&record.repository)?;
    if !valid_id(&record.intent_id)
        || record
            .predecessor_intent_id
            .as_ref()
            .is_some_and(|previous| !valid_id(previous) || previous == &record.intent_id)
    {
        bail!("the issue creation journal contains an invalid intent ID")
    }
    if record
        .observed_hash
        .as_ref()
        .is_some_and(|hash| !valid_hash(hash))
    {
        bail!("the issue creation journal contains an invalid observed hash")
    }
    match &record.approved {
        Some(approved) => {
            if approved.approval.intent_id != record.intent_id
                || approved.approval.repository != record.repository
                || !valid_hash(&approved.approval.shared_copy_hash)
                || approved.approval.shared_copy_hash
                    != IssueFieldHash::of_fields(
                        &approved.shared_copy.title,
                        &approved.shared_copy.body,
                    )
            {
                bail!("the issue creation journal contains an inconsistent approved copy")
            }
        }
        None => {
            if !matches!(
                record.state,
                IssueCreationIntentState::Prepared | IssueCreationIntentState::Abandoned
            ) || !record.pending_vault_fields.is_empty()
                || record.created.is_some()
                || record.observed_hash.is_some()
                || record.restore_link_finalized
                || record.write_response_mismatch
            {
                bail!("the issue creation journal contains an unapproved result")
            }
        }
    }
    if record.pending_vault_fields.len() > 2
        || (record.pending_vault_fields.len() == 2
            && record.pending_vault_fields[0] == record.pending_vault_fields[1])
    {
        bail!("the issue creation journal contains duplicate Vault fields")
    }
    if let Some(created) = &record.created {
        let locator = &created.locator;
        let valid_name = |name: &str, extra: &[u8]| {
            !name.is_empty()
                && name.len() <= 100
                && name != "."
                && name != ".."
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || extra.contains(&byte))
        };
        if !record.repository.contains(&created.issue)
            || !valid_id(&created.issue.issue_id)
            || !valid_name(&locator.owner, b"-_")
            || !valid_name(&locator.repository, b"-_.")
            || locator.number == 0
            || locator.number > i32::MAX as u64
            || created.url
                != format!(
                    "https://github.com/{}/{}/issues/{}",
                    locator.owner, locator.repository, locator.number
                )
        {
            bail!("the issue creation journal contains a different creation target")
        }
    }
    if record.state == IssueCreationIntentState::Confirmed
        && (record.created.is_none()
            || record.observed_hash.as_ref()
                != record
                    .approved
                    .as_ref()
                    .map(|approved| &approved.approval.shared_copy_hash))
    {
        bail!("the issue creation journal contains an unverified confirmed result")
    }
    if record.restore_link_finalized && record.state != IssueCreationIntentState::Confirmed {
        bail!("the issue creation journal contains an unconfirmed restore link")
    }
    Ok(())
}
