//! Transactional Observe control and checkpoints. Network calls remain outside this adapter.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, Transaction, params};

use crate::application::port::outbound::issue_automation_store_port::{
    IssueCollectionApplyResult, IssueCollectionCommit, IssueCollectionModeSaveResult,
};
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest,
};
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_automation::collection::{
    IssueCollectionConfig, IssueCollectionFailure, IssueCollectionFence, IssueCollectionMode,
    IssueCollectionPage, IssueCollectionProjection, IssueCollectionQuery, IssueCollectionStatus,
    IssueResyncBatch, IssueResyncCursor, StoredIssueCollection,
};
use crate::domain::issue_automation::source::{
    IssueDiscoveryCursor, IssueSourceRead, PersistedIssueDiscoveryCursor,
};

use super::{
    SqliteIssueAutomationStoreAdapter, decode_enum, enum_name, next_revision, read_binding,
    read_revision, sql_revision, transaction, validate_expected_revision, validate_reference_scope,
};
mod observations;
pub(super) mod queries;
mod schema;
pub(super) use schema::{CONTROL_SCHEMA, CONTROL_TABLE, OBSERVATIONS_SCHEMA, OBSERVATIONS_TABLE};

#[cfg(test)]
mod tests;

const CONTROL_COLUMNS: &str = "mode, epoch, revision, binding_id, discovery_cursor, resync_cursor, \
    last_attempt_at_ms, last_success_at_ms, last_failure, consecutive_failures";
const MAX_CONTROL_JSON_BYTES: usize = 128 * 1024;

pub(super) fn get(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
) -> Result<Option<StoredIssueCollection>> {
    adapter.with_connection(workspace, |connection, _guard| {
        read_control(connection, workspace)
    })
}

pub(super) fn projection(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
) -> Result<Option<IssueCollectionProjection>> {
    adapter.with_connection(workspace, |connection, guard| {
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let result = read_control(&transaction, workspace)?
            .map(|state| project_control(&transaction, workspace, &state))
            .transpose()?;
        guard.verify_identity()?;
        transaction.commit()?;
        Ok(result)
    })
}

pub(super) fn ensure(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
) -> Result<StoredIssueCollection> {
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            if let Some(current) = read_control(transaction, workspace)? {
                return Ok(current);
            }
            let state = initial(IssueCollectionMode::Observe);
            insert_control(transaction, workspace, &state)?;
            Ok(state)
        })
    })
}

pub(super) fn set_mode(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected_epoch: Option<u64>,
    mode: IssueCollectionMode,
) -> Result<IssueCollectionModeSaveResult> {
    if let Some(expected) = expected_epoch {
        validate_expected_revision(expected)?;
    }
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            let Some(current) = read_control(transaction, workspace)? else {
                if expected_epoch.is_some() {
                    return Ok(IssueCollectionModeSaveResult::Conflict { current: None });
                }
                let state = initial(mode);
                insert_control(transaction, workspace, &state)?;
                return saved_mode(transaction, workspace, state, true);
            };
            let same_mode = current.config.mode == mode;
            let expected = expected_epoch.unwrap_or(0);
            if current.epoch != expected {
                if same_mode && expected.checked_add(1) == Some(current.epoch) {
                    return saved_mode(transaction, workspace, current, false);
                }
                return Ok(IssueCollectionModeSaveResult::Conflict {
                    current: Some(Box::new(current)),
                });
            }
            if same_mode {
                return saved_mode(transaction, workspace, current, false);
            }
            let mut next = current.clone();
            next.config.mode = mode;
            next.epoch = next_revision(current.epoch)?;
            next.revision = next_revision(current.revision)?;
            save_control(transaction, workspace, current.fence(), &next)?;
            saved_mode(transaction, workspace, next, true)
        })
    })
}

