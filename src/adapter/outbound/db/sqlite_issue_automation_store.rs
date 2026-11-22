//! Separate, shared-copy-only issue automation ledger (#2536).
//! The composition supplies a private runtime directory. This adapter never opens planning files.
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationBindingInsertResult, IssueAutomationBindingSaveResult,
    IssueAutomationIssueInsertResult, IssueAutomationIssueSaveResult, IssueAutomationStorePort,
    IssueCollectionApplyResult, IssueCollectionModeSaveResult, IssueDecisionCommit,
    IssueDecisionFence, IssueDecisionSaveResult, IssueTriageBatch, IssueTriageFence,
    IssueTriageSaveResult, StoredIssueAutomationBinding,
};
use crate::domain::issue_automation::{
    IssueAutomationAutonomy, IssueAutomationBinding, IssueAutomationBindingId,
    IssueAutomationFlags, IssueAutomationSnapshot, IssueAutomationState,
    PersistedIssueAutomationBinding, PersistedTrackedIssue, TrackedIssue,
};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueReference, IssueSharedCopy,
};

use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest,
};
use crate::domain::issue_automation::collection::{
    IssueCollectionFailure, IssueCollectionFence, IssueCollectionItem, IssueCollectionMode,
    IssueCollectionPage, IssueCollectionProjection, IssueCollectionQuery, IssueResyncBatch,
    StoredIssueCollection,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionId, IssueDecisionRequest, StoredIssueDecision,
};
use crate::domain::issue_automation::source::IssueSourceRead;
use crate::domain::issue_automation::triage::IssueTriageAssessment;

mod autonomy;
mod collection;
mod coordinator;
mod decision;
mod events;
mod execution;
mod execution_answers;
mod intervention;
mod private_store;
mod triage;

#[cfg(test)]
mod tests;

type RuntimeDirectoryResolver = dyn Fn(&str) -> Result<PathBuf> + Send + Sync;

const APPLICATION_ID: i64 = 0x414b4941;
const SCHEMA_VERSION: i64 = 8;
const BINDINGS_TABLE: &str = "issue_automation_bindings";
const ISSUES_TABLE: &str = "tracked_issues";

const BINDINGS_SCHEMA: &str = r#"
CREATE TABLE issue_automation_bindings (
    binding_id TEXT COLLATE BINARY NOT NULL PRIMARY KEY,
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    autonomy TEXT NOT NULL CHECK (autonomy IN ('off', 'observe', 'propose', 'gated', 'auto')),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    UNIQUE (binding_id, provider, server_id, repository_id, workspace_dir)
) WITHOUT ROWID
"#;

const ISSUES_SCHEMA: &str = r#"
CREATE TABLE tracked_issues (
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    binding_id TEXT COLLATE BINARY NOT NULL,
    source_hash_version INTEGER NOT NULL CHECK (source_hash_version >= 1),
    source_hash_sha256 TEXT NOT NULL,
    shared_title TEXT NOT NULL,
    shared_body TEXT NOT NULL,
    provider_updated_at TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'discovered', 'ineligible', 'needs_info', 'awaiting_decision', 'ready',
        'queued', 'running', 'in_review', 'done', 'failed', 'cancelled'
    )),
    held INTEGER NOT NULL CHECK (held IN (0, 1)),
    stale INTEGER NOT NULL CHECK (stale IN (0, 1)),
    unknown INTEGER NOT NULL CHECK (unknown IN (0, 1)),
    decision_resume TEXT CHECK (decision_resume IS NULL OR decision_resume IN (
        'discovered', 'ready', 'running', 'in_review', 'failed'
    )),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    CHECK ((state = 'awaiting_decision') = (decision_resume IS NOT NULL)),
    PRIMARY KEY (provider, server_id, repository_id, issue_id, workspace_dir),
    FOREIGN KEY (binding_id, provider, server_id, repository_id, workspace_dir)
        REFERENCES issue_automation_bindings
            (binding_id, provider, server_id, repository_id, workspace_dir)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) WITHOUT ROWID
"#;

const BINDING_COLUMNS: &str =
    "binding_id, provider, server_id, repository_id, workspace_dir, autonomy, revision";
const ISSUE_COLUMNS: &str = "provider, server_id, repository_id, issue_id, workspace_dir, binding_id, source_hash_version, \
     source_hash_sha256, shared_title, shared_body, provider_updated_at, policy_version, state, \
     held, stale, unknown, decision_resume, revision";

pub struct SqliteIssueAutomationStoreAdapter {
    resolve_runtime_dir: Box<RuntimeDirectoryResolver>,
}

impl SqliteIssueAutomationStoreAdapter {
    pub fn new(
        resolve_runtime_dir: impl Fn(&str) -> Result<PathBuf> + Send + Sync + 'static,
    ) -> Self {
        Self {
            resolve_runtime_dir: Box::new(resolve_runtime_dir),
        }
    }

    fn with_connection<T>(
        &self,
        workspace_dir: &str,
        work: impl FnOnce(&mut Connection, &private_store::PrivateStoreGuard) -> Result<T>,
    ) -> Result<T> {
        validate_workspace(workspace_dir)?;
        let runtime = (self.resolve_runtime_dir)(workspace_dir)?;
        let mut opened = private_store::open(&runtime)?;
        let (connection, guard) = opened.parts();
        ensure_schema(connection, guard)?;
        guard.verify_identity()?;
        let result = work(connection, guard)?;
        guard.verify_identity()?;
        Ok(result)
    }
}

