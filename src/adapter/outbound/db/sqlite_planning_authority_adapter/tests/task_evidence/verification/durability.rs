use super::*;

#[test]
fn host_verification_concurrent_claims_reserve_one_workspace_execution() {
    let f = HostFixture::new("host-verification-concurrency");
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|index| {
            let barrier = barrier.clone();
            let workspace = f.base.workspace.clone();
            let request = f.request(&format!("request-{index}"));
            std::thread::spawn(move || {
                barrier.wait();
                SqlitePlanningAuthorityAdapter::new().claim_host_verification(&workspace, &request)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let failure = results.into_iter().find_map(Result::err).unwrap();
    assert_eq!(reason(failure), TaskEvidenceError::Limit);
    assert_eq!(f.base.counters().1, 1);
    assert_eq!(f.base.counters().3, MAX_TASK_EVIDENCE_ARTIFACT_BYTES);
}

#[test]
fn host_verification_shares_reported_ordinal_and_reserves_bytes_before_execution() {
    let f = HostFixture::new("host-verification-shared-quota");
    assert_eq!(f.base.prepare("reported-1").attempt, 1);
    let claim = f.claim("host-1");
    assert_eq!(claim.stored.attempt.attempt, 2);
    let finish = TaskVerificationFinishCommit {
        execution: claim.execution.unwrap(),
        result: TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::Cancelled,
        },
        artifact: None,
        at: NOW.into(),
    };
    f.base
        .adapter
        .finish_host_verification(&f.base.workspace, &finish)
        .unwrap();
    assert_eq!(f.base.prepare("reported-2").attempt, 3);
    assert_eq!(f.base.counters().1, 3);
    assert_eq!(f.base.counters().3, 2 * MAX_TASK_EVIDENCE_ARTIFACT_BYTES);
    let connection = authority_connection(&f.base.workspace);
    connection.execute("UPDATE task_evidence_attempts SET ordinal=64,attempt_json=json_set(attempt_json,'$.attempt',64) WHERE request_id='reported-2'",[]).unwrap();
    assert_eq!(
        reason(
            f.base
                .adapter
                .claim_host_verification(&f.base.workspace, &f.request("host-limit"))
                .unwrap_err()
        ),
        TaskEvidenceError::Limit
    );
}

#[test]
fn host_verification_not_run_releases_only_its_artifact_reservation() {
    let f = HostFixture::new("host-verification-not-run-reservation");
    let claim = f.claim("host-1");
    let execution = claim.execution.clone().unwrap();
    let finish = TaskVerificationFinishCommit {
        execution,
        result: TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::Cancelled,
        },
        artifact: None,
        at: NOW.into(),
    };

    let saved = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &finish)
        .unwrap();
    assert!(saved.changed);
    assert!(matches!(
        saved.value.result,
        TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::Cancelled
        }
    ));
    assert_eq!(f.base.counters().1, 1);
    assert_eq!(f.base.counters().2, 0);
    assert_eq!(f.base.counters().3, 0);

    let replay = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &finish)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.value, saved.value);
    assert_eq!(f.base.counters().3, 0);

    // A later verification can reserve the released capacity.
    assert!(f.claim("host-2").execution.is_some());
    assert_eq!(f.base.counters().3, MAX_TASK_EVIDENCE_ARTIFACT_BYTES);
}

