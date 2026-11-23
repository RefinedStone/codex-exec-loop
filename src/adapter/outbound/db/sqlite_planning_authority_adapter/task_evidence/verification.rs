//! Host claims share evidence quotas and outbox, but never masquerade as reported attempts.
use super::*;
use crate::application::port::outbound::task_verification_repository_port::*;
use crate::domain::planning::TaskAcceptanceChannel;
use crate::domain::planning::verification::*;

mod claim;
mod schema;
mod terminal;

pub(super) fn ensure(connection: &mut Connection) -> Result<()> {
    schema::ensure(connection)
}

struct Run {
    stored: StoredTaskVerification,
    owner_token: String,
    run_token: String,
    claim_fingerprint: String,
    finish_fingerprint: Option<String>,
}

struct RunRow {
    owner_token: String,
    run_token: String,
    claim_fingerprint: String,
    finish_fingerprint: Option<String>,
    phase: String,
    lease: String,
    attempt: String,
    manifest: Option<String>,
    receipt: Option<String>,
    request_id: String,
    task_id: String,
    scope_id: String,
    criterion_id: String,
    ordinal: u32,
    finished_revision: Option<u64>,
}

fn require_owner(connection: &Connection, owner: &str) -> Result<()> {
    validate_evidence_token(owner)?;
    let current: Option<String> = connection.query_row(
        "SELECT owner_token FROM task_verification_state WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if current.as_deref() != Some(owner) {
        return Err(TaskEvidenceError::Conflict.into());
    }
    Ok(())
}

fn require_execution(run: &Run, execution: &TaskVerificationExecutionKey) -> Result<()> {
    validate_evidence_token(&execution.attempt_id)?;
    validate_evidence_token(&execution.owner_token)?;
    validate_evidence_token(&execution.run_token)?;
    if run.stored.attempt.attempt_id != execution.attempt_id
        || run.owner_token != execution.owner_token
        || run.run_token != execution.run_token
    {
        return Err(TaskEvidenceError::Conflict.into());
    }
    Ok(())
}