impl IssueAutomationStorePort for SqliteIssueAutomationStoreAdapter {
    fn issue_autonomy_selection(
        &self,
        workspace: &str,
    ) -> Result<crate::domain::issue_automation::autonomy::IssueAutonomySelection> {
        self.with_connection(workspace, |connection, _| {
            autonomy::read(connection, workspace)
        })
    }
    fn get_issue_autonomy_command(
        &self,
        workspace: &str,
        id: &str,
    ) -> Result<Option<crate::domain::issue_automation::autonomy::StoredIssueAutonomyCommand>> {
        self.with_connection(workspace, |connection, _| {
            autonomy::command(connection, workspace, id)
        })
    }
    fn begin_issue_autonomy(
        &self,
        workspace: &str,
        intent: &crate::domain::issue_automation::autonomy::IssueAutonomyIntent,
    ) -> Result<crate::domain::issue_automation::autonomy::StoredIssueAutonomyCommand> {
        autonomy::begin(self, workspace, intent)
    }
    fn claim_issue_autonomy_fence(
        &self,
        workspace: &str,
        intent: &crate::domain::issue_automation::autonomy::IssueAutonomyIntent,
        work: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint,
    ) -> Result<crate::domain::issue_automation::intervention::StoredIssueIntervention> {
        autonomy::claim_fence(self, workspace, intent, work)
    }
    fn finish_issue_autonomy(
        &self,
        workspace: &str,
        intent: &crate::domain::issue_automation::autonomy::IssueAutonomyIntent,
    ) -> Result<crate::domain::issue_automation::autonomy::IssueAutonomyCommit> {
        autonomy::finish(self, workspace, intent)
    }
    fn list_issue_work(
        &self,
        workspace: &str,
    ) -> Result<Vec<crate::domain::issue_automation::coordinator::IssueWorkCheckpoint>> {
        coordinator::list(self, workspace)
    }
    fn closed_issue_decisions(
        &self,
        workspace: &str,
        issue: &IssueReference,
        limit: u16,
    ) -> Result<Vec<StoredIssueDecision>> {
        decision::closed(self, workspace, issue, limit)
    }

    fn issue_authorization_epoch(&self, workspace: &str, issue: &IssueReference) -> Result<u64> {
        validate_reference_scope(workspace, issue)?;
        self.with_connection(workspace, |connection, _| {
            intervention::authorization_epoch(connection, workspace, issue)
        })
    }

    fn pending_issue_work_events(
        &self,
        workspace: &str,
    ) -> Result<Vec<crate::domain::issue_automation::coordinator::IssueWorkEvent>> {
        events::pending(self, workspace)
    }
    fn acknowledge_issue_work_event(&self, workspace: &str, event_id: &str) -> Result<()> {
        events::acknowledge(self, workspace, event_id)
    }
    fn get_issue_intervention(
        &self,
        workspace: &str,
        command_id: &str,
    ) -> Result<Option<crate::domain::issue_automation::intervention::StoredIssueIntervention>>
    {
        intervention::get(self, workspace, command_id)
    }
    fn claim_issue_steer_delivery(
        &self,
        workspace: &str,
        request: &crate::domain::issue_automation::intervention::IssueInterventionRequest,
        receipt: &crate::domain::issue_automation::intervention::IssueInterventionReceipt,
    ) -> Result<bool> {
        intervention::claim_steer(self, workspace, request, receipt)
    }
    fn begin_issue_intervention(
        &self,
        workspace: &str,
        request: &crate::domain::issue_automation::intervention::IssueInterventionRequest,
    ) -> Result<crate::domain::issue_automation::intervention::StoredIssueIntervention> {
        intervention::begin(self, workspace, request)
    }
    fn finish_issue_intervention(
        &self,
        workspace: &str,
        receipt: &crate::domain::issue_automation::intervention::IssueInterventionReceipt,
    ) -> Result<()> {
        intervention::finish(self, workspace, receipt)
    }
    fn pending_issue_interventions(
        &self,
        workspace: &str,
    ) -> Result<Vec<crate::domain::issue_automation::intervention::StoredIssueIntervention>> {
        intervention::pending(self, workspace)
    }

    fn claim_issue_execution_answer(
        &self,
        workspace: &str,
        intent: &crate::domain::issue_automation::execution_answer::IssueExecutionAnswerIntent,
    ) -> Result<crate::domain::issue_automation::execution_answer::IssueExecutionAnswerClaim> {
        execution_answers::claim(self, workspace, intent)
    }

