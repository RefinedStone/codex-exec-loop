use super::*;

#[test]
fn failure_limit_old_scope_terminal_fact_cannot_pause_changed_or_recreated_task() {
    for recreate in [false, true] {
        let f = fixture(&format!("host-failure-scope-{recreate}"));
        fail(&f, "old-1");
        fail(&f, "old-2");
        let claim = f.claim("old-late-3");
        let bind = f.bind(&claim);
        let old_scope = claim.stored.attempt.scope.clone();
        let mut changed = task(&f);
        if recreate {
            f.base.save(Vec::new());
        } else {
            changed.acceptance[0].scenario =
                "The replacement criterion has different meaning".into();
            changed.acceptance_revision += 1;
        }
        f.base.save(vec![changed]);
        let current_scope = f.base.snapshot().tasks[0].scope.clone();
        assert_ne!(current_scope, old_scope);
        let mut finish = f.finish(&bind);
        finish.result = observation(TaskVerificationExecutionOutcome::Exited { exit_code: 2 });
        let receipt = f
            .base
            .adapter
            .finish_host_verification(&f.base.workspace, &finish)
            .unwrap()
            .value;
        assert_eq!(receipt.attempt.scope, old_scope);
        let before = task(&f);
        let decision = apply(&f, &receipt);
        assert_eq!(
            decision.outcome,
            TaskVerificationFailureLimitOutcome::Superseded
        );
        assert_eq!(decision.failure_count, 3);
        assert!(!decision.changed);
        assert_eq!(task(&f), before);
        assert_eq!(f.base.snapshot().tasks[0].scope, current_scope);
        assert_eq!(apply(&f, &receipt), decision);
    }
}

#[test]
fn failure_limit_superseded_task_or_lease_is_not_re_evaluated_on_retry() {
    for replacement_lease in [false, true] {
        let f = fixture(&format!("host-failure-superseded-{replacement_lease}"));
        let receipts = three_failures(&f);
        if replacement_lease {
            let mut replacement = f.base.lease.clone();
            replacement.lease_generation = Some("f".repeat(64));
            f.base
                .adapter
                .upsert_runtime_slot_lease(&f.base.workspace, &replacement)
                .unwrap();
        } else {
            let mut waiting = task(&f);
            waiting.status = TaskStatus::AwaitingUser;
            waiting.last_updated_by = TaskActor::User;
            f.base.save(vec![waiting]);
        }
        let decision = apply(&f, &receipts[2]);
        assert_eq!(
            decision.outcome,
            TaskVerificationFailureLimitOutcome::Superseded
        );
        if replacement_lease {
            f.base
                .adapter
                .upsert_runtime_slot_lease(&f.base.workspace, &f.base.lease)
                .unwrap();
        } else {
            let mut resumed = task(&f);
            resumed.status = TaskStatus::InProgress;
            resumed.last_updated_by = TaskActor::User;
            f.base.save(vec![resumed]);
        }
        let before = task(&f);
        let revision = planning_revision(&f);
        assert_eq!(apply(&f, &receipts[2]), decision);
        assert_eq!(task(&f), before);
        assert_eq!(planning_revision(&f), revision);
    }
}

#[test]
fn failure_limit_respects_current_waiver_and_optional_criterion() {
    for waived in [false, true] {
        let f = fixture(&format!("host-failure-exception-{waived}"));
        if !waived {
            let mut optional = task(&f);
            optional.acceptance[0].essential = false;
            optional.acceptance_revision += 1;
            f.base.save(vec![optional]);
        }
        let receipts = three_failures(&f);
        if waived {
            f.base
                .adapter
                .waive_criterion(
                    &f.base.workspace,
                    &TaskCriterionWaiverCommit {
                        request_id: "human-waiver".into(),
                        scope: receipts[2].attempt.scope.clone(),
                        criterion_id: receipts[2].attempt.criterion_id.clone(),
                        reason: "The operator accepted this criterion exception".into(),
                        confirmation_ref: "verified-native-confirmation-fixture".into(),
                        at: NOW.into(),
                    },
                )
                .unwrap();
        }
        let before = task(&f);
        assert_eq!(
            apply(&f, &receipts[2]).outcome,
            TaskVerificationFailureLimitOutcome::Superseded
        );
        assert_eq!(task(&f), before);
    }
}

