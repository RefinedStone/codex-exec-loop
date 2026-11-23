use super::*;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningValidationRecoveryKind, PlanningValidationRecoveryOutcome,
    PlanningValidationRecoveryRequest, PlanningWorkPermit, PlanningWorkPermitBasis,
    PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::domain::parallel_mode::ParallelModeQueueItemState;

fn queue_record(key: &str, task_id: &str) -> PlanningAuthorityDistributorQueueRecord {
    serde_json::from_value(serde_json::json!({
        "queue_item_id":key,"session_key":format!("session-{key}"),"agent_id":"agent",
        "task_id":task_id,"task_title":"Synthetic result","branch_name":"agent/branch",
        "worktree_path":"/tmp/synthetic-branch","commit_sha":"a".repeat(40),
        "validation_summary":"","authority_refresh_outcome":"",
        "queue_state":ParallelModeQueueItemState::Queued,"integration_note":"",
        "enqueued_at":"2026-11-21T00:00:00Z","updated_at":"2026-11-21T00:00:00Z"
    }))
    .unwrap()
}

#[test]
fn distributor_queue_and_immutable_validation_source_are_saved_together() {
    let workspace = workspace("validation-source-atomic");
    seed(&workspace, vec![task("legacy-source")]);
    register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let record = queue_record("validation-owned", "owned-source");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace, &record)
        .unwrap();
    let source = adapter
        .load_work_order_validation_source(&workspace, &record.queue_item_id)
        .unwrap()
        .unwrap();
    assert_eq!(source.work_order_id.as_deref(), Some("order-a"));
    assert_eq!(source.source_task_id, "owned-source");
    let mut rebound = record.clone();
    rebound.task_id = "legacy-source".into();
    assert!(
        adapter
            .upsert_runtime_distributor_queue_record(&workspace, &rebound)
            .is_err()
    );
    assert_eq!(
        adapter
            .load_runtime_projections(&workspace)
            .unwrap()
            .distributor_queue_records,
        vec![record.clone()]
    );
    connection(&workspace)
        .execute("DELETE FROM runtime_distributor_queue", [])
        .unwrap();
    assert_eq!(
        adapter
            .load_work_order_validation_source(&workspace, &record.queue_item_id)
            .unwrap(),
        Some(source)
    );
    assert_eq!(
        adapter
            .work_order_validation_record_keys(&workspace, "order-a")
            .unwrap(),
        ["validation-owned"]
    );
}

#[test]
fn missing_source_is_distinct_from_verified_legacy_and_cannot_silently_rebind() {
    let workspace = workspace("validation-source-legacy");
    seed(&workspace, vec![task("legacy-source")]);
    register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    assert!(
        adapter
            .load_work_order_validation_source(&workspace, "unknown-record")
            .unwrap()
            .is_none()
    );
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace,
            &queue_record("legacy-record", "legacy-source"),
        )
        .unwrap();
    let legacy = adapter
        .load_work_order_validation_source(&workspace, "legacy-record")
        .unwrap()
        .unwrap();
    assert!(legacy.work_order_id.is_none());
    assert_eq!(legacy.source_task_id, "legacy-source");
    assert!(
        adapter
            .load_work_order_validation_link(&workspace, "legacy-record")
            .unwrap()
            .is_none()
    );
    assert!(
        adapter
            .link_supplemental_task(&workspace, "order-a", "legacy-source")
            .is_err()
    );
}

#[test]
fn failed_validation_source_write_rolls_back_queue_and_source() {
    let workspace = workspace("validation-source-rollback");
    seed(&workspace, vec![task("source")]);
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .load_work_order_validation_source(&workspace, "unused")
        .unwrap();
    connection(&workspace).execute_batch(
        "CREATE TRIGGER fail_validation_source BEFORE INSERT ON planning_work_order_validation_links BEGIN SELECT RAISE(ABORT,'synthetic source failure'); END;"
    ).unwrap();
    assert!(
        adapter
            .upsert_runtime_distributor_queue_record(
                &workspace,
                &queue_record("failed-source", "source")
            )
            .is_err()
    );
    assert!(
        adapter
            .load_runtime_projections(&workspace)
            .unwrap()
            .distributor_queue_records
            .is_empty()
    );
    assert_eq!(count(&workspace, "planning_work_order_validation_links"), 0);
}