    fn pending_issue_execution_answers(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Vec<crate::domain::issue_automation::execution_answer::IssueExecutionAnswerRecord>>
    {
        execution_answers::pending(self, workspace, work_order_id)
    }

    fn save_issue_execution_answer(
        &self,
        workspace: &str,
        command_id: &str,
        expected_revision: u64,
        status: &crate::domain::issue_automation::execution_answer::IssueExecutionAnswerStatus,
    ) -> Result<Option<crate::domain::issue_automation::execution_answer::IssueExecutionAnswerRecord>>
    {
        execution_answers::save(self, workspace, command_id, expected_revision, status)
    }

    fn get_issue_execution(
        &self,
        workspace: &str,
        issue: &IssueReference,
    ) -> Result<Option<crate::domain::issue_automation::execution::IssueExecutionRecord>> {
        execution::get(self, workspace, issue)
    }

    fn apply_issue_execution(
        &self,
        workspace: &str,
        work: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint,
        expected_progress_revision: u64,
        observation: &crate::domain::issue_automation::execution::IssueWorkExecutionObservation,
        max_attempts: Option<u32>,
    ) -> Result<
        crate::application::port::outbound::issue_automation_store_port::IssueExecutionSaveResult,
    > {
        execution::apply(
            self,
            workspace,
            work,
            expected_progress_revision,
            observation,
            max_attempts,
        )
    }

    fn get_issue_work(
        &self,
        workspace: &str,
        issue: &IssueReference,
    ) -> Result<Option<crate::domain::issue_automation::coordinator::IssueWorkCheckpoint>> {
        coordinator::get(self, workspace, issue)
    }
    fn save_issue_work(
        &self,
        workspace: &str,
        expected: IssueTriageFence,
        assessment: &IssueTriageAssessment,
        candidate: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint,
    ) -> Result<crate::application::port::outbound::issue_automation_store_port::IssueWorkSaveResult>
    {
        coordinator::save(self, workspace, expected, assessment, candidate)
    }
    fn apply_issue_resync(
        &self,
        workspace: &str,
        expected: IssueCollectionFence,
        issue: &IssueReference,
        read: &IssueSourceRead,
    ) -> Result<IssueCollectionApplyResult> {
        collection::apply_one(self, workspace, expected, issue, read)
    }

    fn insert_binding(
        &self,
        workspace_dir: &str,
        binding: &IssueAutomationBinding,
    ) -> Result<IssueAutomationBindingInsertResult> {
        validate_binding_scope(workspace_dir, binding)?;
        let record = binding.to_record();
        self.with_connection(workspace_dir, |connection, guard| {
            transaction(connection, guard, |transaction| {
                if let Some(existing) = read_binding(transaction, workspace_dir, binding.id())? {
                    existing.binding.validate_replacement(binding)?;
                    return Ok(IssueAutomationBindingInsertResult {
                        binding: existing,
                        inserted: false,
                    });
                }
                transaction.execute(
                    "INSERT INTO issue_automation_bindings
                     (binding_id, provider, server_id, repository_id, workspace_dir, autonomy, revision)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                    params![
                        record.id.as_str(),
                        enum_name(&record.provider)?,
                        record.server_id,
                        record.repository_id,
                        record.project_scope.workspace_dir,
                        enum_name(&record.autonomy)?,
                    ],
                )?;
                triage::bump_catalog(transaction, workspace_dir)?;
                Ok(IssueAutomationBindingInsertResult {
                    binding: StoredIssueAutomationBinding {
                        binding: binding.clone(),
                        revision: 1,
                    },
                    inserted: true,
                })
            })
        })
    }

    fn get_binding(
        &self,
        workspace_dir: &str,
        binding_id: &IssueAutomationBindingId,
    ) -> Result<Option<StoredIssueAutomationBinding>> {
        self.with_connection(workspace_dir, |connection, _guard| {
            read_binding(connection, workspace_dir, binding_id)
        })
    }

    fn list_bindings(&self, workspace_dir: &str) -> Result<Vec<StoredIssueAutomationBinding>> {
        self.with_connection(workspace_dir, |connection, _guard| {
            let mut statement = connection.prepare(&format!(
                "SELECT {BINDING_COLUMNS} FROM issue_automation_bindings
                 WHERE workspace_dir = ?1 ORDER BY binding_id"
            ))?;
            let mut rows = statement.query([workspace_dir])?;
            let mut bindings = Vec::new();
            while let Some(row) = rows.next()? {
                let binding = hydrate_binding(row)?;
                validate_binding_scope(workspace_dir, &binding.binding)?;
                bindings.push(binding);
            }
            Ok(bindings)
        })
    }

    fn save_binding(
        &self,
        workspace_dir: &str,
        expected_revision: u64,
        binding: &IssueAutomationBinding,
    ) -> Result<IssueAutomationBindingSaveResult> {
        validate_expected_revision(expected_revision)?;
        validate_binding_scope(workspace_dir, binding)?;
        self.with_connection(workspace_dir, |connection, guard| {
            transaction(connection, guard, |transaction| {
                let Some(current) = read_binding(transaction, workspace_dir, binding.id())? else {
                    return Ok(IssueAutomationBindingSaveResult::Conflict {
                        actual_revision: None,
                    });
                };
                current.binding.validate_replacement(binding)?;
                let identical = current.binding == *binding;
                if current.revision != expected_revision {
                    if identical && expected_revision.checked_add(1) == Some(current.revision) {
                        return Ok(IssueAutomationBindingSaveResult::Saved {
                            record: Box::new(current),
                            changed: false,
                        });
                    }
                    return Ok(IssueAutomationBindingSaveResult::Conflict {
                        actual_revision: Some(current.revision),
                    });
                }
                if identical {
                    return Ok(IssueAutomationBindingSaveResult::Saved {
                        record: Box::new(current),
                        changed: false,
                    });
                }
                let revision = next_revision(current.revision)?;
                let changed = transaction.execute(
                    "UPDATE issue_automation_bindings SET autonomy = ?1, revision = ?2
                     WHERE binding_id = ?3 AND workspace_dir = ?4 AND revision = ?5",
                    params![
                        enum_name(&binding.autonomy())?,
                        sql_revision(revision)?,
                        binding.id().as_str(),
                        workspace_dir,
                        sql_revision(expected_revision)?,
                    ],
                )?;
                if changed != 1 {
                    bail!("the issue automation binding changed during its transaction")
                }
                triage::bump_catalog(transaction, workspace_dir)?;
                Ok(IssueAutomationBindingSaveResult::Saved {
                    record: Box::new(StoredIssueAutomationBinding {
                        binding: binding.clone(),
                        revision,
                    }),
                    changed: true,
                })
            })
        })
    }

    fn insert_issue(
        &self,
        workspace_dir: &str,
        issue: &TrackedIssue,
    ) -> Result<IssueAutomationIssueInsertResult> {
        let record = issue.to_record();
        validate_reference_scope(workspace_dir, &record.reference)?;
        if record.revision != 0
            || record.state != IssueAutomationState::Discovered
            || record.decision_resume.is_some()
        {
            bail!("only a new issue discovery can be inserted")
        }
        self.with_connection(workspace_dir, |connection, guard| {
            transaction(connection, guard, |transaction| {
                if let Some(existing) = read_issue(transaction, workspace_dir, &record.reference)? {
                    return Ok(IssueAutomationIssueInsertResult {
                        issue: existing,
                        inserted: false,
                    });
                }
                collection::validate_new_workflow_binding(transaction, workspace_dir, &record)?;
                validate_issue_binding(transaction, workspace_dir, &record)?;
                let mut inserted = record.clone();
                inserted.revision = 1;
                let inserted_issue = TrackedIssue::hydrate(inserted.clone())?;
                insert_issue_row(transaction, &inserted)?;
                triage::bump_catalog(transaction, workspace_dir)?;
                Ok(IssueAutomationIssueInsertResult {
                    issue: inserted_issue,
                    inserted: true,
                })
            })
        })
    }

    fn get_issue(
        &self,
        workspace_dir: &str,
        reference: &IssueReference,
    ) -> Result<Option<TrackedIssue>> {
        validate_reference_scope(workspace_dir, reference)?;
        self.with_connection(workspace_dir, |connection, _guard| {
            read_issue(connection, workspace_dir, reference)
        })
    }

    fn list_issues(
        &self,
        workspace_dir: &str,
        binding_id: Option<&IssueAutomationBindingId>,
    ) -> Result<Vec<TrackedIssue>> {
        self.with_connection(workspace_dir, |connection, _guard| {
            let mut statement = connection.prepare(&format!(
                "SELECT {ISSUE_COLUMNS} FROM tracked_issues
                 WHERE workspace_dir = ?1 AND (?2 IS NULL OR binding_id = ?2)
                 ORDER BY provider, server_id, repository_id, issue_id, workspace_dir"
            ))?;
            let mut rows = statement.query(params![
                workspace_dir,
                binding_id.map(IssueAutomationBindingId::as_str),
            ])?;
            let mut issues = Vec::new();
            while let Some(row) = rows.next()? {
                let issue = hydrate_issue(row)?;
                let record = issue.to_record();
                validate_reference_scope(workspace_dir, &record.reference)?;
                validate_issue_binding(connection, workspace_dir, &record)?;
                issues.push(issue);
            }
            Ok(issues)
        })
    }

    fn save_issue(
        &self,
        workspace_dir: &str,
        expected_revision: u64,
        issue: &TrackedIssue,
    ) -> Result<IssueAutomationIssueSaveResult> {
        validate_expected_revision(expected_revision)?;
        let candidate = issue.to_record();
        validate_reference_scope(workspace_dir, &candidate.reference)?;
        if candidate.revision != expected_revision {
            bail!("the issue candidate revision does not match its expected revision")
        }
        self.with_connection(workspace_dir, |connection, guard| {
            transaction(connection, guard, |transaction| {
                let Some(current) = read_issue(transaction, workspace_dir, &candidate.reference)?
                else {
                    return Ok(IssueAutomationIssueSaveResult::Conflict {
                        actual_revision: None,
                    });
                };
                let current_revision = current.to_record().revision;
                let identical = current.same_content(issue);
                if current_revision != expected_revision {
                    if identical && expected_revision.checked_add(1) == Some(current_revision) {
                        return Ok(IssueAutomationIssueSaveResult::Saved {
                            record: Box::new(current),
                            changed: false,
                        });
                    }
                    return Ok(IssueAutomationIssueSaveResult::Conflict {
                        actual_revision: Some(current_revision),
                    });
                }
                current.validate_replacement(issue)?;
                validate_issue_binding(transaction, workspace_dir, &candidate)?;
                if identical {
                    return Ok(IssueAutomationIssueSaveResult::Saved {
                        record: Box::new(current),
                        changed: false,
                    });
                }
                let mut saved = candidate.clone();
                saved.revision = next_revision(current_revision)?;
                let saved_issue = TrackedIssue::hydrate(saved.clone())?;
                update_issue_row(transaction, expected_revision, &saved)?;
                triage::bump_catalog(transaction, workspace_dir)?;
                Ok(IssueAutomationIssueSaveResult::Saved {
                    record: Box::new(saved_issue),
                    changed: true,
                })
            })
        })
    }

    fn get_collection(&self, workspace_dir: &str) -> Result<Option<StoredIssueCollection>> {
        collection::get(self, workspace_dir)
    }

    fn get_collection_projection(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<IssueCollectionProjection>> {
        collection::projection(self, workspace_dir)
    }

    fn ensure_collection(&self, workspace_dir: &str) -> Result<StoredIssueCollection> {
        collection::ensure(self, workspace_dir)
    }

    fn set_collection_mode(
        &self,
        workspace_dir: &str,
        expected_epoch: Option<u64>,
        mode: IssueCollectionMode,
    ) -> Result<IssueCollectionModeSaveResult> {
        collection::set_mode(self, workspace_dir, expected_epoch, mode)
    }

    fn set_collection_mode_exact(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        from: IssueCollectionMode,
        mode: IssueCollectionMode,
    ) -> Result<IssueCollectionModeSaveResult> {
        collection::set_mode_exact(self, workspace_dir, expected, from, mode)
    }

    fn apply_discovery_page(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        request: &IssueDiscoveryRequest,
        page: &IssueDiscoveryPage,
    ) -> Result<IssueCollectionApplyResult> {
        collection::apply_discovery(self, workspace_dir, expected, request, page)
    }

    fn reset_discovery_cursor(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
    ) -> Result<IssueCollectionApplyResult> {
        collection::reset_discovery(self, workspace_dir, expected)
    }

    fn read_resync_batch(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        limit: u16,
    ) -> Result<Option<IssueResyncBatch>> {
        collection::read_resync(self, workspace_dir, expected, limit)
    }

    fn apply_resync_batch(
        &self,
        workspace_dir: &str,
        batch: &IssueResyncBatch,
        reads: &[IssueSourceRead],
    ) -> Result<IssueCollectionApplyResult> {
        collection::apply_resync(self, workspace_dir, batch, reads)
    }

    fn record_collection_attempt(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        at_unix_ms: u64,
        failure: Option<IssueCollectionFailure>,
    ) -> Result<IssueCollectionApplyResult> {
        collection::record_attempt(self, workspace_dir, expected, at_unix_ms, failure)
    }

    fn query_collection_issues(
        &self,
        workspace_dir: &str,
        query: &IssueCollectionQuery,
    ) -> Result<IssueCollectionPage> {
        collection::query(self, workspace_dir, query)
    }

    fn scan_collection_issues(
        &self,
        workspace_dir: &str,
        binding_id: Option<&IssueAutomationBindingId>,
        visitor: &mut dyn FnMut(IssueCollectionItem) -> Result<()>,
    ) -> Result<u64> {
        triage::scan(self, workspace_dir, binding_id, visitor)
    }

    fn read_triage_batch(
        &self,
        workspace_dir: &str,
        after: Option<&IssueReference>,
        limit: u16,
    ) -> Result<IssueTriageBatch> {
        triage::batch(self, workspace_dir, after, limit)
    }

    fn save_triage(
        &self,
        workspace_dir: &str,
        expected: IssueTriageFence,
        assessment: &IssueTriageAssessment,
    ) -> Result<IssueTriageSaveResult> {
        triage::save(self, workspace_dir, expected, assessment)
    }
    fn get_issue_decision(
        &self,
        workspace_dir: &str,
        id: &IssueDecisionId,
    ) -> Result<Option<StoredIssueDecision>> {
        decision::get(self, workspace_dir, id)
    }

    fn get_issue_decision_commit(
        &self,
        workspace_dir: &str,
        id: &IssueDecisionId,
    ) -> Result<Option<IssueDecisionCommit>> {
        decision::get_commit(self, workspace_dir, id)
    }

    fn ensure_issue_decision(
        &self,
        workspace_dir: &str,
        expected: IssueTriageFence,
        current: &IssueTriageAssessment,
        request: &IssueDecisionRequest,
    ) -> Result<IssueDecisionSaveResult> {
        decision::ensure(self, workspace_dir, expected, current, request)
    }

    fn save_issue_decision(
        &self,
        workspace_dir: &str,
        expected: IssueDecisionFence,
        current: &IssueTriageAssessment,
        request: &IssueDecisionRequest,
    ) -> Result<IssueDecisionSaveResult> {
        decision::save(self, workspace_dir, expected, current, request)
    }
}

fn transaction<T>(
    connection: &mut Connection,
    guard: &private_store::PrivateStoreGuard,
    work: impl FnOnce(&Transaction<'_>) -> Result<T>,
) -> Result<T> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    guard.verify_identity()?;
    let result = work(&transaction)?;
    guard.verify_identity()?;
    transaction.commit()?;
    guard.verify_identity()?;
    Ok(result)
}

fn ensure_schema(
    connection: &mut Connection,
    guard: &private_store::PrivateStoreGuard,
) -> Result<()> {
    if schema_identity(connection)? == (APPLICATION_ID, SCHEMA_VERSION) {
        return validate_schema(connection);
    }
    transaction(connection, guard, |transaction| {
        match schema_identity(transaction)? {
            (APPLICATION_ID, SCHEMA_VERSION) => validate_schema(transaction),
            (APPLICATION_ID, version @ 1..=7) => {
                // Exact prior schemas are validated before the additive migration.
                // Existing bindings, observations, workflow rows and revisions remain untouched.
                validate_schema_version(transaction, version)?;
                if version == 1 {
                    transaction.execute_batch(collection::CONTROL_SCHEMA)?;
                    transaction.execute_batch(collection::OBSERVATIONS_SCHEMA)?;
                }
                if version < 3 {
                    transaction.execute_batch(triage::CATALOG_SCHEMA)?;
                    transaction.execute_batch(triage::ASSESSMENTS_SCHEMA)?;
                    triage::seed_catalog(transaction)?;
                }
                if version < 4 {
                    transaction.execute_batch(decision::SCHEMA)?;
                }
                if version < 5 {
                    transaction.execute_batch(coordinator::SCHEMA)?;
                    transaction.execute_batch(events::SCHEMA)?;
                }
                if version < 6 {
                    transaction.execute_batch(intervention::COMMAND_SCHEMA)?;
                    transaction.execute_batch(intervention::TRACKING_SCHEMA)?;
                }
                if version < 7 {
                    transaction.execute_batch(autonomy::SCHEMA)?;
                    transaction.execute_batch(autonomy::COMMAND_SCHEMA)?;
                    transaction.execute_batch(autonomy::FENCE_SCHEMA)?;
                }
                transaction.execute_batch(execution::SCHEMA)?;
                transaction.execute_batch(execution::RECEIPT_SCHEMA)?;
                execution_answers::create_schema(transaction)?;
                transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                validate_schema(transaction)
            }
            (0, 0) => {
                let count: i64 = transaction.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |row| row.get(0),
                )?;
                if count != 0 {
                    bail!("the issue automation database has an unrecognized schema")
                }
                transaction.execute_batch(BINDINGS_SCHEMA)?;
                transaction.execute_batch(ISSUES_SCHEMA)?;
                transaction.execute_batch(collection::CONTROL_SCHEMA)?;
                transaction.execute_batch(collection::OBSERVATIONS_SCHEMA)?;
                transaction.execute_batch(triage::CATALOG_SCHEMA)?;
                transaction.execute_batch(triage::ASSESSMENTS_SCHEMA)?;
                transaction.execute_batch(decision::SCHEMA)?;
                transaction.execute_batch(coordinator::SCHEMA)?;
                transaction.execute_batch(events::SCHEMA)?;
                transaction.execute_batch(intervention::COMMAND_SCHEMA)?;
                transaction.execute_batch(intervention::TRACKING_SCHEMA)?;
                transaction.execute_batch(autonomy::SCHEMA)?;
                transaction.execute_batch(autonomy::COMMAND_SCHEMA)?;
                transaction.execute_batch(autonomy::FENCE_SCHEMA)?;
                transaction.execute_batch(execution::SCHEMA)?;
                transaction.execute_batch(execution::RECEIPT_SCHEMA)?;
                execution_answers::create_schema(transaction)?;
                transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
                transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                validate_schema(transaction)
            }
            _ => bail!("the issue automation database identity or schema version is unsupported"),
        }
    })
}

fn schema_identity(connection: &Connection) -> Result<(i64, i64)> {
    Ok((
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?,
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?,
    ))
}

fn validate_schema(connection: &Connection) -> Result<()> {
    validate_schema_version(connection, SCHEMA_VERSION)
}

fn validate_schema_version(connection: &Connection, version: i64) -> Result<()> {
    if !matches!(version, 1..=SCHEMA_VERSION) {
        bail!("the issue automation schema version is unsupported")
    }
    let mut statement = connection.prepare(
        "SELECT name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let mut rows = statement.query([])?;
    let mut expected = vec![
        (BINDINGS_TABLE, BINDINGS_SCHEMA),
        (ISSUES_TABLE, ISSUES_SCHEMA),
    ];
    if version >= 2 {
        expected.push((collection::CONTROL_TABLE, collection::CONTROL_SCHEMA));
        expected.push((
            collection::OBSERVATIONS_TABLE,
            collection::OBSERVATIONS_SCHEMA,
        ));
    }
    if version >= 3 {
        expected.push((triage::CATALOG_TABLE, triage::CATALOG_SCHEMA));
        expected.push((triage::ASSESSMENTS_TABLE, triage::ASSESSMENTS_SCHEMA));
    }
    if version >= 4 {
        expected.push((decision::TABLE, decision::SCHEMA));
    }
    if version >= 5 {
        expected.push((coordinator::TABLE, coordinator::SCHEMA));
        expected.push((events::TABLE, events::SCHEMA));
    }
    if version >= 6 {
        expected.push((intervention::COMMAND_TABLE, intervention::COMMAND_SCHEMA));
        expected.push((intervention::TRACKING_TABLE, intervention::TRACKING_SCHEMA));
    }
    if version >= 7 {
        expected.push((autonomy::TABLE, autonomy::SCHEMA));
        expected.push((autonomy::COMMAND_TABLE, autonomy::COMMAND_SCHEMA));
        expected.push((autonomy::FENCE_TABLE, autonomy::FENCE_SCHEMA));
    }
    if version >= 8 {
        expected.push((execution::TABLE, execution::SCHEMA));
        expected.push((execution::RECEIPT_TABLE, execution::RECEIPT_SCHEMA));
        expected.push((execution_answers::TABLE, execution_answers::SCHEMA));
        expected.extend(execution_answers::INDEXES);
    }
    expected.sort_by_key(|(name, _)| *name);
    for (name, schema) in expected {
        let Some(row) = rows.next()? else {
            bail!("the issue automation database is missing a required table")
        };
        let stored_name: String = row.get(0)?;
        let stored_schema: String = row.get(1)?;
        if stored_name != name || normalized_schema(&stored_schema) != normalized_schema(schema) {
            bail!("the issue automation database schema does not match its version")
        }
    }
    if rows.next()?.is_some() {
        bail!("the issue automation database has unexpected schema objects")
    }
    Ok(())
}

fn normalized_schema(schema: &str) -> String {
    schema
        .trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_workspace(workspace_dir: &str) -> Result<()> {
    let path = Path::new(workspace_dir);
    if workspace_dir.is_empty()
        || !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        bail!("the issue automation workspace must be an absolute canonical scope")
    }
    Ok(())
}

fn validate_reference_scope(workspace_dir: &str, reference: &IssueReference) -> Result<()> {
    validate_workspace(workspace_dir)?;
    if reference.project_scope.workspace_dir != workspace_dir {
        bail!("the issue automation reference belongs to a different project scope")
    }
    Ok(())
}

fn validate_binding_scope(workspace_dir: &str, binding: &IssueAutomationBinding) -> Result<()> {
    validate_workspace(workspace_dir)?;
    if binding.project_scope().workspace_dir != workspace_dir {
        bail!("the issue automation binding belongs to a different project scope")
    }
    Ok(())
}

fn sql_revision(revision: u64) -> Result<i64> {
    i64::try_from(revision).context("the issue automation revision exceeds SQLite's range")
}

fn validate_expected_revision(revision: u64) -> Result<()> {
    if revision == 0 {
        bail!("an issue automation save requires a persisted base revision")
    }
    sql_revision(revision)?;
    Ok(())
}

fn next_revision(revision: u64) -> Result<u64> {
    let next = revision
        .checked_add(1)
        .context("the issue automation revision overflowed")?;
    sql_revision(next)?;
    Ok(next)
}

fn read_revision(row: &rusqlite::Row<'_>, index: usize) -> Result<u64> {
    let revision: i64 = row.get(index)?;
    if revision < 1 {
        bail!("the stored issue automation revision is invalid")
    }
    Ok(revision as u64)
}

fn read_flag(row: &rusqlite::Row<'_>, index: usize) -> Result<bool> {
    match row.get::<_, i64>(index)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => bail!("the stored issue automation flag is invalid"),
    }
}

fn enum_name<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(name) => Ok(name),
        _ => bail!("the issue automation enum does not have a string representation"),
    }
}

