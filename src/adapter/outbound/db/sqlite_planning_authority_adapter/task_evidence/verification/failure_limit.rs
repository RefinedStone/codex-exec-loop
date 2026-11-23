//! Automatic completion applies this policy explicitly; terminal facts and manual checks do not.
use super::*;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityMutationAudit,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PriorityQueueService, TaskActor, TaskMutationProvenance,
};

use super::super::super::{
    ADMIN_AUTHORITY_MUTATION_CLAIM_KIND, ADMIN_AUTHORITY_MUTATION_SCOPE_KEY,
    ADMIN_TASK_MUTATION_CLAIM_KIND, load_direction_catalog_from_connection,
    load_task_authority_snapshot_from_connection, runtime_projection,
};

const FAILURE_LIMIT: u32 = 3;

mod session;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Application {
    fingerprint: String,
    outcome: TaskVerificationFailureLimitOutcome,
    failure_count: u32,
    planning_revision: Option<i64>,
    at: String,
}

impl Application {
    fn validate(&self) -> Result<()> {
        validate_evidence_token(&self.fingerprint)?;
        validate_time(&self.at)?;
        if self.failure_count > MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION
            || self.planning_revision.is_some_and(|revision| revision < 0)
            || (self.outcome == TaskVerificationFailureLimitOutcome::AwaitingUser
                && (self.failure_count < FAILURE_LIMIT || self.planning_revision.is_none()))
            || (self.outcome != TaskVerificationFailureLimitOutcome::AwaitingUser
                && self.planning_revision.is_some())
            || (self.outcome == TaskVerificationFailureLimitOutcome::NotFailed
                && self.failure_count != 0)
            || (self.outcome == TaskVerificationFailureLimitOutcome::BelowLimit
                && !(1..FAILURE_LIMIT).contains(&self.failure_count))
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        Ok(())
    }

    fn result(&self, changed: bool) -> TaskVerificationFailureLimitResult {
        TaskVerificationFailureLimitResult {
            outcome: self.outcome,
            failure_count: self.failure_count,
            planning_revision: self.planning_revision,
            changed,
        }
    }
}

fn definitive_failure(receipt: &TaskVerificationReceipt) -> bool {
    // This fixed recipe does not use --exit-code. Git's CHECK_FAILED bit is 2; other
    // nonzero statuses (including fatal 128) do not establish a checker finding.
    receipt.validate().is_ok()
        && receipt.attempt.recipe == TaskVerificationRecipe::GitDiffCheckV1
        && receipt.artifact.is_some()
        && matches!(
            &receipt.result,
            TaskVerificationResult::Executed { observation }
                if observation.outcome == (TaskVerificationExecutionOutcome::Exited { exit_code: 2 })
                    && observation.stdout.complete && observation.stderr.complete
        )
}