#[test]
fn preupgrade_queue_repairs_source_but_missing_queue_never_proves_legacy() {
    let workspace = workspace("validation-source-upgrade");
    seed(&workspace, vec![]);
    register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let record = queue_record("old-record", "owned-source");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace, &record)
        .unwrap();
    connection(&workspace)
        .execute("DELETE FROM planning_work_order_validation_links", [])
        .unwrap();
    assert_eq!(
        adapter
            .load_work_order_validation_source(&workspace, "old-record")
            .unwrap()
            .unwrap()
            .work_order_id
            .as_deref(),
        Some("order-a")
    );
    connection(&workspace)
        .execute("DELETE FROM planning_work_order_validation_links", [])
        .unwrap();
    connection(&workspace)
        .execute("DELETE FROM runtime_distributor_queue", [])
        .unwrap();
    assert!(
        adapter
            .load_work_order_validation_source(&workspace, "old-record")
            .unwrap()
            .is_none()
    );
}

fn validation_links(workspace: &str) -> Vec<(String, Option<String>, String, i64)> {
    connection(workspace)
        .prepare(
            "SELECT validation_record_key, work_order_id, source_task_id, ordinal
             FROM planning_work_order_validation_links ORDER BY validation_record_key",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn preupgrade_validation_keys_first_read_repairs_owned_queue_and_survives_reopen() {
    let workspace = workspace("validation-keys-upgrade");
    seed(&workspace, vec![task("unowned-source")]);
    register_next(&workspace, "order-a", "owned-source");
    register_next(&workspace, "order-b", "foreign-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let basis = PlanningWorkPermitBasis {
        delivery_review: None,
        authorization_epoch: 0,
        policy_version: "original-policy".into(),
        field_hash: "original-field-hash".into(),
        gate_revision: 7,
        source_sha: "a".repeat(40),
    };
    adapter
        .set_work_order_permit_basis(&workspace, "order-a", &basis)
        .unwrap();
    adapter
        .grant_work_order_permit(
            &workspace,
            &PlanningWorkPermit {
                work_order_id: "order-a".into(),
                stage: PlanningWorkPermitStage::Dispatch,
                basis: basis.clone(),
                requires_human_review: false,
            },
        )
        .unwrap();
    let original = stored(&workspace, "order-a");
    assert_eq!(original.permits.len(), 1);
    for record in [
        queue_record("old-b", "owned-source"),
        queue_record("foreign-record", "foreign-source"),
        queue_record("unowned-record", "unowned-source"),
        queue_record("old-a", "owned-source"),
    ] {
        adapter
            .upsert_runtime_distributor_queue_record(&workspace, &record)
            .unwrap();
    }
    // The parent version retains the queue and membership, but has no link table.
    connection(&workspace)
        .execute_batch("DROP TABLE planning_work_order_validation_links;")
        .unwrap();
    let queue_before = adapter
        .load_runtime_projections(&workspace)
        .unwrap()
        .distributor_queue_records;
    let planning_revision = snapshot(&workspace).planning_revision;
    let events_before = count(&workspace, "runtime_events");

    // Discovery must repair the sources without first looking up any known validation key.
    assert_eq!(
        adapter
            .work_order_validation_record_keys(&workspace, "order-a")
            .unwrap(),
        ["old-a", "old-b"]
    );
    let expected: Vec<(String, Option<String>, String, i64)> = vec![
        (
            "old-a".into(),
            Some("order-a".into()),
            "owned-source".into(),
            0,
        ),
        (
            "old-b".into(),
            Some("order-a".into()),
            "owned-source".into(),
            1,
        ),
    ];
    assert_eq!(validation_links(&workspace), expected);
    assert_eq!(count(&workspace, "runtime_events"), events_before + 2);
    assert_eq!(snapshot(&workspace).planning_revision, planning_revision);
    assert_eq!(stored(&workspace, "order-a"), original);
    assert_eq!(
        adapter
            .load_work_order_permit_basis(&workspace, "order-a")
            .unwrap(),
        Some(basis.clone())
    );
    assert_eq!(
        adapter
            .load_runtime_projections(&workspace)
            .unwrap()
            .distributor_queue_records,
        queue_before
    );

    let reopened = SqlitePlanningAuthorityAdapter::new();
    for _ in 0..2 {
        assert_eq!(
            reopened
                .work_order_validation_record_keys(&workspace, "order-a")
                .unwrap(),
            ["old-a", "old-b"]
        );
        assert_eq!(validation_links(&workspace), expected);
        assert_eq!(count(&workspace, "runtime_events"), events_before + 2);
    }
    assert!(
        reopened
            .work_order_validation_record_keys(&workspace, "unknown-order")
            .unwrap()
            .is_empty()
    );
    assert_eq!(validation_links(&workspace), expected);

    connection(&workspace)
        .execute("DELETE FROM runtime_distributor_queue", [])
        .unwrap();
    assert_eq!(
        reopened
            .work_order_validation_record_keys(&workspace, "order-a")
            .unwrap(),
        ["old-a", "old-b"]
    );
    assert_eq!(validation_links(&workspace), expected);
    assert_eq!(count(&workspace, "runtime_events"), events_before + 2);
    assert_eq!(snapshot(&workspace).planning_revision, planning_revision);
    assert_eq!(stored(&workspace, "order-a"), original);
    assert_eq!(
        reopened
            .load_work_order_permit_basis(&workspace, "order-a")
            .unwrap(),
        Some(basis)
    );
}

fn queue_record_at(
    key: &str,
    task_id: &str,
    enqueued_at: &str,
) -> PlanningAuthorityDistributorQueueRecord {
    let mut record = queue_record(key, task_id);
    record.enqueued_at = enqueued_at.into();
    record.updated_at = enqueued_at.into();
    record
}

#[test]
fn preupgrade_validation_key_backfill_preserves_identity_and_existing_relative_order() {
    let workspace = workspace("validation-keys-existing");
    seed(&workspace, vec![]);
    register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    for (key, time) in [
        ("retained-first", "2026-11-21T11:00:00+02:00"),
        ("retained-last", "2026-11-21T11:00:00Z"),
        ("missing-middle", "2026-11-21T10:00:00Z"),
    ] {
        adapter
            .upsert_runtime_distributor_queue_record(
                &workspace,
                &queue_record_at(key, "owned-source", time),
            )
            .unwrap();
    }
    connection(&workspace)
        .execute(
            "DELETE FROM planning_work_order_validation_links WHERE validation_record_key='missing-middle'",
            [],
        )
        .unwrap();
    let retained = validation_links(&workspace);
    let events_before = count(&workspace, "runtime_events");

    // 11:00+02:00 precedes 10:00Z. Existing source identity and relative order survive,
    // but the later retained source moves to make room for the proven legacy record.
    assert_eq!(
        adapter
            .work_order_validation_record_keys(&workspace, "order-a")
            .unwrap(),
        ["retained-first", "missing-middle", "retained-last"]
    );
    let after = validation_links(&workspace);
    for original in &retained {
        let repaired = after.iter().find(|row| row.0 == original.0).unwrap();
        assert_eq!(
            (&repaired.0, &repaired.1, &repaired.2),
            (&original.0, &original.1, &original.2)
        );
    }
    assert_eq!(
        after
            .iter()
            .find(|row| row.0 == "retained-first")
            .unwrap()
            .3,
        0
    );
    assert_eq!(
        after
            .iter()
            .find(|row| row.0 == "missing-middle")
            .unwrap()
            .3,
        1
    );
    assert_eq!(
        after.iter().find(|row| row.0 == "retained-last").unwrap().3,
        2
    );
    assert_eq!(count(&workspace, "runtime_events"), events_before + 1);
    assert_eq!(
        adapter
            .work_order_validation_record_keys(&workspace, "order-a")
            .unwrap(),
        ["retained-first", "missing-middle", "retained-last"]
    );
    assert_eq!(validation_links(&workspace), after);
    assert_eq!(count(&workspace, "runtime_events"), events_before + 1);
}

#[test]
fn preupgrade_mismatched_queue_identity_rolls_back_all_discovered_links() {
    let workspace = workspace("validation-keys-identity");
    seed(&workspace, vec![]);
    let original = register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    for key in ["retained", "01-valid", "02-invalid"] {
        adapter
            .upsert_runtime_distributor_queue_record(&workspace, &queue_record(key, "owned-source"))
            .unwrap();
    }
    let mut mismatched = queue_record("02-invalid", "owned-source");
    mismatched.queue_item_id = "different-payload-identity".into();
    let connection = connection(&workspace);
    connection
        .execute(
            "DELETE FROM planning_work_order_validation_links WHERE validation_record_key != 'retained'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE runtime_distributor_queue SET content=?1 WHERE queue_item_id='02-invalid'",
            [serde_json::to_string(&mismatched).unwrap()],
        )
        .unwrap();
    drop(connection);
    let retained = validation_links(&workspace);
    let events_before = count(&workspace, "runtime_events");
    let revision_before = snapshot(&workspace).planning_revision;
    let error = adapter
        .work_order_validation_record_keys(&workspace, "order-a")
        .unwrap_err();
    assert!(
        error.to_string().contains("queue identity mismatch"),
        "{error:#}"
    );
    // 01-valid was visited first; its insert and event must roll back with 02-invalid.
    assert_eq!(validation_links(&workspace), retained);
    assert_eq!(count(&workspace, "runtime_events"), events_before);
    assert_eq!(snapshot(&workspace).planning_revision, revision_before);
    assert_eq!(stored(&workspace, "order-a"), original);
}

#[test]
fn preupgrade_unprovable_mixed_order_rolls_back_imports_and_existing_ordinals() {
    for scenario in ["pruned", "invalid-time", "contradiction", "tie", "overflow"] {
        let workspace = workspace(&format!("validation-order-{scenario}"));
        seed(&workspace, vec![]);
        register_next(&workspace, "order-a", "owned-source");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        for (key, time) in [
            ("retained-first", "2026-11-21T09:00:00Z"),
            ("retained-last", "2026-11-21T11:00:00Z"),
            ("missing-middle", "2026-11-21T10:00:00Z"),
        ] {
            adapter
                .upsert_runtime_distributor_queue_record(
                    &workspace,
                    &queue_record_at(key, "owned-source", time),
                )
                .unwrap();
        }
        let db = connection(&workspace);
        db.execute(
            "DELETE FROM planning_work_order_validation_links WHERE validation_record_key='missing-middle'",
            [],
        )
        .unwrap();
        let expected_error = match scenario {
            "pruned" => {
                db.execute(
                    "DELETE FROM runtime_distributor_queue WHERE queue_item_id='retained-last'",
                    [],
                )
                .unwrap();
                "requires every existing source queue"
            }
            "overflow" => {
                db.execute(
                    "UPDATE planning_work_order_validation_links SET ordinal=?1 WHERE validation_record_key='retained-last'",
                    [i64::MAX - 1],
                ).unwrap();
                "ordinal exhausted"
            }
            _ => {
                let time = match scenario {
                    "invalid-time" => "not-a-timestamp",
                    "contradiction" => "2026-11-21T12:00:00Z",
                    "tie" => "2026-11-21T10:00:00Z",
                    _ => unreachable!(),
                };
                let record = queue_record_at("retained-first", "owned-source", time);
                db.execute(
                    "UPDATE runtime_distributor_queue SET content=?1,enqueued_at=?2 WHERE queue_item_id='retained-first'",
                    rusqlite::params![serde_json::to_string(&record).unwrap(), time],
                ).unwrap();
                match scenario {
                    "invalid-time" => "invalid validation source enqueue timestamp",
                    "contradiction" => "contradicts existing source order",
                    "tie" => "ambiguous at an existing source",
                    _ => unreachable!(),
                }
            }
        };
        drop(db);
        let retained = validation_links(&workspace);
        let events_before = count(&workspace, "runtime_events");
        let authority_before = snapshot(&workspace);
        let work_before = stored(&workspace, "order-a");
        let error = adapter
            .work_order_validation_record_keys(&workspace, "order-a")
            .unwrap_err();
        assert!(
            format!("{error:#}").contains(expected_error),
            "{scenario}: {error:#}"
        );
        // The missing source was inserted before anchor/order validation. Its event and
        // every ordinal relocation must roll back together when the proof is unavailable.
        assert_eq!(validation_links(&workspace), retained, "{scenario}");
        assert_eq!(
            count(&workspace, "runtime_events"),
            events_before,
            "{scenario}"
        );
        assert_eq!(snapshot(&workspace), authority_before, "{scenario}");
        assert_eq!(stored(&workspace, "order-a"), work_before, "{scenario}");
    }
}

#[test]
fn preupgrade_direct_recovery_checks_latest_source_after_backfill() {
    use crate::domain::parallel_mode::{
        PrValidationCommitSha, PrValidationEvent, PrValidationRecord, PrValidationRecordKey,
        PrValidationTarget, PrValidationTargetShaSnapshot, PrValidationTerminalReason,
    };
    let workspace = workspace("validation-recovery-latest-legacy");
    seed(&workspace, vec![]);
    register_next(&workspace, "order-a", "owned-source");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let basis = PlanningWorkPermitBasis {
        delivery_review: None,
        authorization_epoch: 0,
        policy_version: "original-policy".into(),
        field_hash: "original-field".into(),
        gate_revision: 1,
        source_sha: "a".repeat(40),
    };
    adapter
        .set_work_order_permit_basis(&workspace, "order-a", &basis)
        .unwrap();
    let validation = |key: &str| {
        PrValidationRecord::register(
            PrValidationRecordKey::new(key).unwrap(),
            PrValidationTarget::new("sample/widgets", 1).unwrap(),
            PrValidationTargetShaSnapshot::new(
                PrValidationCommitSha::new("a".repeat(40)).unwrap(),
                PrValidationCommitSha::new("b".repeat(40)).unwrap(),
            ),
        )
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
    };
    let original = validation("older-validation")
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let newer = validation("newer-validation");
    for (record, time) in [
        (&original, "2026-11-21T09:00:00Z"),
        (&newer, "2026-11-21T10:00:00Z"),
    ] {
        let mut queued = queue_record_at(record.key().as_str(), "owned-source", time);
        queued.pull_request_number = Some(1);
        adapter
            .upsert_runtime_distributor_queue_record(&workspace, &queued)
            .unwrap();
        assert!(
            adapter
                .compare_and_swap_runtime_pr_validation_record(
                    &workspace,
                    record.key(),
                    None,
                    Some(record),
                )
                .unwrap()
        );
    }
    connection(&workspace).execute(
        "DELETE FROM planning_work_order_validation_links WHERE validation_record_key='newer-validation'",
        [],
    ).unwrap();
    let order = stored(&workspace, "order-a");
    let before = snapshot(&workspace);
    let links_before = validation_links(&workspace);
    let events_before = count(&workspace, "runtime_events");
    let receipt = adapter
        .recover_work_order_validation(
            &workspace,
            PlanningValidationRecoveryRequest {
                command_id: "stale-recovery",
                work_order_id: "order-a",
                terminal_record_id: "older-terminal",
                kind: PlanningValidationRecoveryKind::ObserveAgain,
                authorization_epoch: 0,
                gate_revision: 1,
                policy_version: "original-policy",
                original: &original,
                observed_work_order: &order,
                observed_planning_revision: before.planning_revision,
                is_current_terminal: true,
            },
        )
        .unwrap();
    assert_eq!(
        receipt.outcome,
        PlanningValidationRecoveryOutcome::Blocked {
            reason: "validation_changed".into()
        }
    );
    assert_eq!(
        count(&workspace, "planning_work_order_validation_recoveries"),
        0
    );
    assert_eq!(count(&workspace, "runtime_pr_validation_records"), 2);
    assert_eq!(validation_links(&workspace), links_before);
    assert_eq!(count(&workspace, "runtime_events"), events_before);
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(stored(&workspace, "order-a"), order);
    assert_eq!(
        adapter
            .load_runtime_pr_validation_record(&workspace, original.key())
            .unwrap(),
        Some(original)
    );
}
