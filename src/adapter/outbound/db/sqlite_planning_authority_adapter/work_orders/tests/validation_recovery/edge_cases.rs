use super::*;
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::AUTHORITY_STORE_SCHEMA_VERSION;
use crate::domain::parallel_mode::{
    IntegrationAttestation, IntegrationMethod, PostMergeValidationContract,
};
use chrono::{DateTime, Utc};

fn integrated_failure(reason: PrValidationTerminalReason) -> PrValidationRecord {
    let record = PrValidationRecord::register(
        PrValidationRecordKey::new("validation-original").unwrap(),
        PrValidationTarget::new("synthetic/repository", 1).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new("a".repeat(40)).unwrap(),
            PrValidationCommitSha::new("b".repeat(40)).unwrap(),
        ),
    );
    // A legacy persisted contract differs from the default and must survive a new successor.
    let mut encoded = serde_json::to_value(record).unwrap();
    encoded
        .as_object_mut()
        .unwrap()
        .remove("post_merge_validation_contract");
    let record: PrValidationRecord = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        record.post_merge_validation_contract(),
        &PostMergeValidationContract::legacy_ci_gate_v1()
    );
    let now = DateTime::parse_from_rfc3339("2026-11-21T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let attestation = IntegrationAttestation::new(
        IntegrationMethod::GithubRebaseMerge,
        PrValidationCommitSha::new("a".repeat(40)).unwrap(),
        Some(PrValidationCommitSha::new("b".repeat(40)).unwrap()),
        PrValidationCommitSha::new("c".repeat(40)).unwrap(),
        Some(1),
        Some(PrValidationCommitSha::new("c".repeat(40)).unwrap()),
        now,
        now + chrono::Duration::seconds(1),
    )
    .unwrap();
    let integrated = record
        .transition(PrValidationEvent::IntegrationAttested(attestation))
        .unwrap()
        .transition(PrValidationEvent::ObservationCheckpointed {
            delivery_revision: 99,
            cursor: Some("post-merge-cursor".into()),
            evidence_fingerprint: "post-merge-evidence".into(),
        })
        .unwrap();
    assert_eq!(integrated.phase(), PrValidationPhase::PostMergeObservation);
    assert!(integrated.has_post_merge_checkpoint());
    integrated
        .transition(PrValidationEvent::Fail(reason))
        .unwrap()
}