fn decode_enum<T: DeserializeOwned>(value: String) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(value))
        .context("the issue automation database contains an invalid enum")
}

fn hydrate_binding(row: &rusqlite::Row<'_>) -> Result<StoredIssueAutomationBinding> {
    let binding = IssueAutomationBinding::hydrate(PersistedIssueAutomationBinding {
        id: IssueAutomationBindingId::new(row.get::<_, String>(0)?)?,
        provider: decode_enum(row.get(1)?)?,
        server_id: row.get(2)?,
        repository_id: row.get(3)?,
        project_scope: IssueProjectScope {
            workspace_dir: row.get(4)?,
        },
        autonomy: decode_enum::<IssueAutomationAutonomy>(row.get(5)?)?,
    })?;
    Ok(StoredIssueAutomationBinding {
        binding,
        revision: read_revision(row, 6)?,
    })
}

fn read_binding(
    connection: &Connection,
    workspace_dir: &str,
    binding_id: &IssueAutomationBindingId,
) -> Result<Option<StoredIssueAutomationBinding>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {BINDING_COLUMNS} FROM issue_automation_bindings WHERE binding_id = ?1"
    ))?;
    let mut rows = statement.query([binding_id.as_str()])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let binding = hydrate_binding(row)?;
    // An ID in the selected database must not silently become a missing row under another scope.
    validate_binding_scope(workspace_dir, &binding.binding)?;
    Ok(Some(binding))
}

