use super::*;
use crate::application::service::parallel_mode::distributor::install_before_work_order_permit_rearm_hook;

fn current_record(
    fixture: &WorkPermitDeliveryFixture,
    task_id: &str,
) -> PlanningAuthorityDistributorQueueRecord {
    SqlitePlanningAuthorityAdapter::load_runtime_projections(&fixture.repo.workspace_dir())
        .expect("authoritative queue should load")
        .distributor_queue_records
        .into_iter()
        .find(|record| record.task_id == task_id)
        .expect("the task should have a durable queue record")
}

fn mirror_record(
    workspace: &str,
    pool_root: &Path,
    record: &PlanningAuthorityDistributorQueueRecord,
) {
    let body = serde_json::to_string_pretty(record).expect("queue record should serialize");
    test_parallel_runtime()
        .write_runtime_mirror_atomic(
            pool_root,
            &Path::new(".distributor-queue").join(format!("{}.json", record.queue_item_id)),
            &body,
        )
        .unwrap_or_else(|error| panic!("queue mirror should persist for {workspace}: {error}"));
}

fn assert_record_preserved(
    fixture: &WorkPermitDeliveryFixture,
    expected: &PlanningAuthorityDistributorQueueRecord,
) {
    let current = current_record(fixture, &expected.task_id);
    let expected_value = serde_json::to_value(expected).expect("expected record should serialize");
    assert_eq!(
        serde_json::to_value(current).expect("current record should serialize"),
        expected_value,
        "a stale permit observer must not overwrite any authoritative queue field"
    );
    let mirror = fs::read_to_string(
        fixture
            .repo
            .pool_root()
            .join(".distributor-queue")
            .join(format!("{}.json", expected.queue_item_id)),
    )
    .expect("queue mirror should remain readable");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&mirror).expect("queue mirror should be valid"),
        expected_value,
        "the losing rearm must not replace the newer queue mirror"
    );
}

fn install_competing_rearm(
    fixture: &WorkPermitDeliveryFixture,
    observed: &PlanningAuthorityDistributorQueueRecord,
    replacement: &PlanningAuthorityDistributorQueueRecord,
) {
    let workspace = fixture.repo.workspace_dir();
    let pool_root = fixture.repo.pool_root();
    let observed = observed.clone();
    let replacement = replacement.clone();
    install_before_work_order_permit_rearm_hook(move || {
        assert!(
            SqlitePlanningAuthorityAdapter::compare_and_swap_runtime_distributor_queue_record(
                &workspace,
                &observed,
                &replacement,
            )
            .expect("the competing owner should update the actual SQLite row"),
            "the hook must win the same CAS that the stale observer will lose"
        );
        mirror_record(&workspace, &pool_root, &replacement);
    });
}

#[test]
fn work_order_permit_rearm_conflict_stops_selection_and_fresh_tick_resumes_head() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-rearm-selection-conflict",
        FakeGithubAutomationPort::ready(),
    );
    let waiting = fixture.process();
    fixture.assert_blocked("permit_missing", &waiting);
    let unrelated = enqueue_unrelated_delivery(&fixture.service, &fixture.repo);
    let observed = current_record(&fixture, TASK_ID);
    let following = current_record(&fixture, "task-2");
    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        false,
    );
    let mut competing = observed.clone();
    competing.queue_state = ParallelModeQueueItemState::Queued;
    competing.integration_note = "another distributor rearmed this queue head".to_string();
    competing.recovery_note = Some("retain the winning rearm evidence".to_string());
    install_competing_rearm(&fixture, &observed, &competing);
    let remote_before = remote_heads(&fixture.repo);
    let operations_before = fixture.operations();

    let error = fixture
        .service
        .process_distributor_queue(&fixture.repo.workspace_dir())
        .expect_err("a lost rearm CAS must stop selection instead of skipping the queue head");

    assert!(error.contains("distributor queue changed during work-order permit refresh"));
    assert_eq!(fixture.operations(), operations_before);
    assert_eq!(remote_heads(&fixture.repo), remote_before);
    assert_record_preserved(&fixture, &competing);
    assert_record_preserved(&fixture, &following);

    // A fresh invocation reads the competing owner's Queued row and can proceed;
    // the contention must not persist a new block or require another permit grant.
    let resumed = fixture.process();
    assert!(
        resumed
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head")),
        "the original queue head should resume from current authority: {resumed:?}"
    );
    assert_eq!(
        current_record(&fixture, TASK_ID).queue_state,
        ParallelModeQueueItemState::Done
    );
    assert_eq!(
        current_record(&fixture, "task-2").queue_state,
        ParallelModeQueueItemState::Queued
    );
    assert!(
        !fixture
            .operations()
            .iter()
            .any(|operation| { operation == &format!("push:{}:false", unrelated.branch_name) }),
        "neither the stale invocation nor its fresh retry may deliver the following task"
    );
}

#[test]
fn work_order_permit_rearm_conflict_stops_recovery_before_later_row_mutation() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-rearm-recovery-conflict",
        FakeGithubAutomationPort::ready(),
    );
    let waiting = fixture.process();
    fixture.assert_blocked("permit_missing", &waiting);
    enqueue_unrelated_delivery(&fixture.service, &fixture.repo);
    let observed = current_record(&fixture, TASK_ID);
    let mut following = current_record(&fixture, "task-2");
    // If recovery continues past the lost CAS, this invalid target is rewritten
    // into a durable Blocked row. It must remain untouched in the aborted pass.
    following.delivery_target = None;
    following.integration_note = "later row must wait for a fresh recovery snapshot".to_string();
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &fixture.repo.workspace_dir(),
        &following,
    )
    .expect("the following recovery candidate should persist");
    mirror_record(
        &fixture.repo.workspace_dir(),
        &fixture.repo.pool_root(),
        &following,
    );
    fixture.grant(&[PlanningWorkPermitStage::Push], false);
    let mut competing = observed.clone();
    competing.queue_state = ParallelModeQueueItemState::Pushing;
    competing.integration_note = "a concurrent owner already started delivery".to_string();
    competing.recovery_note = Some("retain the active owner's recovery evidence".to_string());
    install_competing_rearm(&fixture, &observed, &competing);
    let remote_before = remote_heads(&fixture.repo);

    let result = fixture
        .service
        .distributor_service
        .recover_runtime_state(&fixture.repo.workspace_dir());
    let Err(error) = result else {
        panic!("a lost rearm CAS must abort the entire stale recovery pass");
    };

    assert!(error.contains("distributor queue changed during work-order permit refresh"));
    assert_eq!(remote_heads(&fixture.repo), remote_before);
    assert_record_preserved(&fixture, &competing);
    assert_record_preserved(&fixture, &following);
    fixture.assert_no_integration_push();
}