#[test]
fn recovery_resolves_130_successors_and_rejects_a_cycle_to_the_first_record() {
    const CHAIN_LENGTH: usize = 130;
    let (workspace, original) = setup(
        "recovery-long-chain",
        PrValidationTerminalReason::ObservationFailed,
    );
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let before = snapshot(&workspace);
    let mut current = original.clone();
    for round in 0..CHAIN_LENGTH {
        let order = stored(&workspace, "order-a");
        let command_id = format!("recover-chain-{round}");
        let terminal_id = format!("terminal-chain-{round}");
        let mut recovery = request(&current, &order, before.planning_revision);
        recovery.command_id = &command_id;
        recovery.terminal_record_id = &terminal_id;
        let receipt = adapter
            .recover_work_order_validation(&workspace, recovery)
            .unwrap();
        let successor = load(&workspace, &registered(&receipt));
        let failed = successor
            .transition(PrValidationEvent::BeginPreMergeObservation)
            .unwrap()
            .transition(PrValidationEvent::ObservationCheckpointed {
                delivery_revision: 1,
                cursor: None,
                evidence_fingerprint: format!("chain-evidence-{round}"),
            })
            .unwrap()
            .transition(PrValidationEvent::Fail(
                PrValidationTerminalReason::ObservationFailed,
            ))
            .unwrap();
        assert!(
            adapter
                .compare_and_swap_runtime_pr_validation_record(
                    &workspace,
                    failed.key(),
                    Some(&successor),
                    Some(&failed),
                )
                .unwrap()
        );
        current = failed;
    }

    assert_eq!(
        adapter
            .resolve_runtime_pr_validation_successor_key(&workspace, original.key())
            .unwrap(),
        *current.key()
    );
    assert_eq!(
        adapter
            .load_work_order_validation_recovery_origin(&workspace, current.key().as_str())
            .unwrap()
            .as_deref(),
        Some(original.key().as_str())
    );
    let db = connection(&workspace);
    let wrong_origins: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM planning_work_order_validation_recoveries
             WHERE remediation_origin_record_key<>?1",
            [original.key().as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(wrong_origins, 0);
    assert_eq!(
        count(&workspace, "planning_work_order_validation_recoveries"),
        CHAIN_LENGTH as i64
    );
    assert_eq!(
        count(&workspace, "runtime_pr_validation_records"),
        CHAIN_LENGTH as i64 + 1
    );
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_order_started_tasks"), 0);
    assert_eq!(load(&workspace, original.key().as_str()), original);

    let last_command = format!("recover-chain-{}", CHAIN_LENGTH - 1);
    let encoded: String = db
        .query_row(
            "SELECT receipt_json FROM planning_work_order_validation_recoveries
             WHERE command_id=?1",
            [&last_command],
            |row| row.get(0),
        )
        .unwrap();
    let mut forged_receipt: PlanningValidationRecoveryReceipt =
        serde_json::from_str(&encoded).unwrap();
    forged_receipt.outcome = PlanningValidationRecoveryOutcome::Registered {
        validation_record_key: original.key().as_str().to_owned(),
    };
    assert_eq!(
        db.execute(
            "UPDATE planning_work_order_validation_recoveries
             SET successor_record_key=?1,receipt_json=?2 WHERE command_id=?3",
            rusqlite::params![
                original.key().as_str(),
                serde_json::to_string(&forged_receipt).unwrap(),
                last_command,
            ],
        )
        .unwrap(),
        1
    );
    let error = adapter
        .resolve_runtime_pr_validation_successor_key(&workspace, original.key())
        .expect_err("a recovery cycle must not resolve to a usable successor");
    assert!(
        error.to_string().contains("successor chain is cyclic"),
        "{error:#}"
    );
}

#[test]
fn v19_reopening_adds_recovery_lookup_index_and_preserves_existing_authority_facts() {
    let (workspace, original) = setup(
        "recovery-v19-migration",
        PrValidationTerminalReason::ObservationFailed,
    );
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let control_receipt = adapter
        .apply_work_order_control(
            &workspace,
            "order-a",
            "v19-hold",
            PlanningWorkControlAction::Hold,
        )
        .unwrap();
    let before = snapshot(&workspace);
    let order_before = stored(&workspace, "order-a");
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace)
            .unwrap();
    let receipt_json: String = connection(&workspace)
        .query_row(
            "SELECT result_json FROM planning_work_order_control_commands WHERE command_id='v19-hold'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    // Reconstruct the v19 schema: both v20-only tables and their indexes are absent,
    // rather than changing only the version on a database that still has v20 objects.
    {
        let mut db = connection(&workspace);
        let tx = db.transaction().unwrap();
        tx.execute_batch(
            "DROP TABLE runtime_worker_continuations;
             DROP TABLE planning_work_order_validation_recoveries;
             UPDATE authority_metadata SET value='19' WHERE key='schema_version';",
        )
        .unwrap();
        tx.commit().unwrap();
    }
    {
        let old = Connection::open_with_flags(
            &location.authority_store_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let version: String = old
            .query_row(
                "SELECT value FROM authority_metadata WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "19");
        for table in [
            "runtime_worker_continuations",
            "planning_work_order_validation_recoveries",
        ] {
            assert!(!table_exists(&old, table).unwrap());
        }
        let index_count: i64 = old
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_work_order_validation_recovery_original'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 0);
        let original_json: String = old
            .query_row(
                "SELECT content FROM runtime_pr_validation_records WHERE record_key=?1",
                [original.key().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<PrValidationRecord>(&original_json).unwrap(),
            original
        );
        let prior_receipt: String = old
            .query_row(
                "SELECT result_json FROM planning_work_order_control_commands WHERE command_id='v19-hold'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(prior_receipt, receipt_json);
    }

    let reopened = connection(&workspace);
    let version: String = reopened
        .query_row(
            "SELECT value FROM authority_metadata WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, AUTHORITY_STORE_SCHEMA_VERSION.to_string());
    for table in [
        "runtime_worker_continuations",
        "planning_work_order_validation_recoveries",
    ] {
        assert!(table_exists(&reopened, table).unwrap());
    }
    let columns = reopened
        .prepare(
            "SELECT name FROM pragma_index_info('idx_work_order_validation_recovery_original')
             ORDER BY seqno",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(columns, vec!["original_record_key".to_owned()]);
    let after_receipt: String = reopened
        .query_row(
            "SELECT result_json FROM planning_work_order_control_commands WHERE command_id='v19-hold'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after_receipt, receipt_json);
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(stored(&workspace, "order-a"), order_before);
    assert_eq!(load(&workspace, original.key().as_str()), original);
    assert_eq!(
        adapter
            .apply_work_order_control(
                &workspace,
                "order-a",
                "v19-hold",
                PlanningWorkControlAction::Hold,
            )
            .unwrap(),
        control_receipt
    );
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_order_control_commands"), 1);
    assert_eq!(count(&workspace, "runtime_pr_validation_records"), 1);
    assert_eq!(
        count(&workspace, "planning_work_order_validation_recoveries"),
        0
    );
    assert_eq!(count(&workspace, "planning_work_order_started_tasks"), 0);
}

#[test]
fn integrated_observation_recovery_requires_zero_execution_cap_and_preserves_attestation() {
    let original = integrated_failure(PrValidationTerminalReason::ObservationFailed);
    let (workspace, original) = setup_with_record("recovery-post-merge", original);
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .set_work_order_execution_limit(&workspace, "order-a", 1)
        .unwrap();
    let before = snapshot(&workspace);
    let order = stored(&workspace, "order-a");
    let blocked = adapter
        .recover_work_order_validation(
            &workspace,
            request(&original, &order, before.planning_revision),
        )
        .unwrap();
    assert_eq!(
        blocked.outcome,
        PlanningValidationRecoveryOutcome::Blocked {
            reason: "integrated_execution_limit_changed".into()
        }
    );
    assert_eq!(count(&workspace, "runtime_pr_validation_records"), 1);
    assert_eq!(
        count(&workspace, "planning_work_order_validation_recoveries"),
        0
    );
    assert_eq!(count(&workspace, "planning_work_order_started_tasks"), 0);
    assert_eq!(snapshot(&workspace), before);

    adapter
        .set_work_order_execution_limit(&workspace, "order-a", 0)
        .unwrap();
    let mut recovery = request(&original, &order, before.planning_revision);
    recovery.command_id = "observe-only-recovery";
    let receipt = adapter
        .recover_work_order_validation(&workspace, recovery)
        .unwrap();
    let successor = load(&workspace, &registered(&receipt));
    assert_eq!(successor.phase(), PrValidationPhase::PostMergeObservation);
    assert_eq!(successor.target(), original.target());
    assert_eq!(successor.target_shas(), original.target_shas());
    assert_eq!(
        successor.integration_attestation(),
        original.integration_attestation()
    );
    assert_eq!(successor.merge_sha(), original.merge_sha());
    assert_eq!(successor.evidence_sha(), original.evidence_sha());
    assert_eq!(
        successor.post_merge_validation_contract(),
        &PostMergeValidationContract::legacy_ci_gate_v1()
    );
    assert_eq!(successor.observation_revision(), 0);
    assert_eq!(successor.observation_cursor(), None);
    assert_eq!(successor.evidence_fingerprint(), None);
    assert!(!successor.has_post_merge_checkpoint());
    assert!(successor.terminal_reason().is_none());
    let indexed: (String, String, String, String) = connection(&workspace)
        .query_row(
            "SELECT integration_method,integration_source_sha,integration_evidence_sha,
                    integration_remote_verified_at
             FROM runtime_pr_validation_records WHERE record_key=?1",
            [successor.key().as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    let attestation = original.integration_attestation().unwrap();
    assert_eq!(
        indexed,
        (
            attestation.method().label().to_owned(),
            attestation.source_sha().as_str().to_owned(),
            attestation.evidence_sha().as_str().to_owned(),
            attestation.remote_verified_at().to_rfc3339(),
        )
    );
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(stored(&workspace, "order-a"), order);
    assert_eq!(count(&workspace, "planning_work_order_started_tasks"), 0);
    assert_eq!(count(&workspace, "planning_work_order_task_admissions"), 0);
    assert_eq!(load(&workspace, original.key().as_str()), original);
    assert_eq!(
        adapter
            .load_work_order_validation_recovery_origin(&workspace, successor.key().as_str())
            .unwrap()
            .as_deref(),
        Some(original.key().as_str())
    );
}

#[test]
fn integrated_queue_recovery_is_blocked_even_with_zero_execution_cap() {
    for cap in [0, 1] {
        let original = integrated_failure(PrValidationTerminalReason::RemediationAdmissionFailed);
        let (workspace, original) =
            setup_with_record(&format!("recovery-integrated-queue-{cap}"), original);
        let adapter = SqlitePlanningAuthorityAdapter::new();
        adapter
            .set_work_order_execution_limit(&workspace, "order-a", cap)
            .unwrap();
        let before = snapshot(&workspace);
        let order = stored(&workspace, "order-a");
        let recovery = request(&original, &order, before.planning_revision);
        assert_eq!(recovery.kind, PlanningValidationRecoveryKind::QueueRecovery);
        assert_eq!(
            adapter
                .recover_work_order_validation(&workspace, recovery)
                .unwrap()
                .outcome,
            PlanningValidationRecoveryOutcome::Blocked {
                reason: "integrated_queue_recovery_blocked".into()
            }
        );
        assert_eq!(snapshot(&workspace), before);
        assert_eq!(stored(&workspace, "order-a"), order);
        assert_eq!(load(&workspace, original.key().as_str()), original);
        assert_eq!(count(&workspace, "runtime_pr_validation_records"), 1);
        assert_eq!(
            count(&workspace, "planning_work_order_validation_recoveries"),
            0
        );
        assert_eq!(count(&workspace, "planning_work_order_started_tasks"), 0);
        assert_eq!(count(&workspace, "planning_work_order_task_admissions"), 0);
    }
}
