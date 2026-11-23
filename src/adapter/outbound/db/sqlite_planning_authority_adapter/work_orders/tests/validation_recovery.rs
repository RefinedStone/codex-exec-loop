use std::sync::{Arc, Barrier};

use super::*;
use crate::adapter::outbound::filesystem::planning_workspace::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningValidationRecoveryKind, PlanningValidationRecoveryOutcome,
    PlanningValidationRecoveryReceipt, PlanningValidationRecoveryRequest,
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort, PlanningWorkPermitBasis,
    PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
use crate::application::port::outbound::pr_validation_remediation_port::{
    PrValidationRemediationKey, PrValidationRemediationPort, PrValidationRemediationRequest,
};
use crate::application::service::parallel_mode::PlanningQueuePrValidationRemediationPort;
use crate::application::service::planning::PlanningServices;
use crate::domain::parallel_mode::{
    PrValidationCommitSha, PrValidationEvent, PrValidationFinding, PrValidationFindingKey,
    PrValidationFindingSource, PrValidationPhase, PrValidationRecord, PrValidationRecordKey,
    PrValidationRemediationCorrelation, PrValidationTarget, PrValidationTargetShaSnapshot,
    PrValidationTerminalReason,
};
use sha2::{Digest, Sha256};

mod edge_cases;

fn basis(revision: u64) -> PlanningWorkPermitBasis {
    PlanningWorkPermitBasis {
        delivery_review: None,
        authorization_epoch: 0,
        policy_version: "policy".into(),
        field_hash: "field".into(),
        gate_revision: revision,
        source_sha: "a".repeat(40),
    }
}

fn finding() -> PrValidationFinding {
    PrValidationFinding::new(
        PrValidationFindingKey::new(PrValidationFindingSource::new("ci").unwrap(), "finding")
            .unwrap(),
        PrValidationCommitSha::new("a".repeat(40)).unwrap(),
        "Repair the configured check",
    )
    .unwrap()
}

fn failed_record(
    key: &str,
    revision: u64,
    reason: PrValidationTerminalReason,
) -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new(key).unwrap(),
        PrValidationTarget::new("synthetic/repository", 1).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new("a".repeat(40)).unwrap(),
            PrValidationCommitSha::new("b".repeat(40)).unwrap(),
        ),
    )
    .transition(PrValidationEvent::BeginPreMergeObservation)
    .unwrap()
    .transition(PrValidationEvent::ObservationCheckpointed {
        delivery_revision: revision,
        cursor: Some("original-cursor".into()),
        evidence_fingerprint: "original-evidence".into(),
    })
    .unwrap()
    .transition(PrValidationEvent::Fail(reason))
    .unwrap()
}

fn setup(label: &str, reason: PrValidationTerminalReason) -> (String, PrValidationRecord) {
    setup_with_record(label, failed_record("validation-original", 99, reason))
}

fn setup_with_record(label: &str, original: PrValidationRecord) -> (String, PrValidationRecord) {
    let workspace = workspace(label);
    seed(&workspace, vec![]);
    register_next(&workspace, "order-a", "original");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .set_work_order_permit_basis(&workspace, "order-a", &basis(1))
        .unwrap();
    assert!(
        adapter
            .compare_and_swap_runtime_pr_validation_record(
                &workspace,
                original.key(),
                None,
                Some(&original),
            )
            .unwrap()
    );
    adapter
        .link_work_order_validation_record(&workspace, original.key().as_str(), "original")
        .unwrap();
    (workspace, original)
}

fn request<'a>(
    original: &'a PrValidationRecord,
    order: &'a PlanningWorkOrderRecord,
    revision: i64,
) -> PlanningValidationRecoveryRequest<'a> {
    PlanningValidationRecoveryRequest {
        command_id: "recover-1",
        work_order_id: "order-a",
        terminal_record_id: "terminal-1",
        kind: if original.terminal_reason()
            == Some(&PrValidationTerminalReason::RemediationAdmissionFailed)
        {
            PlanningValidationRecoveryKind::QueueRecovery
        } else {
            PlanningValidationRecoveryKind::ObserveAgain
        },
        authorization_epoch: 0,
        gate_revision: 1,
        policy_version: "policy",
        original,
        observed_work_order: order,
        observed_planning_revision: revision,
        is_current_terminal: true,
    }
}

