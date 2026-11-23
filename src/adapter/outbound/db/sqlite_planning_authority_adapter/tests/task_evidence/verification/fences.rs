use super::*;

#[test]
fn host_verification_legacy_recipe_and_lease_do_not_claim_execution() {
    let f = HostFixture::new("host-verification-legacy");
    let mut task = f.base.snapshot().tasks[0].task.clone();
    task.acceptance[0].verification_recipe = None;
    task.acceptance_revision += 1;
    f.base.save(vec![task]);
    assert_eq!(
        reason(
            f.base
                .adapter
                .claim_host_verification(&f.base.workspace, &f.request("legacy-recipe"))
                .unwrap_err()
        ),
        TaskEvidenceError::Invalid
    );
    let mut task = f.base.snapshot().tasks[0].task.clone();
    task.acceptance[0].verification_recipe = Some(TaskVerificationRecipe::GitDiffCheckV1);
    task.acceptance_revision += 1;
    f.base.save(vec![task]);
    let mut legacy = f.base.lease.clone();
    legacy.lease_generation = None;
    f.base
        .adapter
        .upsert_runtime_slot_lease(&f.base.workspace, &legacy)
        .unwrap();
    assert_eq!(
        reason(
            f.base
                .adapter
                .claim_host_verification(&f.base.workspace, &f.request("legacy-lease"))
                .unwrap_err()
        ),
        TaskEvidenceError::LeaseChanged
    );
    assert_eq!(f.base.counters().1, 0);
}

#[test]
fn host_verification_scope_change_blocks_bind_but_keeps_the_terminal_audit_fact() {
    let f = HostFixture::new("host-verification-scope-change");
    let claim = f.claim("request-1");
    let mut task = f.base.snapshot().tasks[0].task.clone();
    task.acceptance[0].scenario = "A changed criterion".into();
    task.acceptance_revision += 1;
    f.base.save(vec![task]);
    let bind = TaskVerificationBindCommit {
        execution: claim.execution.clone().unwrap(),
        manifest: manifest(),
        at: NOW.into(),
    };
    assert_eq!(
        reason(
            f.base
                .adapter
                .bind_host_verification(&f.base.workspace, &bind)
                .unwrap_err()
        ),
        TaskEvidenceError::ScopeChanged
    );
    let finish = TaskVerificationFinishCommit {
        execution: bind.execution,
        result: TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::ScopeChanged,
        },
        artifact: None,
        at: NOW.into(),
    };
    let saved = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &finish)
        .unwrap();
    assert_eq!(saved.value.attempt.scope, claim.stored.attempt.scope);
    assert_ne!(f.base.snapshot().tasks[0].scope, saved.value.attempt.scope);
    let replay = f
        .base
        .adapter
        .claim_host_verification(
            &f.base.workspace,
            &TaskVerificationClaimCommit {
                scope_id: claim.stored.attempt.scope.scope_id.clone(),
                expected_acceptance_revision: claim.stored.attempt.scope.acceptance_revision,
                ..f.request("request-1")
            },
        )
        .unwrap();
    assert!(!replay.changed);
    assert!(replay.execution.is_none());
}

#[test]
fn host_verification_old_execution_finishes_after_delete_recreate_without_inheriting_scope() {
    let f = HostFixture::new("host-verification-scope-aba");
    let claim = f.claim("request-1");
    let bind = f.bind(&claim);
    let original = f.base.snapshot().tasks[0].task.clone();
    f.base.save(Vec::new());
    f.base.save(vec![original]);
    let current = f.base.snapshot().tasks[0].scope.clone();
    assert_ne!(current.scope_id, claim.stored.attempt.scope.scope_id);
    let saved = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &f.finish(&bind))
        .unwrap();
    assert!(saved.value.passed());
    assert_ne!(saved.value.attempt.scope, current);
    assert_eq!(
        f.base.snapshot().tasks[0].host_verifications[0]
            .receipt
            .as_ref(),
        Some(&saved.value)
    );
}

#[test]
fn host_verification_new_lease_generation_and_done_status_fence_binding() {
    for done in [false, true] {
        let f = HostFixture::new(&format!("host-verification-lease-{done}"));
        let claim = f.claim("request-1");
        if done {
            let mut task = f.base.snapshot().tasks[0].task.clone();
            task.status = TaskStatus::Done;
            f.base.save(vec![task]);
        } else {
            let mut lease = f.base.lease.clone();
            lease.lease_generation = Some("f".repeat(64));
            f.base
                .adapter
                .upsert_runtime_slot_lease(&f.base.workspace, &lease)
                .unwrap();
        }
        let bind = TaskVerificationBindCommit {
            execution: claim.execution.unwrap(),
            manifest: manifest(),
            at: NOW.into(),
        };
        assert_eq!(
            reason(
                f.base
                    .adapter
                    .bind_host_verification(&f.base.workspace, &bind)
                    .unwrap_err()
            ),
            TaskEvidenceError::LeaseChanged
        );
    }
}

#[test]
fn host_verification_fresh_owner_recovers_once_and_never_resumes_an_old_claim() {
    for running in [false, true] {
        let f = HostFixture::new(&format!("host-verification-restart-{running}"));
        let claim = f.claim("request-1");
        if running {
            f.bind(&claim);
        }
        let fresh = "f".repeat(64);
        let recovered = f
            .base
            .adapter
            .recover_incomplete_host_verifications(
                &f.base.workspace,
                &fresh,
                "2026-11-23T11:00:00Z",
            )
            .unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].result, TaskVerificationResult::Interrupted {});
        assert_eq!(recovered[0].manifest.is_some(), running);
        assert!(
            f.base
                .adapter
                .recover_incomplete_host_verifications(&f.base.workspace, &fresh, NOW)
                .unwrap()
                .is_empty()
        );
        let replay = f
            .base
            .adapter
            .claim_host_verification(
                &f.base.workspace,
                &TaskVerificationClaimCommit {
                    owner_token: fresh.clone(),
                    ..f.request("request-1")
                },
            )
            .unwrap();
        assert!(!replay.changed);
        assert!(replay.execution.is_none());
        assert_eq!(replay.stored.receipt.as_ref(), Some(&recovered[0]));
        let old = f
            .base
            .adapter
            .claim_host_verification(&f.base.workspace, &f.request("old-owner-request"));
        assert_eq!(reason(old.unwrap_err()), TaskEvidenceError::Conflict);
        let bind = TaskVerificationBindCommit {
            execution: claim.execution.unwrap(),
            manifest: manifest(),
            at: NOW.into(),
        };
        assert!(
            f.base
                .adapter
                .bind_host_verification(&f.base.workspace, &bind)
                .is_err()
        );
        assert_eq!(
            f.base
                .adapter
                .pending_events(&f.base.workspace, 10)
                .unwrap()
                .len(),
            1
        );
    }
}