fn hydrate_issue(row: &rusqlite::Row<'_>) -> Result<TrackedIssue> {
    let version: i64 = row.get(6)?;
    let version = u32::try_from(version).context("the stored source hash version is invalid")?;
    let resume: Option<String> = row.get(16)?;
    TrackedIssue::hydrate(PersistedTrackedIssue {
        reference: IssueReference {
            provider: decode_enum(row.get(0)?)?,
            server_id: row.get(1)?,
            repository_id: row.get(2)?,
            issue_id: row.get(3)?,
            project_scope: IssueProjectScope {
                workspace_dir: row.get(4)?,
            },
        },
        binding_id: IssueAutomationBindingId::new(row.get::<_, String>(5)?)?,
        snapshot: IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash {
                    version,
                    sha256: row.get(7)?,
                },
                provider_updated_at: row.get(10)?,
            },
            shared_copy: IssueSharedCopy {
                title: row.get(8)?,
                body: row.get(9)?,
            },
            policy_version: row.get(11)?,
        },
        state: decode_enum(row.get(12)?)?,
        flags: IssueAutomationFlags {
            held: read_flag(row, 13)?,
            stale: read_flag(row, 14)?,
            unknown: read_flag(row, 15)?,
        },
        decision_resume: resume.map(decode_enum).transpose()?,
        revision: read_revision(row, 17)?,
    })
    .map_err(Into::into)
}