fn registered(receipt: &PlanningValidationRecoveryReceipt) -> String {
    match &receipt.outcome {
        PlanningValidationRecoveryOutcome::Registered {
            validation_record_key,
        } => validation_record_key.clone(),
        other => panic!("expected recovery registration, got {other:?}"),
    }
}

fn load(workspace: &str, key: &str) -> PrValidationRecord {
    SqlitePlanningAuthorityAdapter::new()
        .load_runtime_pr_validation_record(workspace, &PrValidationRecordKey::new(key).unwrap())
        .unwrap()
        .unwrap()
}

#[test]
fn completed_recovery_is_once_only_and_replays_after_control_or_planning_changes() {
    let (workspace, original) = setup(
        "recovery-replay",
        PrValidationTerminalReason::ObservationFailed,
    );
    let order = stored(&workspace, "order-a");
    let before = snapshot(&workspace);
    let request = request(&original, &order, before.planning_revision);
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let first = adapter
        .recover_work_order_validation(&workspace, request)
        .unwrap();
    let successor = registered(&first);
    assert_eq!(load(&workspace, original.key().as_str()), original);
    assert_eq!(
        load(&workspace, &successor).phase(),
        PrValidationPhase::Registered
    );
    assert_eq!(load(&workspace, &successor).observation_revision(), 0);
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_order_started_tasks"), 0);
    adapter
        .apply_work_order_control(
            &workspace,
            "order-a",
            "cancel-after-recovery",
            PlanningWorkControlAction::Cancel,
        )
        .unwrap();
    assert_eq!(
        SqlitePlanningAuthorityAdapter::new()
            .recover_work_order_validation(&workspace, request)
            .unwrap(),
        first,
    );
    let mut conflict = request;
    conflict.policy_version = "other-policy";
    assert!(
        adapter
            .recover_work_order_validation(&workspace, conflict)
            .is_err()
    );
    assert_eq!(
        count(&workspace, "planning_work_order_validation_recoveries"),
        1
    );
    assert_eq!(count(&workspace, "runtime_pr_validation_records"), 2);
    assert_eq!(count(&workspace, "planning_work_order_validation_links"), 2);
    assert_eq!(load(&workspace, original.key().as_str()), original);
}

#[test]
fn concurrent_recovery_claims_share_one_successor_and_receipt() {
    let (workspace, original) = setup(
        "recovery-concurrent",
        PrValidationTerminalReason::ObservationFailed,
    );
    let order = stored(&workspace, "order-a");
    let revision = snapshot(&workspace).planning_revision;
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let (workspace, original, order, barrier) = (
                workspace.clone(),
                original.clone(),
                order.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                barrier.wait();
                SqlitePlanningAuthorityAdapter::new()
                    .recover_work_order_validation(&workspace, request(&original, &order, revision))
                    .unwrap()
            })
        })
        .collect();
    let mut receipts = threads.into_iter().map(|thread| thread.join().unwrap());
    assert_eq!(receipts.next(), receipts.next());
    assert_eq!(
        count(&workspace, "planning_work_order_validation_recoveries"),
        1
    );
    assert_eq!(count(&workspace, "runtime_pr_validation_records"), 2);
}

#[test]
fn failed_receipt_write_rolls_back_successor_link_events_and_metadata() {
    let (workspace, original) = setup(
        "recovery-rollback",
        PrValidationTerminalReason::ObservationFailed,
    );
    let order = stored(&workspace, "order-a");
    let before = snapshot(&workspace);
    let metadata_before = metadata(&workspace);
    let event_count = count(&workspace, "runtime_events");
    connection(&workspace).execute_batch(
        "CREATE TRIGGER reject_recovery_receipt BEFORE INSERT ON planning_work_order_validation_recoveries
         BEGIN SELECT RAISE(ABORT,'synthetic receipt failure'); END;"
    ).unwrap();
    assert!(
        SqlitePlanningAuthorityAdapter::new()
            .recover_work_order_validation(
                &workspace,
                request(&original, &order, before.planning_revision),
            )
            .is_err()
    );
    assert_eq!(count(&workspace, "runtime_pr_validation_records"), 1);
    assert_eq!(count(&workspace, "planning_work_order_validation_links"), 1);
    assert_eq!(
        count(&workspace, "planning_work_order_validation_recoveries"),
        0
    );
    assert_eq!(count(&workspace, "runtime_events"), event_count);
    assert_eq!(metadata(&workspace), metadata_before);
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(load(&workspace, original.key().as_str()), original);
}