fn failure_count(transaction: &Transaction<'_>, trigger: &TaskVerificationReceipt) -> Result<u32> {
    let mut statement = transaction.prepare(
        "SELECT attempt_id FROM task_verification_runs
         WHERE scope_id=?1 AND criterion_id=?2 AND phase='finished' AND finished_revision<=?3
         ORDER BY ordinal LIMIT ?4",
    )?;
    let ids = statement
        .query_map(
            params![
                trigger.attempt.scope.scope_id,
                trigger.attempt.criterion_id,
                trigger.evidence_revision as i64,
                i64::from(MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION) + 1,
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if ids.len() > MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION as usize {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    let mut failures = 0;
    for id in ids {
        let run = read_run(transaction, &id)?.ok_or(TaskEvidenceError::Unavailable)?;
        let receipt = run.stored.receipt.ok_or(TaskEvidenceError::Unavailable)?;
        if receipt.attempt.scope != trigger.attempt.scope
            || receipt.attempt.criterion_id != trigger.attempt.criterion_id
            || receipt.evidence_revision > trigger.evidence_revision
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        failures += u32::from(definitive_failure(&receipt));
    }
    if failures == 0 {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(failures)
}

fn authority_fenced(transaction: &Transaction<'_>, task_id: &str) -> Result<bool> {
    let retired: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM retired_planning_tasks WHERE task_id=?1)",
        [task_id],
        |row| row.get(0),
    )?;
    if retired {
        return Ok(true);
    }
    // Observe existing admin fences without consuming or changing their ownership.
    let mut statement = transaction.prepare(
        "SELECT claimed_at FROM runtime_claims
         WHERE (claim_kind=?1 AND scope_key=?2) OR (claim_kind=?3 AND scope_key=?4)",
    )?;
    let claims = statement
        .query_map(
            params![
                ADMIN_TASK_MUTATION_CLAIM_KIND,
                task_id,
                ADMIN_AUTHORITY_MUTATION_CLAIM_KIND,
                ADMIN_AUTHORITY_MUTATION_SCOPE_KEY,
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(claims
        .iter()
        .any(|claimed_at| !runtime_projection::claim_is_stale(claimed_at)))
}

fn currently_waived(
    transaction: &Transaction<'_>,
    attempt: &TaskVerificationAttempt,
) -> Result<bool> {
    let row: Option<(String, u64)> = transaction
        .query_row(
            "SELECT receipt_json,evidence_revision FROM task_criterion_waivers
             WHERE task_id=?1 AND scope_id=?2 AND criterion_id=?3
             ORDER BY evidence_revision DESC LIMIT 1",
            params![
                attempt.scope.task_id,
                attempt.scope.scope_id,
                attempt.criterion_id
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((json, revision)) = row else {
        return Ok(false);
    };
    let receipt: TaskCriterionWaiverReceipt = serde_json::from_str(&json)?;
    receipt.validate()?;
    if receipt.waiver.scope != attempt.scope
        || receipt.waiver.criterion_id != attempt.criterion_id
        || receipt.evidence_revision != revision
    {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(true)
}

fn current_task_for_policy(
    transaction: &Transaction<'_>,
    run: &Run,
) -> Result<Option<TaskDefinition>> {
    let attempt = &run.stored.attempt;
    if read_scope(transaction, &attempt.scope.task_id)?.as_ref() != Some(&attempt.scope)
        || authority_fenced(transaction, &attempt.scope.task_id)?
    {
        return Ok(None);
    }
    let task = require_current_scope(transaction, &attempt.scope, &attempt.criterion_id)?;
    if !task.acceptance.iter().any(|criterion| {
        criterion.id == attempt.criterion_id
            && criterion.essential
            && criterion.channel == TaskAcceptanceChannel::Command
            && criterion.verification_recipe == Some(attempt.recipe)
    }) || currently_waived(transaction, attempt)?
    {
        return Ok(None);
    }
    match claim::require_host_lease(transaction, &task, &run.stored.lease) {
        Ok(()) => Ok(Some(task)),
        Err(error)
            if error.downcast_ref::<TaskEvidenceError>()
                == Some(&TaskEvidenceError::LeaseChanged) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn pause_task(
    transaction: &Transaction<'_>,
    location: &PlanningAuthorityLocation,
    current: &TaskDefinition,
    at: &str,
) -> Result<i64> {
    let snapshot = load_task_authority_snapshot_from_connection(transaction)?
        .ok_or(TaskEvidenceError::Unavailable)?;
    let mut candidate = snapshot.task_authority.clone();
    let task = candidate
        .tasks
        .iter_mut()
        .find(|task| task.id == current.id)
        .ok_or(TaskEvidenceError::Unavailable)?;
    if task != current {
        return Err(TaskEvidenceError::Conflict.into());
    }
    task.status = TaskStatus::AwaitingUser;
    task.last_updated_by = TaskActor::System;
    task.updated_at = at.to_owned();
    task.source_turn_id = None;
    task.provenance = TaskMutationProvenance::default();
    runtime_projection::ensure_task_not_retired(
        transaction,
        &task.id,
        "verification failure limit",
    )?;
    let directions = load_direction_catalog_from_connection(transaction)?
        .ok_or(TaskEvidenceError::Unavailable)?;
    let projection = PriorityQueueService::new().build_projection(&directions, &candidate)?;
    SqlitePlanningAuthorityAdapter::write_task_authority_change_in_transaction(
        transaction,
        location,
        PlanningTaskAuthorityCommit {
            observed_planning_revision: Some(snapshot.planning_revision),
            task_authority: &candidate,
            queue_projection: &projection,
        },
        Some(&snapshot.task_authority),
        Some(PlanningTaskAuthorityMutationAudit {
            task_ids: std::slice::from_ref(&current.id),
            legacy_source_turn_id: None,
            provenance: &TaskMutationProvenance::default(),
        }),
    )
}

pub(super) fn apply(
    workspace: &str,
    commit: &TaskVerificationFailureLimitCommit,
) -> Result<TaskVerificationFailureLimitResult> {
    validate_evidence_token(&commit.attempt_id)?;
    validate_time(&commit.at)?;
    validate_captured_lease(&commit.expected_lease)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_owner(&transaction, &commit.owner_token)?;
    let run = read_run(&transaction, &commit.attempt_id)?.ok_or(TaskEvidenceError::Missing)?;
    require_captured_lease(&run.stored.lease, &commit.expected_lease)?;
    let receipt = run
        .stored
        .receipt
        .as_ref()
        .ok_or(TaskEvidenceError::Invalid)?;
    let fingerprint = digest(&(
        "host-failure-limit-v1",
        &commit.attempt_id,
        captured_lease_fingerprint(&commit.expected_lease)?,
    ))?;
    let prior: Option<String> = transaction.query_row(
        "SELECT failure_limit_application_json FROM task_verification_runs WHERE attempt_id=?1",
        [&commit.attempt_id],
        |row| row.get(0),
    )?;
    if let Some(json) = prior {
        let original: Application = serde_json::from_str(&json)?;
        original.validate()?;
        if original.fingerprint != fingerprint {
            return Err(TaskEvidenceError::Conflict.into());
        }
        transaction.commit()?;
        return Ok(original.result(false));
    }
    let mut application = Application {
        fingerprint,
        outcome: TaskVerificationFailureLimitOutcome::NotFailed,
        failure_count: 0,
        planning_revision: None,
        at: commit.at.clone(),
    };
    if definitive_failure(receipt) {
        application.failure_count = failure_count(&transaction, receipt)?;
        application.outcome = match current_task_for_policy(&transaction, &run)? {
            None => TaskVerificationFailureLimitOutcome::Superseded,
            Some(_) if application.failure_count < FAILURE_LIMIT => {
                TaskVerificationFailureLimitOutcome::BelowLimit
            }
            Some(task) => {
                // The waiting task must already have an answerable session when
                // this transaction commits, even if completion is then cancelled.
                session::preserve(&transaction, &run.stored.lease, &commit.at)?;
                application.planning_revision =
                    Some(pause_task(&transaction, &location, &task, &commit.at)?);
                TaskVerificationFailureLimitOutcome::AwaitingUser
            }
        };
    }
    application.validate()?;
    let json = serde_json::to_string(&application)?;
    let changed = transaction.execute(
        "UPDATE task_verification_runs SET failure_limit_application_json=?2
         WHERE attempt_id=?1 AND receipt_json IS NOT NULL AND failure_limit_application_json IS NULL",
        params![commit.attempt_id, json],
    )?;
    if changed != 1 {
        return Err(TaskEvidenceError::Conflict.into());
    }
    let paused = application.outcome == TaskVerificationFailureLimitOutcome::AwaitingUser;
    if paused {
        runtime_projection::append_runtime_event(
            &transaction,
            "task_verification_failure_limit_applied",
            "task_verification",
            &commit.attempt_id,
            "같은 기준의 확정 검사 실패가 세 번 누적되어 사용자 확인을 기다립니다.",
            &serde_json::to_string(&serde_json::json!({
                "task_id": receipt.attempt.scope.task_id,
                "scope_id": receipt.attempt.scope.scope_id,
                "criterion_id": receipt.attempt.criterion_id,
                "failure_count": application.failure_count,
                "planning_revision": application.planning_revision,
            }))?,
        )?;
    }
    transaction.commit()?;
    Ok(application.result(paused))
}