#[test]
fn failure_limit_admin_reset_and_guard_preserve_authority_and_original_receipt() {
    let f = fixture("host-failure-admin-guard");
    let receipts = three_failures(&f);
    let ids = vec![f.base.lease.task_id.clone()];
    f.base
        .adapter
        .clear_parallel_runtime_projections_for_tasks(
            &f.base.workspace,
            &ids,
            "operator reset before editing",
        )
        .unwrap();
    f.base
        .adapter
        .acquire_admin_task_mutation_guard(&f.base.workspace, &ids, "admin-owner")
        .unwrap();
    let before = task(&f);
    let result = apply(&f, &receipts[2]);
    assert_eq!(
        result.outcome,
        TaskVerificationFailureLimitOutcome::Superseded
    );
    assert_eq!(task(&f), before);
    assert_eq!(
        f.base
            .adapter
            .load_host_verification(&f.base.workspace, &receipts[2].attempt.attempt_id)
            .unwrap()
            .unwrap()
            .receipt
            .as_ref(),
        Some(&receipts[2])
    );
    f.base
        .adapter
        .release_admin_task_mutation_guard(&f.base.workspace, &ids, "admin-owner")
        .unwrap();
    f.base
        .adapter
        .upsert_runtime_slot_lease(&f.base.workspace, &f.base.lease)
        .unwrap();
    assert_eq!(apply(&f, &receipts[2]), result);
    assert_eq!(task(&f), before);
}

#[test]
fn failure_limit_requires_current_owner_and_the_original_captured_lease() {
    let f = fixture("host-failure-owner");
    let receipts = three_failures(&f);
    let receipt = &receipts[2];
    let fresh_owner = "f".repeat(64);
    f.base
        .adapter
        .recover_incomplete_host_verifications(&f.base.workspace, &fresh_owner, NOW)
        .unwrap();
    assert_eq!(
        reason(
            f.base
                .adapter
                .apply_host_verification_failure_limit(&f.base.workspace, &request(&f, receipt))
                .unwrap_err()
        ),
        TaskEvidenceError::Conflict
    );
    let mut forged = request(&f, receipt);
    forged.owner_token = fresh_owner.clone();
    forged.expected_lease.lease_generation = Some("b".repeat(64));
    assert_eq!(
        reason(
            f.base
                .adapter
                .apply_host_verification_failure_limit(&f.base.workspace, &forged)
                .unwrap_err()
        ),
        TaskEvidenceError::LeaseChanged
    );
    assert!(marker(&f, receipt).is_none());
    let mut allowed = request(&f, receipt);
    allowed.owner_token = fresh_owner;
    let result = f
        .base
        .adapter
        .apply_host_verification_failure_limit(&f.base.workspace, &allowed)
        .unwrap();
    assert_eq!(
        result.outcome,
        TaskVerificationFailureLimitOutcome::AwaitingUser
    );
    assert!(result.changed);
}

#[test]
fn failure_limit_pending_is_not_a_terminal_fact_and_recovery_is_not_a_failure() {
    let f = fixture("host-failure-pending");
    fail(&f, "known-1");
    fail(&f, "known-2");
    let claim = f.claim("unfinished");
    let mut policy = TaskVerificationFailureLimitCommit {
        attempt_id: claim.stored.attempt.attempt_id,
        expected_lease: f.base.lease.clone(),
        owner_token: f.owner.clone(),
        at: NOW.into(),
    };
    assert_eq!(
        reason(
            f.base
                .adapter
                .apply_host_verification_failure_limit(&f.base.workspace, &policy)
                .unwrap_err()
        ),
        TaskEvidenceError::Invalid
    );
    policy.owner_token = "f".repeat(64);
    let recovered = f
        .base
        .adapter
        .recover_incomplete_host_verifications(&f.base.workspace, &policy.owner_token, NOW)
        .unwrap();
    assert_eq!(recovered.len(), 1);
    let result = f
        .base
        .adapter
        .apply_host_verification_failure_limit(&f.base.workspace, &policy)
        .unwrap();
    assert_eq!(
        result.outcome,
        TaskVerificationFailureLimitOutcome::NotFailed
    );
    assert!(!result.changed);
    assert_ne!(task(&f).status, TaskStatus::AwaitingUser);
}
