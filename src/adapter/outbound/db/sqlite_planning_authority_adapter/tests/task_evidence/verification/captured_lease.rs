use super::*;

#[test]
fn automatic_host_claim_keeps_the_launch_generation_across_leased_to_running() {
    let f = HostFixture::new("host-captured-launch");
    let mut launch = f.base.lease.clone();
    launch.state = ParallelModeSlotLeaseState::Leased;
    launch.running_started_at = None;
    let request = TaskVerificationClaimCommit {
        expected_lease: Some(launch),
        ..f.request("automatic-launch")
    };
    let claim = f
        .base
        .adapter
        .claim_host_verification(&f.base.workspace, &request)
        .unwrap();
    assert_eq!(claim.stored.lease, f.base.lease);
    assert_eq!(
        claim.stored.lease.state,
        ParallelModeSlotLeaseState::Running
    );
    let replay = f
        .base
        .adapter
        .claim_host_verification(
            &f.base.workspace,
            &TaskVerificationClaimCommit {
                expected_lease: Some(f.base.lease.clone()),
                ..request
            },
        )
        .unwrap();
    assert!(!replay.changed);
    assert!(replay.execution.is_none());
    assert_eq!(replay.stored, claim.stored);
}

#[test]
fn automatic_host_claim_never_selects_a_replacement_generation_or_frozen_target() {
    for changed_target in [false, true] {
        let f = HostFixture::new(&format!("host-captured-stale-{changed_target}"));
        let mut captured = f.base.lease.clone();
        if changed_target {
            captured
                .delivery_target
                .as_mut()
                .unwrap()
                .integration_base_commit_sha = "c".repeat(40);
        } else {
            captured.lease_generation = Some("f".repeat(64));
        }
        let before = f.base.counters();
        let error = f
            .base
            .adapter
            .claim_host_verification(
                &f.base.workspace,
                &TaskVerificationClaimCommit {
                    expected_lease: Some(captured),
                    ..f.request("automatic-stale")
                },
            )
            .unwrap_err();
        assert_eq!(reason(error), TaskEvidenceError::LeaseChanged);
        assert_eq!(f.base.counters(), before);
        // Explicit command behavior still selects the current authorized Running lease.
        assert_eq!(f.claim("manual-current").stored.lease, f.base.lease);
    }
}

#[test]
fn automatic_host_replay_retains_original_receipt_but_rejects_rebinding_its_id() {
    let f = HostFixture::new("host-captured-replay");
    let request = TaskVerificationClaimCommit {
        expected_lease: Some(f.base.lease.clone()),
        ..f.request("automatic-once")
    };
    let claim = f
        .base
        .adapter
        .claim_host_verification(&f.base.workspace, &request)
        .unwrap();
    let bind = f.bind(&claim);
    let receipt = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &f.finish(&bind))
        .unwrap()
        .value;
    let mut replacement = f.base.lease.clone();
    replacement.lease_generation = Some("f".repeat(64));
    f.base
        .adapter
        .upsert_runtime_slot_lease(&f.base.workspace, &replacement)
        .unwrap();
    let before = f.base.counters();
    let replay = f
        .base
        .adapter
        .claim_host_verification(&f.base.workspace, &request)
        .unwrap();
    assert!(!replay.changed);
    assert!(replay.execution.is_none());
    assert_eq!(replay.stored.receipt.as_ref(), Some(&receipt));
    for expected_lease in [Some(replacement), None] {
        assert_eq!(
            reason(
                f.base
                    .adapter
                    .claim_host_verification(
                        &f.base.workspace,
                        &TaskVerificationClaimCommit {
                            expected_lease,
                            ..request.clone()
                        }
                    )
                    .unwrap_err()
            ),
            TaskEvidenceError::Conflict
        );
    }
    assert_eq!(f.base.counters(), before);
}

#[test]
fn explicit_host_claim_preserves_the_legacy_fingerprint() {
    let f = HostFixture::new("host-manual-fingerprint");
    let request = f.request("manual-legacy");
    let claim = f.claim(&request.request_id);
    let expected = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                &request.task_id,
                &request.criterion_id,
                &request.scope_id,
                request.expected_acceptance_revision,
            ))
            .unwrap()
        )
    );
    let stored: String = authority_connection(&f.base.workspace)
        .query_row(
            "SELECT claim_fingerprint FROM task_verification_runs WHERE attempt_id=?1",
            [&claim.stored.attempt.attempt_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, expected);
}
