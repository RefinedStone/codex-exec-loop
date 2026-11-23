//! 판정은 workflow와 독립된 CAS 행이다. 관찰 판과 gate 판은 서로 대체하지 않는다.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::application::port::outbound::issue_automation_store_port::{
    IssueTriageBatch, IssueTriageFence, IssueTriageSaveResult,
};
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_automation::collection::{
    IssueCollectionItem, IssueCollectionMode, IssueCollectionQuery, IssueTriageProjection,
};
use crate::domain::issue_automation::triage::{
    IssueTriageAssessment, PersistedIssueTriageAssessment,
};
use crate::domain::issue_work::IssueReference;

use super::collection::{queries, read_control};
use super::{
    SqliteIssueAutomationStoreAdapter, enum_name, next_revision, read_revision, sql_revision,
    transaction, validate_expected_revision, validate_reference_scope,
};

pub(super) const CATALOG_TABLE: &str = "issue_automation_catalog";
pub(super) const ASSESSMENTS_TABLE: &str = "issue_triage_assessments";
pub(super) const CATALOG_SCHEMA: &str = r#"
CREATE TABLE issue_automation_catalog (
    workspace_dir TEXT COLLATE BINARY NOT NULL PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision >= 1)
) STRICT, WITHOUT ROWID
"#;
pub(super) const ASSESSMENTS_SCHEMA: &str = r#"
CREATE TABLE issue_triage_assessments (
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    binding_id TEXT COLLATE BINARY NOT NULL,
    observation_revision INTEGER CHECK (observation_revision IS NULL OR observation_revision >= 1),
    tracked_revision INTEGER CHECK (tracked_revision IS NULL OR tracked_revision >= 1),
    input_fingerprint TEXT NOT NULL CHECK (length(input_fingerprint) = 64),
    assessment TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (provider, server_id, repository_id, issue_id, workspace_dir),
    FOREIGN KEY (binding_id, provider, server_id, repository_id, workspace_dir)
        REFERENCES issue_automation_bindings
            (binding_id, provider, server_id, repository_id, workspace_dir)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
const MAX_ASSESSMENT_BYTES: usize = 512 * 1024;
const SCAN_PAGE_SIZE: u16 = 100;

pub(super) fn catalog_revision(connection: &Connection, workspace: &str) -> Result<u64> {
    let value: Option<i64> = connection
        .query_row(
            "SELECT revision FROM issue_automation_catalog WHERE workspace_dir = ?1",
            [workspace],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|value| {
            let revision = u64::try_from(value).context("the catalog revision is invalid")?;
            validate_expected_revision(revision)?;
            Ok(revision)
        })
        .transpose()
        .map(|revision| revision.unwrap_or(0))
}

pub(super) fn bump_catalog(transaction: &Transaction<'_>, workspace: &str) -> Result<u64> {
    let current = catalog_revision(transaction, workspace)?;
    let next = next_revision(current)?;
    if current == 0 {
        transaction.execute(
            "INSERT INTO issue_automation_catalog (workspace_dir, revision) VALUES (?1, ?2)",
            params![workspace, sql_revision(next)?],
        )?;
    } else {
        let changed = transaction.execute(
            "UPDATE issue_automation_catalog SET revision = ?1
             WHERE workspace_dir = ?2 AND revision = ?3",
            params![sql_revision(next)?, workspace, sql_revision(current)?],
        )?;
        if changed != 1 {
            bail!("the issue catalog changed during its transaction")
        }
    }
    Ok(next)
}

pub(super) fn seed_catalog(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(
        "INSERT INTO issue_automation_catalog (workspace_dir, revision)
         SELECT workspace_dir, max(revision) FROM (
             SELECT workspace_dir, revision FROM issue_automation_bindings
             UNION ALL SELECT workspace_dir, revision FROM tracked_issues
             UNION ALL SELECT workspace_dir, revision FROM issue_collection_control
             UNION ALL SELECT workspace_dir, revision FROM issue_source_observations
         ) GROUP BY workspace_dir",
    )?;
    Ok(())
}

pub(super) fn scan(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    binding_id: Option<&IssueAutomationBindingId>,
    visitor: &mut dyn FnMut(IssueCollectionItem) -> Result<()>,
) -> Result<u64> {
    adapter.with_connection(workspace, |connection, guard| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let revision = catalog_revision(&transaction, workspace)?;
        let mut after = None;
        loop {
            let page = queries::page(
                &transaction,
                workspace,
                &IssueCollectionQuery {
                    binding_id: binding_id.cloned(),
                    after,
                    limit: SCAN_PAGE_SIZE,
                },
            )?;
            for item in page.items {
                visitor(item)?;
            }
            match page.next_after {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        guard.verify_identity()?;
        transaction.commit()?;
        Ok(revision)
    })
}

pub(super) fn batch(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    after: Option<&IssueReference>,
    limit: u16,
) -> Result<IssueTriageBatch> {
    let query = IssueCollectionQuery {
        binding_id: None,
        after: after.cloned(),
        limit,
    };
    query.validate()?;
    adapter.with_connection(workspace, |connection, guard| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let collection = read_control(&transaction, workspace)?;
        let page = queries::page(&transaction, workspace, &query)?;
        guard.verify_identity()?;
        transaction.commit()?;
        Ok(IssueTriageBatch {
            collection,
            revision: page.revision,
            items: page.items,
            next_after: page.next_after,
        })
    })
}

pub(super) fn read(
    connection: &Connection,
    workspace: &str,
    reference: &IssueReference,
    binding_id: &IssueAutomationBindingId,
) -> Result<Option<IssueTriageProjection>> {
    let mut statement = connection.prepare(
        "SELECT binding_id, observation_revision, tracked_revision, input_fingerprint, assessment, revision
         FROM issue_triage_assessments
         WHERE provider = ?1 AND server_id = ?2 AND repository_id = ?3 AND issue_id = ?4
           AND workspace_dir = ?5",
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
    if row.get::<_, String>(0)? != binding_id.as_str() {
        bail!("the stored issue assessment belongs to a different binding")
    }
    let observation_revision = optional_revision(row, 1)?;
    let tracked_revision = optional_revision(row, 2)?;
    let fingerprint: String = row.get(3)?;
    let encoded: String = row.get(4)?;
    if encoded.len() > MAX_ASSESSMENT_BYTES {
        bail!("the stored issue assessment exceeds its storage bound")
    }
    let persisted: PersistedIssueTriageAssessment =
        serde_json::from_str(&encoded).context("the stored issue assessment is malformed")?;
    let assessment = IssueTriageAssessment::hydrate(persisted)?;
    assessment.validate_for(reference)?;
    if assessment.input_fingerprint() != fingerprint {
        bail!("the stored issue assessment fingerprint is inconsistent")
    }
    Ok(Some(IssueTriageProjection {
        assessment,
        revision: read_revision(row, 5)?,
        observation_revision,
        tracked_revision,
        stale: false,
    }))
}

pub(super) fn save(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueTriageFence,
    assessment: &IssueTriageAssessment,
) -> Result<IssueTriageSaveResult> {
    validate_expected_revision(expected.collection_epoch)?;
    super::sql_revision(expected.binding_run_revision)?;
    for revision in [
        expected.observation_revision,
        expected.tracked_revision,
        expected.assessment_revision,
    ]
    .into_iter()
    .flatten()
    {
        validate_expected_revision(revision)?;
    }
    let reference = &assessment.basis().issue;
    validate_reference_scope(workspace, reference)?;
    assessment.validate_for(reference)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            if !super::autonomy::permits_assessment(transaction, workspace, assessment.basis().runtime_selection_revision)? {
                return Ok(IssueTriageSaveResult::Conflict);
            }
            let Some(collection) = read_control(transaction, workspace)? else {
                return Ok(IssueTriageSaveResult::Conflict);
            };
            if collection.epoch != expected.collection_epoch {
                return Ok(IssueTriageSaveResult::Conflict);
            }
            if collection.config.mode != IssueCollectionMode::Observe
                && super::coordinator::read(transaction, workspace, reference)?
                    .is_none_or(|work| work.task_ids.is_empty())
            {
                // A paused collector still tracks an already registered execution.
                // It cannot assess or admit a new issue through this exception.
                return Ok(IssueTriageSaveResult::Conflict);
            }
            let Some(item) = queries::item(transaction, workspace, reference)? else {
                return Ok(IssueTriageSaveResult::Conflict);
            };
            if item.binding_run_revision != expected.binding_run_revision
                || item.observation_revision != expected.observation_revision
                || item.tracked_revision != expected.tracked_revision
            {
                return Ok(IssueTriageSaveResult::Conflict);
            }
            assessment.validate_current(
                reference,
                item.current.as_ref(),
                item.tracked.as_ref().is_some_and(|tracked| tracked.flags.held),
            )?;
            let current = item.triage.as_ref();
            let current_revision = current.map(|current| current.revision);
            if current_revision != expected.assessment_revision {
                if let Some(current) = current
                    && expected.assessment_revision.unwrap_or(0).checked_add(1) == Some(current.revision)
                    && current.assessment == *assessment
                    && current.observation_revision == item.observation_revision
                    && current.tracked_revision == item.tracked_revision
                {
                    return saved(
                        current.clone(),
                        item.binding_id,
                        catalog_revision(transaction, workspace)?,
                        false,
                    );
                }
                return Ok(IssueTriageSaveResult::Conflict);
            }
            assessment.validate_successor(current.map(|current| &current.assessment))?;
            if let Some(current) = current
                && current.assessment == *assessment
                && current.observation_revision == item.observation_revision
                && current.tracked_revision == item.tracked_revision
            {
                return saved(
                    current.clone(),
                    item.binding_id,
                    catalog_revision(transaction, workspace)?,
                    false,
                );
            }
            let encoded = serde_json::to_string(&assessment.to_record())?;
            if encoded.len() > MAX_ASSESSMENT_BYTES {
                bail!("the shared issue assessment exceeds its storage bound")
            }
            let changed = current.is_none_or(|current| current.assessment != *assessment);
            let revision = next_revision(current_revision.unwrap_or(0))?;
            transaction.execute(
                "INSERT INTO issue_triage_assessments
                     (provider, server_id, repository_id, issue_id, workspace_dir, binding_id,
                      observation_revision, tracked_revision, input_fingerprint, assessment, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (provider, server_id, repository_id, issue_id, workspace_dir)
                 DO UPDATE SET observation_revision = excluded.observation_revision,
                     tracked_revision = excluded.tracked_revision,
                     input_fingerprint = excluded.input_fingerprint,
                     assessment = excluded.assessment, revision = excluded.revision",
                params![
                    enum_name(&reference.provider)?, reference.server_id, reference.repository_id,
                    reference.issue_id, workspace, item.binding_id.as_str(),
                    item.observation_revision.map(sql_revision).transpose()?,
                    item.tracked_revision.map(sql_revision).transpose()?,
                    assessment.input_fingerprint(), encoded, sql_revision(revision)?,
                ],
            )?;
            let catalog_revision = bump_catalog(transaction, workspace)?;
            saved(
                IssueTriageProjection {
                    assessment: assessment.clone(),
                    revision,
                    observation_revision: item.observation_revision,
                    tracked_revision: item.tracked_revision,
                    stale: false,
                },
                item.binding_id,
                catalog_revision,
                changed,
            )
        })
    })
}

fn saved(
    record: IssueTriageProjection,
    binding_id: IssueAutomationBindingId,
    catalog_revision: u64,
    changed: bool,
) -> Result<IssueTriageSaveResult> {
    Ok(IssueTriageSaveResult::Saved {
        record: Box::new(record),
        binding_id,
        catalog_revision,
        changed,
    })
}

fn optional_revision(row: &rusqlite::Row<'_>, index: usize) -> Result<Option<u64>> {
    let value: Option<i64> = row.get(index)?;
    value
        .map(|value| {
            let revision =
                u64::try_from(value).context("the stored assessment revision is invalid")?;
            validate_expected_revision(revision)?;
            Ok(revision)
        })
        .transpose()
}

#[cfg(test)]
mod tests;