#[test]
fn recovery_never_bypasses_control_epoch_policy_gate_or_operator_pause() {
    for scenario in 0..10 {
        let (workspace, original) = setup(
            &format!("recovery-fence-{scenario}"),
            PrValidationTerminalReason::ObservationFailed,
        );
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let order = stored(&workspace, "order-a");
        let mut request = request(&original, &order, snapshot(&workspace).planning_revision);
        match scenario {
            0..=3 => {
                let action = [
                    PlanningWorkControlAction::Hold,
                    PlanningWorkControlAction::TakeOver,
                    PlanningWorkControlAction::Cancel,
                    PlanningWorkControlAction::Untrack,
                ][scenario];
                adapter
                    .apply_work_order_control(&workspace, "order-a", "control", action)
                    .unwrap();
            }
            4 => request.authorization_epoch = 1,
            5 => request.policy_version = "stale-policy",
            6 => request.gate_revision = 2,
            7 => {
                adapter
                    .revoke_work_order_permits(
                        &workspace,
                        "order-a",
                        &PlanningWorkPermitStage::ALL,
                        "revoked",
                    )
                    .unwrap();
            }
            8 => {
                connection(&workspace)
                    .execute(
                        "UPDATE runtime_pr_validation_records SET operator_paused=1",
                        [],
                    )
                    .unwrap();
            }
            _ => {
                connection(&workspace)
                    .execute(
                        "UPDATE runtime_pr_validation_records SET poll_lease_token='in-flight',
                 poll_lease_expires_at='2999-01-01T00:00:00+00:00'",
                        [],
                    )
                    .unwrap();
            }
        }
        let result = adapter
            .recover_work_order_validation(&workspace, request)
            .unwrap();
        assert!(
            matches!(
                result.outcome,
                PlanningValidationRecoveryOutcome::Blocked { .. }
            ),
            "scenario {scenario}"
        );
        assert_eq!(
            count(&workspace, "planning_work_order_validation_recoveries"),
            0
        );
        assert_eq!(count(&workspace, "runtime_pr_validation_records"), 1);
    }
}

#[test]
fn original_revision_or_current_execution_drift_requires_a_new_observation() {
    for scenario in 0..3 {
        let (workspace, original) = setup(
            &format!("recovery-stale-{scenario}"),
            PrValidationTerminalReason::ObservationFailed,
        );
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let order = stored(&workspace, "order-a");
        let before = snapshot(&workspace);
        let mut request = request(&original, &order, before.planning_revision);
        match scenario {
            0 => {
                let changed = failed_record(
                    original.key().as_str(),
                    100,
                    PrValidationTerminalReason::ObservationFailed,
                );
                assert!(
                    adapter
                        .compare_and_swap_runtime_pr_validation_record(
                            &workspace,
                            original.key(),
                            Some(&original),
                            Some(&changed),
                        )
                        .unwrap()
                );
            }
            1 => {
                let mut document = before.task_authority.clone();
                document.tasks[0].updated_at = "2026-11-21T10:00:00Z".into();
                commit_document(&workspace, Some(before.planning_revision), &document);
            }
            _ => request.is_current_terminal = false,
        }
        let receipt = adapter
            .recover_work_order_validation(&workspace, request)
            .unwrap();
        assert!(matches!(
            receipt.outcome,
            PlanningValidationRecoveryOutcome::Blocked { .. }
        ));
        assert_eq!(
            count(&workspace, "planning_work_order_validation_recoveries"),
            0
        );
    }
}