#[test]
fn host_verification_entry_and_disk_budgets_are_shared_with_the_e2_ledger() {
    for entries in [true, false] {
        let f = HostFixture::new(&format!("host-verification-budget-{entries}"));
        let connection = authority_connection(&f.base.workspace);
        if entries {
            connection
                .execute(
                    "UPDATE task_evidence_state SET entry_count=?1",
                    [MAX_TASK_EVIDENCE_RECEIPTS],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "UPDATE task_evidence_state SET reserved_bytes=?1",
                    [MAX_TASK_EVIDENCE_TOTAL_BYTES],
                )
                .unwrap();
        }
        let before = f.base.counters();
        assert_eq!(
            reason(
                f.base
                    .adapter
                    .claim_host_verification(&f.base.workspace, &f.request("exhausted"))
                    .unwrap_err()
            ),
            TaskEvidenceError::Limit
        );
        assert_eq!(f.base.counters(), before);
        let count: i64 = connection
            .query_row("SELECT count(*) FROM task_verification_runs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn host_verification_additive_schema_keeps_existing_e2_receipts_scope_and_reservations() {
    let f = Fixture::new("host-verification-additive-schema");
    let attempt = f.prepare("existing-reported");
    let receipt = f
        .adapter
        .record_evidence(&f.workspace, &f.record_commit(&attempt))
        .unwrap()
        .value;
    let scope = f.snapshot().tasks[0].scope.clone();
    let pending = f.prepare("pending-reported");
    let before = f.counters();
    {
        let connection = authority_connection(&f.workspace);
        connection
            .execute_batch("DROP TABLE task_verification_runs; DROP TABLE task_verification_state;")
            .unwrap();
    }
    let restored = f.snapshot();
    assert_eq!(restored.tasks[0].scope, scope);
    assert_eq!(restored.tasks[0].evidence, vec![receipt]);
    assert!(restored.tasks[0].host_verifications.is_empty());
    assert_eq!(
        f.adapter
            .load_attempt(&f.workspace, &pending.attempt_id)
            .unwrap()
            .unwrap()
            .attempt,
        pending
    );
    assert_eq!(f.counters(), before);
}

#[test]
fn host_verification_outbox_ack_requires_the_original_terminal_receipt() {
    let f = HostFixture::new("host-verification-outbox-original");
    let claim = f.claim("request-1");
    let bind = f.bind(&claim);
    let original = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &f.finish(&bind))
        .unwrap()
        .value;
    let mut changed = original.clone();
    changed.finished_at = "2026-11-23T12:00:01Z".into();
    let event = TaskEvidenceEvent::VerificationFinished {
        receipt: Box::new(changed),
    };
    authority_connection(&f.base.workspace)
        .execute(
            "UPDATE task_evidence_events SET payload_json=?1 WHERE event_id=?2",
            rusqlite::params![serde_json::to_string(&event).unwrap(), original.event_id],
        )
        .unwrap();
    assert_eq!(
        reason(
            f.base
                .adapter
                .acknowledge_event(&f.base.workspace, &original.event_id)
                .unwrap_err()
        ),
        TaskEvidenceError::Unavailable
    );
    let acknowledged: i64 = authority_connection(&f.base.workspace)
        .query_row(
            "SELECT acknowledged FROM task_evidence_events WHERE event_id=?1",
            [&original.event_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(acknowledged, 0);
}

#[test]
fn host_verification_projection_rejects_index_identity_drift() {
    let f = HostFixture::new("host-verification-index-identity");
    let claim = f.claim("request-1");
    let connection = authority_connection(&f.base.workspace);
    connection
        .execute(
            "UPDATE task_verification_runs SET criterion_id='criterion two' WHERE attempt_id=?1",
            [&claim.stored.attempt.attempt_id],
        )
        .unwrap();
    assert_eq!(
        reason(
            f.base
                .adapter
                .load_host_verification(&f.base.workspace, &claim.stored.attempt.attempt_id,)
                .unwrap_err()
        ),
        TaskEvidenceError::Unavailable
    );
    assert_eq!(
        reason(
            f.base
                .adapter
                .load_current_snapshot(
                    &f.base.workspace,
                    &TaskEvidenceSnapshotQuery {
                        task_ids: vec![claim.stored.attempt.scope.task_id],
                        after_task_id: None,
                        limit: 1,
                    },
                )
                .unwrap_err()
        ),
        TaskEvidenceError::Unavailable
    );
}