fn read_issue(
    connection: &Connection,
    workspace_dir: &str,
    reference: &IssueReference,
) -> Result<Option<TrackedIssue>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ISSUE_COLUMNS} FROM tracked_issues
         WHERE provider = ?1 AND server_id = ?2 AND repository_id = ?3
           AND issue_id = ?4 AND workspace_dir = ?5"
    ))?;
    let mut rows = statement.query(params![
        enum_name(&reference.provider)?,
        reference.server_id,
        reference.repository_id,
        reference.issue_id,
        workspace_dir,
    ])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let issue = hydrate_issue(row)?;
    let record = issue.to_record();
    validate_reference_scope(workspace_dir, &record.reference)?;
    if record.reference != *reference {
        bail!("the issue automation row does not match its requested reference")
    }
    validate_issue_binding(connection, workspace_dir, &record)?;
    Ok(Some(issue))
}

fn validate_issue_binding(
    connection: &Connection,
    workspace_dir: &str,
    issue: &PersistedTrackedIssue,
) -> Result<()> {
    let Some(stored) = read_binding(connection, workspace_dir, &issue.binding_id)? else {
        bail!("the tracked issue has no stored automation binding")
    };
    let binding = stored.binding;
    if binding.provider() != issue.reference.provider
        || binding.server_id() != issue.reference.server_id
        || binding.repository_id() != issue.reference.repository_id
        || binding.project_scope() != &issue.reference.project_scope
    {
        bail!("the tracked issue does not belong to its automation binding")
    }
    Ok(())
}