#[test]
fn successor_keeps_provider_pacing_without_reusing_the_original_poll_lease() {
    let (workspace, original) = setup(
        "recovery-pacing",
        PrValidationTerminalReason::ObservationFailed,
    );
    connection(&workspace)
        .execute(
            "UPDATE runtime_pr_validation_records SET next_poll_at='2999-02-01T00:00:00+00:00',
         last_polled_at='2026-11-21T00:00:00+00:00',poll_attempt=7,consecutive_error_count=3,
         last_error_class='transient',rate_limit_remaining=0,
         rate_limit_reset_at='2999-02-02T00:00:00+00:00',
         poll_lease_token='expired',poll_lease_expires_at='2000-01-01T00:00:00+00:00'",
            [],
        )
        .unwrap();
    let order = stored(&workspace, "order-a");
    let receipt = SqlitePlanningAuthorityAdapter::new()
        .recover_work_order_validation(
            &workspace,
            request(&original, &order, snapshot(&workspace).planning_revision),
        )
        .unwrap();
    let successor = registered(&receipt);
    let pacing: (String,i64,i64,i64,String,Option<String>) = connection(&workspace).query_row(
        "SELECT next_poll_at,poll_attempt,consecutive_error_count,rate_limit_remaining,
         rate_limit_reset_at,poll_lease_token FROM runtime_pr_validation_records WHERE record_key=?1",
        [&successor], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
    ).unwrap();
    assert_eq!(
        pacing,
        (
            "2999-02-01T00:00:00+00:00".into(),
            7,
            3,
            0,
            "2999-02-02T00:00:00+00:00".into(),
            None,
        )
    );
    assert_eq!(load(&workspace, original.key().as_str()), original);
}

#[test]
fn recovery_chain_keeps_its_first_origin_and_pr_lookup_selects_the_successor() {
    let (workspace, original) = setup(
        "recovery-chain",
        PrValidationTerminalReason::ObservationFailed,
    );
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let order = stored(&workspace, "order-a");
    let revision = snapshot(&workspace).planning_revision;
    let first = registered(
        &adapter
            .recover_work_order_validation(&workspace, request(&original, &order, revision))
            .unwrap(),
    );
    let prior = load(&workspace, &first);
    let failed = prior
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::ObservationCheckpointed {
            delivery_revision: 1,
            cursor: None,
            evidence_fingerprint: "next-round".into(),
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
                Some(&prior),
                Some(&failed),
            )
            .unwrap()
    );
    let mut next = request(&failed, &order, revision);
    next.command_id = "recover-2";
    next.terminal_record_id = "terminal-2";
    let latest = registered(
        &adapter
            .recover_work_order_validation(&workspace, next)
            .unwrap(),
    );
    assert_eq!(
        adapter
            .resolve_runtime_pr_validation_successor_key(&workspace, original.key())
            .unwrap()
            .as_str(),
        latest
    );
    assert_eq!(
        adapter
            .load_runtime_pr_validation_record_for_pr(&workspace, 1)
            .unwrap()
            .unwrap()
            .key()
            .as_str(),
        latest
    );
    assert_eq!(
        adapter
            .load_work_order_validation_recovery_origin(&workspace, &latest)
            .unwrap()
            .as_deref(),
        Some(original.key().as_str())
    );
    assert_eq!(load(&workspace, original.key().as_str()), original);
    assert_eq!(load(&workspace, &first), failed);
}

fn canonical_request(key: &str) -> PrValidationRemediationRequest {
    let finding = finding();
    let mut digest = Sha256::new();
    for part in [
        key,
        finding.target_sha().as_str(),
        finding.key().source().as_str(),
    ] {
        digest.update(part);
        digest.update([0]);
    }
    digest.update(finding.key().provider_event_id());
    PrValidationRemediationRequest {
        idempotency_key: PrValidationRemediationKey::new(format!(
            "pr-remediation-{:x}",
            digest.finalize()
        ))
        .unwrap(),
        validation_record_key: PrValidationRecordKey::new(key).unwrap(),
        target: PrValidationTarget::new("synthetic/repository", 1).unwrap(),
        target_sha: finding.target_sha().clone(),
        finding_key: finding.key().clone(),
        summary: finding.summary().to_owned(),
    }
}