/// 확인 창이 보여 준 mode와 두 판을 저장 직전 같은 transaction에서 확인한다.
pub(super) fn set_mode_exact(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueCollectionFence,
    from: IssueCollectionMode,
    mode: IssueCollectionMode,
) -> Result<IssueCollectionModeSaveResult> {
    expected.validate()?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            let Some(current) = read_control(transaction, workspace)? else {
                return Ok(IssueCollectionModeSaveResult::Conflict { current: None });
            };
            if current.fence() != expected || current.config.mode != from {
                return Ok(IssueCollectionModeSaveResult::Conflict {
                    current: Some(Box::new(current)),
                });
            }
            if current.config.mode == mode {
                return saved_mode(transaction, workspace, current, false);
            }
            let mut next = current.clone();
            next.config.mode = mode;
            next.epoch = next_revision(current.epoch)?;
            next.revision = next_revision(current.revision)?;
            save_control(transaction, workspace, current.fence(), &next)?;
            saved_mode(transaction, workspace, next, true)
        })
    })
}

pub(super) fn apply_discovery(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueCollectionFence,
    request: &IssueDiscoveryRequest,
    page: &IssueDiscoveryPage,
) -> Result<IssueCollectionApplyResult> {
    expected.validate()?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            let current = require_control(transaction, workspace)?;
            if !accepts(&current, expected) {
                return discarded(transaction, workspace, current);
            }
            observations::validate_page(workspace, request, page)?;
            if current.discovery_cursor != request.cursor {
                return discarded(transaction, workspace, current);
            }
            let mut next = current.clone();
            let binding =
                observations::resolve_binding(transaction, workspace, &current, &page.repository)?;
            next.binding_id = Some(binding.id().clone());
            next.discovery_cursor = page.next_cursor.clone();
            next.revision = next_revision(current.revision)?;
            let mut newly_tracked = Vec::new();
            for observation in &page.observations {
                if observations::apply(
                    transaction,
                    workspace,
                    &binding,
                    &IssueSourceRead::Observed(Box::new(observation.clone())),
                    next.revision,
                )? {
                    newly_tracked.push(observation.issue.clone());
                }
            }
            save_control(transaction, workspace, current.fence(), &next)?;
            applied(transaction, workspace, next, newly_tracked)
        })
    })
}

pub(super) fn reset_discovery(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueCollectionFence,
) -> Result<IssueCollectionApplyResult> {
    expected.validate()?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            let current = require_control(transaction, workspace)?;
            if !accepts(&current, expected) {
                return discarded(transaction, workspace, current);
            }
            if current.discovery_cursor.is_none() {
                return applied(transaction, workspace, current, Vec::new());
            }
            let mut next = current.clone();
            next.discovery_cursor = None;
            next.epoch = next_revision(current.epoch)?;
            next.revision = next_revision(current.revision)?;
            save_control(transaction, workspace, current.fence(), &next)?;
            applied(transaction, workspace, next, Vec::new())
        })
    })
}

pub(super) fn read_resync(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueCollectionFence,
    limit: u16,
) -> Result<Option<IssueResyncBatch>> {
    expected.validate()?;
    queries::validate_limit(limit)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            let current = require_control(transaction, workspace)?;
            if !accepts(&current, expected) {
                return Ok(None);
            }
            Ok(Some(queries::resync_batch(
                transaction,
                workspace,
                &current,
                limit,
            )?))
        })
    })
}