fn insert_issue_row(transaction: &Transaction<'_>, issue: &PersistedTrackedIssue) -> Result<()> {
    transaction.execute(
        "INSERT INTO tracked_issues (
             provider, server_id, repository_id, issue_id, workspace_dir, binding_id,
             source_hash_version, source_hash_sha256, shared_title, shared_body,
             provider_updated_at, policy_version, state, held, stale, unknown,
             decision_resume, revision
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
         )",
        rusqlite::params_from_iter(issue_values(issue)?),
    )?;
    Ok(())
}

fn update_issue_row(
    transaction: &Transaction<'_>,
    expected_revision: u64,
    issue: &PersistedTrackedIssue,
) -> Result<()> {
    let mut values = issue_values(issue)?;
    values.push(rusqlite::types::Value::Integer(sql_revision(
        expected_revision,
    )?));
    let changed = transaction.execute(
        "UPDATE tracked_issues SET
             binding_id = ?6, source_hash_version = ?7, source_hash_sha256 = ?8,
             shared_title = ?9, shared_body = ?10, provider_updated_at = ?11,
             policy_version = ?12, state = ?13, held = ?14, stale = ?15,
             unknown = ?16, decision_resume = ?17, revision = ?18
         WHERE provider = ?1 AND server_id = ?2 AND repository_id = ?3
           AND issue_id = ?4 AND workspace_dir = ?5 AND revision = ?19",
        rusqlite::params_from_iter(values),
    )?;
    if changed != 1 {
        bail!("the tracked issue changed during its transaction")
    }
    Ok(())
}