#[test]
fn queue_recovery_reuses_an_admitted_task_even_when_the_original_ack_was_lost() {
    let (workspace, original) = setup(
        "recovery-queue-receipt",
        PrValidationTerminalReason::RemediationAdmissionFailed,
    );
    let adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning = PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        adapter.clone(),
        adapter.clone(),
        Arc::new(NoopPlanningWorkerPort),
    )
    .with_work_order_repository(adapter.clone());
    let port = PlanningQueuePrValidationRemediationPort::new(&planning.queue, &workspace);
    let admitted = port
        .request_remediation(&canonical_request(original.key().as_str()))
        .unwrap();
    // The task transaction committed, but the original validation record never got its ACK.
    assert!(original.remediation_for_task(admitted.as_str()).is_none());
    adapter
        .set_work_order_permit_basis(&workspace, "order-a", &basis(2))
        .unwrap();
    adapter
        .set_work_order_execution_limit(&workspace, "order-a", 0)
        .unwrap();
    let before = snapshot(&workspace);
    let order = stored(&workspace, "order-a");
    let mut recovery = request(&original, &order, before.planning_revision);
    recovery.gate_revision = 2;
    let successor = registered(
        &adapter
            .recover_work_order_validation(&workspace, recovery)
            .unwrap(),
    );
    for _ in 0..3 {
        assert_eq!(
            port.request_remediation(&canonical_request(&successor))
                .unwrap(),
            admitted
        );
    }
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_order_task_admissions"), 1);
    assert_eq!(count(&workspace, "planning_work_order_started_tasks"), 0);
    assert_eq!(
        stored(&workspace, "order-a").task_ids,
        ["original", admitted.as_str()]
    );
}

#[test]
fn copied_remediation_correlation_resolves_to_the_successor_and_keeps_task_state() {
    let (workspace, original) = setup(
        "recovery-existing-correlation",
        PrValidationTerminalReason::ObservationFailed,
    );
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let before = snapshot(&workspace);
    let mut document = candidate(&before, "existing-remediation");
    document.tasks.last_mut().unwrap().status = TaskStatus::AwaitingUser;
    commit_document(&workspace, Some(before.planning_revision), &document);
    adapter
        .link_supplemental_task(&workspace, "order-a", "existing-remediation")
        .unwrap();
    let finding = finding();
    let correlated = PrValidationRecord::register(
        original.key().clone(),
        original.target().clone(),
        original.target_shas().clone(),
    )
    .transition(PrValidationEvent::BeginPreMergeObservation)
    .unwrap()
    .transition(PrValidationEvent::FindingObserved(finding.clone()))
    .unwrap()
    .transition(PrValidationEvent::RemediationQueued(
        PrValidationRemediationCorrelation::new(
            finding.key().clone(),
            PrValidationRecordKey::new("existing-remediation").unwrap(),
        ),
    ))
    .unwrap()
    .transition(PrValidationEvent::Fail(
        PrValidationTerminalReason::ObservationFailed,
    ))
    .unwrap();
    assert!(
        adapter
            .compare_and_swap_runtime_pr_validation_record(
                &workspace,
                original.key(),
                Some(&original),
                Some(&correlated),
            )
            .unwrap()
    );
    adapter
        .set_work_order_permit_basis(&workspace, "order-a", &basis(2))
        .unwrap();
    let order = stored(&workspace, "order-a");
    let before = snapshot(&workspace);
    let mut recovery = request(&correlated, &order, before.planning_revision);
    recovery.gate_revision = 2;
    let successor = registered(
        &adapter
            .recover_work_order_validation(&workspace, recovery)
            .unwrap(),
    );
    let selected = adapter
        .load_runtime_pr_validation_record_for_remediation(&workspace, "existing-remediation")
        .unwrap()
        .unwrap();
    assert_eq!(selected.key().as_str(), successor);
    assert_eq!(selected.phase(), PrValidationPhase::RemediationRunning);
    assert_eq!(
        selected.remediation_for(finding.key()),
        correlated.remediation_for(finding.key())
    );
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(load(&workspace, original.key().as_str()), correlated);
}
