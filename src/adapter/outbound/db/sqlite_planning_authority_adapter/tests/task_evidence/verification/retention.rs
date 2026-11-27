use super::*;

#[test]
fn pressure_reuses_one_expired_pending_interruption_charge_without_altering_its_receipt() {
    let f = HostFixture::new("unused-verification-charge");
    let claim = f.claim("interrupted-before-bind");
    let original = f
        .base
        .adapter
        .recover_incomplete_host_verifications(
            &f.base.workspace,
            &"9".repeat(64),
            "2001-01-01T00:00:00Z",
        )
        .unwrap()
        .remove(0);
    f.base
        .adapter
        .acknowledge_event(&f.base.workspace, &original.event_id)
        .unwrap();
    let mut retired = f.base.snapshot().tasks[0].task.clone();
    let mut active = retired.clone();
    active.id = "new-task".into();
    retired.status = TaskStatus::Done;
    retired.updated_at = "2001-01-01T00:00:00Z".into();
    f.base.save(vec![retired.clone(), active.clone()]);
    let db = authority_connection(&f.base.workspace);
    db.execute(
        "UPDATE task_evidence_state SET reserved_bytes=?1",
        [MAX_TASK_EVIDENCE_TOTAL_BYTES],
    )
    .unwrap();
    let old_lease = f.base.lease.clone();
    let mut new_lease = old_lease.clone();
    new_lease.task_id = active.id.clone();
    new_lease.slot_id = "slot-new".into();
    new_lease.lease_generation = Some("7".repeat(64));
    f.base
        .adapter
        .upsert_runtime_slot_lease(&f.base.workspace, &new_lease)
        .unwrap();
    let current = f
        .base
        .snapshot()
        .tasks
        .into_iter()
        .find(|task| task.task.id == active.id)
        .unwrap();
    let request = TaskEvidencePrepareCommit {
        request_id: "new-report".into(),
        scope: current.scope,
        criterion_id: "criterion one".into(),
        lease: new_lease,
        prepared_tree: manifest().source,
        at: NOW.into(),
    };
    assert_eq!(
        reason(
            f.base
                .adapter
                .prepare_attempt(&f.base.workspace, &request)
                .unwrap_err()
        ),
        TaskEvidenceError::Limit
    );
    db.execute(
        "DELETE FROM runtime_slot_leases WHERE slot_id=?1",
        [&old_lease.slot_id],
    )
    .unwrap();
    assert!(
        f.base
            .adapter
            .prepare_attempt(&f.base.workspace, &request)
            .unwrap()
            .changed
    );
    assert_eq!(
        f.base
            .adapter
            .load_host_verification(&f.base.workspace, &claim.stored.attempt.attempt_id)
            .unwrap()
            .unwrap()
            .receipt,
        Some(original.clone())
    );
    let count: u64 = db.query_row(
        "SELECT COUNT(*) FROM task_evidence_retired_charges WHERE receipt_event_id=?1 AND reserved_bytes=?2",
        rusqlite::params![original.event_id, MAX_TASK_EVIDENCE_ARTIFACT_BYTES], |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        reason(
            f.base
                .adapter
                .prepare_attempt(
                    &f.base.workspace,
                    &TaskEvidencePrepareCommit {
                        request_id: "cannot-reuse-again".into(),
                        ..request
                    }
                )
                .unwrap_err()
        ),
        TaskEvidenceError::Limit
    );
}