pub(super) fn apply_resync(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    batch: &IssueResyncBatch,
    reads: &[IssueSourceRead],
) -> Result<IssueCollectionApplyResult> {
    batch.expected.validate()?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            let current = require_control(transaction, workspace)?;
            if !accepts(&current, batch.expected) {
                return discarded(transaction, workspace, current);
            }
            if reads.len() != batch.targets.len() {
                bail!("the issue resync batch has an incomplete set of reads")
            }
            let limit = u16::try_from(batch.targets.len().max(1))
                .context("the issue resync batch is too large")?;
            queries::validate_limit(limit)?;
            // Do not let a caller skip rows or replace a checkpoint with arbitrary coordinates.
            let selected = queries::resync_batch(transaction, workspace, &current, limit)?;
            if selected != *batch {
                return discarded(transaction, workspace, current);
            }
            let mut next = current.clone();
            next.resync_cursor = batch.next_cursor.clone();
            next.revision = next_revision(current.revision)?;
            let mut newly_tracked = Vec::new();
            for (target, read) in batch.targets.iter().zip(reads) {
                if observations::reference(read) != target {
                    bail!("the issue resync result does not match its requested stable identity")
                }
                let binding = queries::binding_for_reference(transaction, workspace, target)?;
                if observations::apply(transaction, workspace, &binding, read, next.revision)? {
                    newly_tracked.push(target.clone());
                }
            }
            save_control(transaction, workspace, current.fence(), &next)?;
            applied(transaction, workspace, next, newly_tracked)
        })
    })
}

pub(super) fn record_attempt(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueCollectionFence,
    at_unix_ms: u64,
    failure: Option<IssueCollectionFailure>,
) -> Result<IssueCollectionApplyResult> {
    expected.validate()?;
    sql_revision(at_unix_ms)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            let current = require_control(transaction, workspace)?;
            if !accepts(&current, expected) {
                return discarded(transaction, workspace, current);
            }
            let mut next = current.clone();
            next.status.last_attempt_at_ms = Some(at_unix_ms);
            next.status.last_failure = failure;
            if failure.is_some() {
                next.status.consecutive_failures =
                    current.status.consecutive_failures.saturating_add(1);
            } else {
                next.status.last_success_at_ms = Some(at_unix_ms);
                next.status.consecutive_failures = 0;
            }
            next.revision = next_revision(current.revision)?;
            save_control(transaction, workspace, current.fence(), &next)?;
            applied(transaction, workspace, next, Vec::new())
        })
    })
}

pub(super) fn query(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    query: &IssueCollectionQuery,
) -> Result<IssueCollectionPage> {
    query.validate()?;
    if let Some(after) = &query.after {
        validate_reference_scope(workspace, after)?;
    }
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            queries::page(transaction, workspace, query)
        })
    })
}

fn initial(mode: IssueCollectionMode) -> StoredIssueCollection {
    StoredIssueCollection {
        config: IssueCollectionConfig { mode },
        epoch: 1,
        revision: 1,
        binding_id: None,
        discovery_cursor: None,
        resync_cursor: None,
        status: IssueCollectionStatus::default(),
    }
}

fn accepts(current: &StoredIssueCollection, expected: IssueCollectionFence) -> bool {
    current.config.mode == IssueCollectionMode::Observe && current.fence() == expected
}

fn project_control(
    connection: &Connection,
    workspace: &str,
    state: &StoredIssueCollection,
) -> Result<IssueCollectionProjection> {
    let binding = state
        .binding_id
        .as_ref()
        .map(|id| {
            read_binding(connection, workspace, id)?
                .map(|stored| stored.binding)
                .context("the issue collection binding is unavailable")
        })
        .transpose()?;
    Ok(IssueCollectionProjection {
        state: state.clone(),
        binding,
        catalog_revision: super::triage::catalog_revision(connection, workspace)?,
    })
}

fn saved_mode(
    connection: &Connection,
    workspace: &str,
    state: StoredIssueCollection,
    changed: bool,
) -> Result<IssueCollectionModeSaveResult> {
    let projection = project_control(connection, workspace, &state)?;
    Ok(IssueCollectionModeSaveResult::Saved {
        state: Box::new(state),
        projection: Box::new(projection),
        changed,
    })
}

fn discarded(
    connection: &Connection,
    workspace: &str,
    current: StoredIssueCollection,
) -> Result<IssueCollectionApplyResult> {
    let projection = project_control(connection, workspace, &current)?;
    Ok(IssueCollectionApplyResult::Discarded {
        current: Box::new(current),
        projection: Box::new(projection),
    })
}