fn read_run(connection: &Connection, attempt_id: &str) -> Result<Option<Run>> {
    let row = connection
        .query_row(
            "SELECT owner_token,run_token,claim_fingerprint,finish_fingerprint,phase,
         lease_json,attempt_json,manifest_json,receipt_json,request_id,task_id,scope_id,
         criterion_id,ordinal,finished_revision FROM task_verification_runs WHERE attempt_id=?1",
            [attempt_id],
            |row| {
                Ok(RunRow {
                    owner_token: row.get(0)?,
                    run_token: row.get(1)?,
                    claim_fingerprint: row.get(2)?,
                    finish_fingerprint: row.get(3)?,
                    phase: row.get(4)?,
                    lease: row.get(5)?,
                    attempt: row.get(6)?,
                    manifest: row.get(7)?,
                    receipt: row.get(8)?,
                    request_id: row.get(9)?,
                    task_id: row.get(10)?,
                    scope_id: row.get(11)?,
                    criterion_id: row.get(12)?,
                    ordinal: row.get(13)?,
                    finished_revision: row.get(14)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else { return Ok(None) };
    validate_evidence_token(&row.owner_token)?;
    validate_evidence_token(&row.run_token)?;
    let lease: ParallelModeSlotLeaseSnapshot = serde_json::from_str(&row.lease)?;
    let attempt: TaskVerificationAttempt = serde_json::from_str(&row.attempt)?;
    attempt.validate()?;
    let manifest: Option<TaskVerificationExecutionManifest> = row
        .manifest
        .map(|json| serde_json::from_str(&json))
        .transpose()?;
    let receipt: Option<TaskVerificationReceipt> = row
        .receipt
        .map(|json| serde_json::from_str(&json))
        .transpose()?;
    let phase = match row.phase.as_str() {
        "pending" => TaskVerificationPhase::Pending,
        "running" => TaskVerificationPhase::Running,
        "finished" => TaskVerificationPhase::Finished,
        _ => return Err(TaskEvidenceError::Unavailable.into()),
    };
    if attempt.attempt_id != attempt_id
        || row.request_id != attempt.request_id
        || row.task_id != attempt.scope.task_id
        || row.scope_id != attempt.scope.scope_id
        || row.criterion_id != attempt.criterion_id
        || row.ordinal != attempt.attempt
        || row.claim_fingerprint
            != digest(&(
                &attempt.scope.task_id,
                &attempt.criterion_id,
                &attempt.scope.scope_id,
                attempt.scope.acceptance_revision,
            ))?
        || lease.task_id != attempt.scope.task_id
        || lease.slot_id != attempt.slot_id
        || lease.session_key() != attempt.session_key
        || lease.lease_generation.as_deref() != Some(attempt.lease_generation.as_str())
        || lease.state != ParallelModeSlotLeaseState::Running
    {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    if let Some(manifest) = &manifest {
        manifest.validate()?;
        if manifest.recipe != attempt.recipe
            || lease
                .delivery_target
                .as_ref()
                .map(|target| target.integration_base_commit_sha.as_str())
                != Some(manifest.base_commit_sha.as_str())
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
    }
    if let Some(receipt) = &receipt {
        receipt.validate()?;
        let fingerprint = digest(&(&receipt.result, receipt.artifact.as_ref()))?;
        if receipt.attempt != attempt
            || receipt.manifest != manifest
            || row.finished_revision != Some(receipt.evidence_revision)
            || row.finish_fingerprint.as_deref() != Some(fingerprint.as_str())
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
    }
    if (phase == TaskVerificationPhase::Pending && manifest.is_some())
        || (phase == TaskVerificationPhase::Running && manifest.is_none())
        || (phase == TaskVerificationPhase::Finished) != receipt.is_some()
        || receipt.is_some() != row.finish_fingerprint.is_some()
        || receipt.is_some() != row.finished_revision.is_some()
    {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(Some(Run {
        stored: StoredTaskVerification {
            attempt,
            lease,
            phase,
            manifest,
            receipt,
        },
        owner_token: row.owner_token,
        run_token: row.run_token,
        claim_fingerprint: row.claim_fingerprint,
        finish_fingerprint: row.finish_fingerprint,
    }))
}

pub(super) fn latest(
    connection: &Connection,
    task_id: &str,
    criterion: &str,
    scope: &str,
) -> Result<Option<StoredTaskVerification>> {
    let id: Option<String> = connection
        .query_row(
            "SELECT attempt_id FROM task_verification_runs WHERE task_id=?1 AND criterion_id=?2
         ORDER BY (scope_id=?3) DESC,rowid DESC LIMIT 1",
            params![task_id, criterion, scope],
            |row| row.get(0),
        )
        .optional()?;
    id.map(|id| {
        read_run(connection, &id)?
            .map(|run| run.stored)
            .ok_or_else(|| TaskEvidenceError::Unavailable.into())
    })
    .transpose()
}

pub(super) fn original_receipt(
    connection: &Connection,
    attempt_id: &str,
) -> Result<Option<TaskVerificationReceipt>> {
    Ok(read_run(connection, attempt_id)?.and_then(|run| run.stored.receipt))
}

pub(super) fn next_ordinal(connection: &Connection, scope: &str, criterion: &str) -> Result<u32> {
    let current: u32 = connection.query_row(
        "SELECT COALESCE(MAX(ordinal),0) FROM (
          SELECT ordinal FROM task_evidence_attempts WHERE scope_id=?1 AND criterion_id=?2
          UNION ALL SELECT ordinal FROM task_verification_runs WHERE scope_id=?1 AND criterion_id=?2)",
        params![scope,criterion], |row| row.get(0),
    )?;
    current
        .checked_add(1)
        .filter(|next| *next <= MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION)
        .ok_or_else(|| TaskEvidenceError::Limit.into())
}

impl TaskVerificationRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn claim_host_verification(
        &self,
        workspace: &str,
        commit: &TaskVerificationClaimCommit,
    ) -> Result<TaskVerificationClaimResult> {
        claim::claim(workspace, commit)
    }
    fn bind_host_verification(
        &self,
        workspace: &str,
        commit: &TaskVerificationBindCommit,
    ) -> Result<bool> {
        claim::bind(workspace, commit)
    }
    fn finish_host_verification(
        &self,
        workspace: &str,
        commit: &TaskVerificationFinishCommit,
    ) -> Result<TaskEvidenceWriteResult<TaskVerificationReceipt>> {
        terminal::finish(workspace, commit)
    }
    fn load_host_verification(
        &self,
        workspace: &str,
        attempt_id: &str,
    ) -> Result<Option<StoredTaskVerification>> {
        validate_evidence_token(attempt_id)?;
        Ok(read_run(&connection(workspace)?, attempt_id)?.map(|run| run.stored))
    }
    fn recover_incomplete_host_verifications(
        &self,
        workspace: &str,
        owner_token: &str,
        at: &str,
    ) -> Result<Vec<TaskVerificationReceipt>> {
        terminal::recover(workspace, owner_token, at)
    }
}
