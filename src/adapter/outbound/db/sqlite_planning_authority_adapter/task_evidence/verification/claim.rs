use super::*;

fn running_lease(
    connection: &Transaction<'_>,
    task: &TaskDefinition,
) -> Result<ParallelModeSlotLeaseSnapshot> {
    let mut statement = connection.prepare(
        "SELECT content FROM runtime_slot_leases WHERE json_extract(content,'$.task_id')=?1 LIMIT 2",
    )?;
    let rows = statement
        .query_map([&task.id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() != 1 {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    let lease: ParallelModeSlotLeaseSnapshot = serde_json::from_str(&rows[0])?;
    require_host_lease(connection, task, &lease)?;
    Ok(lease)
}

pub(super) fn require_host_lease(
    transaction: &Transaction<'_>,
    task: &TaskDefinition,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<()> {
    require_running_lease(transaction, task, lease)?;
    let base = &lease
        .delivery_target
        .as_ref()
        .ok_or(TaskEvidenceError::LeaseChanged)?
        .integration_base_commit_sha;
    TaskEvidenceTree {
        head_commit_sha: base.clone(),
        tree_sha: base.clone(),
        worktree_clean: true,
    }
    .validate()?;
    // The host recipe evaluates this lease's frozen full range. A Dispatch grant
    // for a different source cannot authorize claim or Pending -> Running binding.
    if matches!(
        super::super::super::work_orders::check_work_order_permit_in_transaction(
            transaction,
            &task.id,
            PlanningWorkPermitStage::Dispatch,
            Some(base),
        )?,
        PlanningWorkPermitDecision::Denied(_)
    ) {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    Ok(())
}

fn current_recipe(task: &TaskDefinition, criterion: &str) -> Result<TaskVerificationRecipe> {
    let criterion = task
        .acceptance
        .iter()
        .find(|value| value.id == criterion)
        .ok_or(TaskEvidenceError::Missing)?;
    if criterion.channel != TaskAcceptanceChannel::Command {
        return Err(TaskEvidenceError::Invalid.into());
    }
    criterion
        .verification_recipe
        .ok_or_else(|| TaskEvidenceError::Invalid.into())
}

pub(super) fn claim(
    workspace: &str,
    commit: &TaskVerificationClaimCommit,
) -> Result<TaskVerificationClaimResult> {
    validate_evidence_request_id(&commit.request_id)?;
    validate_evidence_identifier(&commit.task_id)?;
    validate_evidence_identifier(&commit.criterion_id)?;
    validate_evidence_token(&commit.scope_id)?;
    validate_evidence_token(&commit.owner_token)?;
    validate_time(&commit.at)?;
    if commit.expected_acceptance_revision == 0
        || commit.expected_acceptance_revision > MAX_TASK_EVIDENCE_REVISION
    {
        return Err(TaskEvidenceError::Invalid.into());
    }
    if let Some(expected) = &commit.expected_lease {
        validate_captured_lease(expected)?;
        if expected.task_id != commit.task_id {
            return Err(TaskEvidenceError::LeaseChanged.into());
        }
    }
    let fingerprint = claim_fingerprint(
        &commit.task_id,
        &commit.criterion_id,
        &commit.scope_id,
        commit.expected_acceptance_revision,
        commit.expected_lease.as_ref(),
    )?;
    let mut connection = connection(workspace)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_owner(&transaction, &commit.owner_token)?;
    let prior: Option<String> = transaction
        .query_row(
            "SELECT attempt_id FROM task_verification_runs WHERE request_id=?1",
            [&commit.request_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = prior {
        let run = read_run(&transaction, &id)?.ok_or(TaskEvidenceError::Unavailable)?;
        if run.claim_fingerprint != fingerprint {
            return Err(TaskEvidenceError::Conflict.into());
        }
        if let Some(expected) = &commit.expected_lease {
            // Replay returns the original fact, never a replacement lease or execution key.
            require_captured_lease(&run.stored.lease, expected)?;
        }
        transaction.commit()?;
        return Ok(TaskVerificationClaimResult {
            stored: run.stored,
            execution: None,
            changed: false,
        });
    }
    let scope =
        read_scope(&transaction, &commit.task_id)?.ok_or(TaskEvidenceError::ScopeChanged)?;
    if scope.scope_id != commit.scope_id
        || scope.acceptance_revision != commit.expected_acceptance_revision
    {
        return Err(TaskEvidenceError::ScopeChanged.into());
    }
    let task = require_current_scope(&transaction, &scope, &commit.criterion_id)?;
    let recipe = current_recipe(&task, &commit.criterion_id)?;
    let lease = running_lease(&transaction, &task)?;
    if let Some(expected) = &commit.expected_lease {
        require_captured_lease(&lease, expected)?;
    }
    let unfinished: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_verification_runs WHERE phase!='finished')",
        [],
        |row| row.get(0),
    )?;
    if unfinished {
        return Err(TaskEvidenceError::Limit.into());
    }
    let ordinal = next_ordinal(&transaction, &scope.scope_id, &commit.criterion_id)?;
    let attempt = TaskVerificationAttempt {
        attempt_id: random_token()?,
        request_id: commit.request_id.clone(),
        scope,
        criterion_id: commit.criterion_id.clone(),
        recipe,
        attempt: ordinal,
        slot_id: lease.slot_id.clone(),
        lease_generation: lease
            .lease_generation
            .clone()
            .ok_or(TaskEvidenceError::LeaseChanged)?,
        session_key: lease.session_key(),
        requested_at: commit.at.clone(),
    };
    attempt.validate()?;
    let execution = TaskVerificationExecutionKey {
        attempt_id: attempt.attempt_id.clone(),
        owner_token: commit.owner_token.clone(),
        run_token: random_token()?,
    };
    writes::reserve_entry(&transaction, true)?;
    transaction.execute(
        "INSERT INTO task_verification_runs(request_id,attempt_id,task_id,scope_id,criterion_id,ordinal,claim_fingerprint,owner_token,run_token,phase,lease_json,attempt_json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10,?11)",
        params![attempt.request_id,attempt.attempt_id,attempt.scope.task_id,attempt.scope.scope_id,
            attempt.criterion_id,ordinal,fingerprint,execution.owner_token,execution.run_token,
            serde_json::to_string(&lease)?,serde_json::to_string(&attempt)?],
    )?;
    bump_revision(&transaction)?;
    transaction.commit()?;
    Ok(TaskVerificationClaimResult {
        stored: StoredTaskVerification {
            attempt,
            lease,
            phase: TaskVerificationPhase::Pending,
            manifest: None,
            receipt: None,
        },
        execution: Some(execution),
        changed: true,
    })
}

pub(super) fn bind(workspace: &str, commit: &TaskVerificationBindCommit) -> Result<bool> {
    commit.manifest.validate()?;
    validate_time(&commit.at)?;
    let mut connection = connection(workspace)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run =
        read_run(&transaction, &commit.execution.attempt_id)?.ok_or(TaskEvidenceError::Missing)?;
    require_execution(&run, &commit.execution)?;
    require_owner(&transaction, &commit.execution.owner_token)?;
    if run.stored.phase != TaskVerificationPhase::Pending {
        if run.stored.manifest.as_ref() == Some(&commit.manifest) {
            transaction.commit()?;
            return Ok(false);
        }
        return Err(TaskEvidenceError::Conflict.into());
    }
    let task = require_current_scope(
        &transaction,
        &run.stored.attempt.scope,
        &run.stored.attempt.criterion_id,
    )?;
    require_host_lease(&transaction, &task, &run.stored.lease)?;
    if current_recipe(&task, &run.stored.attempt.criterion_id)? != run.stored.attempt.recipe
        || commit.manifest.recipe != run.stored.attempt.recipe
        || run
            .stored
            .lease
            .delivery_target
            .as_ref()
            .map(|target| target.integration_base_commit_sha.as_str())
            != Some(commit.manifest.base_commit_sha.as_str())
    {
        return Err(TaskEvidenceError::Stale.into());
    }
    transaction.execute(
        "UPDATE task_verification_runs SET manifest_json=?2,phase='running' WHERE attempt_id=?1 AND phase='pending'",
        params![commit.execution.attempt_id,serde_json::to_string(&commit.manifest)?],
    )?;
    bump_revision(&transaction)?;
    transaction.commit()?;
    Ok(true)
}