fn applied(
    connection: &Connection,
    workspace: &str,
    state: StoredIssueCollection,
    newly_tracked: Vec<crate::domain::issue_work::IssueReference>,
) -> Result<IssueCollectionApplyResult> {
    let projection = project_control(connection, workspace, &state)?;
    Ok(IssueCollectionApplyResult::Applied(Box::new(
        IssueCollectionCommit {
            state,
            projection,
            newly_tracked,
        },
    )))
}

fn require_control(connection: &Connection, workspace: &str) -> Result<StoredIssueCollection> {
    read_control(connection, workspace)?
        .context("issue collection must be committed locally before collection starts")
}

pub(super) fn read_control(
    connection: &Connection,
    workspace: &str,
) -> Result<Option<StoredIssueCollection>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {CONTROL_COLUMNS} FROM issue_collection_control WHERE workspace_dir = ?1"
    ))?;
    let mut rows = statement.query([workspace])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let binding: Option<String> = row.get(3)?;
    let discovery: Option<String> = row.get(4)?;
    let resync: Option<String> = row.get(5)?;
    let failure: Option<String> = row.get(8)?;
    let state = StoredIssueCollection {
        config: IssueCollectionConfig {
            mode: decode_enum(row.get(0)?)?,
        },
        epoch: read_revision(row, 1)?,
        revision: read_revision(row, 2)?,
        binding_id: binding.map(IssueAutomationBindingId::new).transpose()?,
        discovery_cursor: discovery
            .map(|value| decode_json::<PersistedIssueDiscoveryCursor>(&value))
            .transpose()?
            .map(IssueDiscoveryCursor::hydrate)
            .transpose()?,
        resync_cursor: resync
            .map(|value| decode_json::<IssueResyncCursor>(&value))
            .transpose()?,
        status: IssueCollectionStatus {
            last_attempt_at_ms: read_timestamp(row, 6)?,
            last_success_at_ms: read_timestamp(row, 7)?,
            last_failure: failure.map(|value| decode_json(&value)).transpose()?,
            consecutive_failures: u32::try_from(row.get::<_, i64>(9)?)
                .context("the stored issue collection failure count is invalid")?,
        },
    };
    validate_control(connection, workspace, &state)?;
    Ok(Some(state))
}

fn validate_control(
    connection: &Connection,
    workspace: &str,
    state: &StoredIssueCollection,
) -> Result<()> {
    state.validate()?;
    if let Some(id) = &state.binding_id {
        let binding = read_binding(connection, workspace, id)?
            .context("the issue collection references a missing binding")?
            .binding;
        if let Some(cursor) = &state.discovery_cursor {
            let repository = cursor.to_record().repository;
            if repository.provider != binding.provider()
                || repository.server_id != binding.server_id()
                || repository.repository_id != binding.repository_id()
                || repository.project_scope != *binding.project_scope()
            {
                bail!("the issue discovery checkpoint belongs to another repository")
            }
        }
    }
    if let Some(cursor) = &state.resync_cursor {
        validate_reference_scope(workspace, &cursor.after)?;
        validate_reference_scope(workspace, &cursor.through)?;
        if queries::key(&cursor.after)? > queries::key(&cursor.through)? {
            bail!("the issue resync checkpoint is out of order")
        }
    }
    Ok(())
}

fn insert_control(
    transaction: &Transaction<'_>,
    workspace: &str,
    state: &StoredIssueCollection,
) -> Result<()> {
    state.validate()?;
    transaction.execute(
        "INSERT INTO issue_collection_control (workspace_dir, mode, epoch, revision, consecutive_failures)
         VALUES (?1, ?2, ?3, ?4, 0)",
        params![workspace, enum_name(&state.config.mode)?, sql_revision(state.epoch)?, sql_revision(state.revision)?],
    )?;
    super::triage::bump_catalog(transaction, workspace)?;
    Ok(())
}