fn issue_values(issue: &PersistedTrackedIssue) -> Result<Vec<rusqlite::types::Value>> {
    use rusqlite::types::Value;
    let reference = &issue.reference;
    let snapshot = &issue.snapshot;
    Ok(vec![
        Value::Text(enum_name(&reference.provider)?),
        Value::Text(reference.server_id.clone()),
        Value::Text(reference.repository_id.clone()),
        Value::Text(reference.issue_id.clone()),
        Value::Text(reference.project_scope.workspace_dir.clone()),
        Value::Text(issue.binding_id.as_str().to_string()),
        Value::Integer(i64::from(snapshot.basis.field_hash.version)),
        Value::Text(snapshot.basis.field_hash.sha256.clone()),
        Value::Text(snapshot.shared_copy.title.clone()),
        Value::Text(snapshot.shared_copy.body.clone()),
        Value::Text(snapshot.basis.provider_updated_at.clone()),
        Value::Text(snapshot.policy_version.clone()),
        Value::Text(enum_name(&issue.state)?),
        Value::Integer(i64::from(issue.flags.held)),
        Value::Integer(i64::from(issue.flags.stale)),
        Value::Integer(i64::from(issue.flags.unknown)),
        issue
            .decision_resume
            .as_ref()
            .map(enum_name)
            .transpose()?
            .map(Value::Text)
            .unwrap_or(Value::Null),
        Value::Integer(sql_revision(issue.revision)?),
    ])
}