fn save_control(
    transaction: &Transaction<'_>,
    workspace: &str,
    expected: IssueCollectionFence,
    state: &StoredIssueCollection,
) -> Result<()> {
    validate_control(transaction, workspace, state)?;
    let discovery = state
        .discovery_cursor
        .as_ref()
        .map(|cursor| encode_json(&cursor.to_record()))
        .transpose()?;
    let resync = state.resync_cursor.as_ref().map(encode_json).transpose()?;
    let failure = state
        .status
        .last_failure
        .as_ref()
        .map(encode_json)
        .transpose()?;
    let changed = transaction.execute(
        "UPDATE issue_collection_control SET mode = ?1, epoch = ?2, revision = ?3,
             binding_id = ?4, discovery_cursor = ?5, resync_cursor = ?6,
             last_attempt_at_ms = ?7, last_success_at_ms = ?8, last_failure = ?9,
             consecutive_failures = ?10
         WHERE workspace_dir = ?11 AND epoch = ?12 AND revision = ?13",
        params![
            enum_name(&state.config.mode)?,
            sql_revision(state.epoch)?,
            sql_revision(state.revision)?,
            state
                .binding_id
                .as_ref()
                .map(IssueAutomationBindingId::as_str),
            discovery,
            resync,
            state
                .status
                .last_attempt_at_ms
                .map(sql_revision)
                .transpose()?,
            state
                .status
                .last_success_at_ms
                .map(sql_revision)
                .transpose()?,
            failure,
            i64::from(state.status.consecutive_failures),
            workspace,
            sql_revision(expected.epoch)?,
            sql_revision(expected.revision)?,
        ],
    )?;
    if changed != 1 {
        bail!("the issue collection changed during its transaction")
    }
    super::triage::bump_catalog(transaction, workspace)?;
    Ok(())
}

fn encode_json<T: serde::Serialize>(value: &T) -> Result<String> {
    let encoded = serde_json::to_string(value)?;
    if encoded.len() > MAX_CONTROL_JSON_BYTES {
        bail!("the issue collection checkpoint is too large")
    }
    Ok(encoded)
}

fn decode_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    if value.len() > MAX_CONTROL_JSON_BYTES {
        bail!("the stored issue collection checkpoint is too large")
    }
    serde_json::from_str(value).context("the stored issue collection checkpoint is malformed")
}

fn read_timestamp(row: &rusqlite::Row<'_>, index: usize) -> Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).context("the stored issue collection timestamp is invalid")
        })
        .transpose()
}

/// A later workflow row must inherit an earlier observation-only row's binding.
pub(super) fn validate_new_workflow_binding(
    connection: &Connection,
    workspace: &str,
    issue: &crate::domain::issue_automation::PersistedTrackedIssue,
) -> Result<()> {
    if let Some(observation) = observations::read(connection, workspace, &issue.reference)?
        && observation.binding_id != issue.binding_id
    {
        bail!("the issue workflow row cannot replace its current observation binding")
    }
    Ok(())
}

/// Fresh approval resync shares the normal observation transaction, without consuming its cursor.
pub(super) fn apply_one(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueCollectionFence,
    issue: &crate::domain::issue_work::IssueReference,
    read: &IssueSourceRead,
) -> Result<IssueCollectionApplyResult> {
    expected.validate()?;
    validate_reference_scope(workspace, issue)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let current = require_control(tx, workspace)?;
            if !accepts(&current, expected) {
                return discarded(tx, workspace, current);
            }
            if observations::reference(read) != issue {
                bail!("the fresh issue result has another identity");
            }
            let binding = queries::binding_for_reference(tx, workspace, issue)?;
            let mut next = current.clone();
            next.revision = next_revision(current.revision)?;
            let inserted = observations::apply(tx, workspace, &binding, read, next.revision)?;
            save_control(tx, workspace, current.fence(), &next)?;
            applied(
                tx,
                workspace,
                next,
                if inserted {
                    vec![issue.clone()]
                } else {
                    Vec::new()
                },
            )
        })
    })
}
