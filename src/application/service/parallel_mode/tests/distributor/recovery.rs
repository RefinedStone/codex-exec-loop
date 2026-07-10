use super::super::*;

// supervisor snapshot은 관찰 전용이어야 한다. queue head가 merge-pending 상태여도
// snapshot 렌더링 과정에서 GitHub inspect, push, recovery 같은 runtime 작업이
// 실행되면 TUI 조회만으로 상태가 바뀌므로, fake GitHub 호출이 비어 있음을 확인한다.
#[test]
fn build_supervisor_snapshot_does_not_trigger_runtime_recovery_side_effects() {
    let repo = TempGitRepo::new("snapshot-no-recovery");
    let github = Arc::new(FakeGithubAutomationPort::ready());
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(github);
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(
        Path::new(&lease.worktree_path),
        "snapshot.txt",
        "snapshot result\n",
        "record snapshot result",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-snapshot",
            None,
            Some("Snapshot render should stay read-only."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: distributor delivery approved",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    let mut queue_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queued record should exist");
    queue_record.queue_state = ParallelModeQueueItemState::MergePending;
    queue_record.pull_request_number = Some(77);
    queue_record.pull_request_url =
        Some("https://github.com/RefinedStone/codex-exec-loop/pull/77".to_string());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &queue_record,
    )
    .expect("queue record should update");
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert_eq!(snapshot.distributor.head_summary, "merge pending");
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty(),
        "snapshot rendering should not invoke GitHub recovery work"
    );
}

#[test]
fn distributor_tick_recovers_commit_ready_result_missing_its_queue_record() {
    let repo = TempGitRepo::new("commit-ready-enqueue-recovery");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(
        Path::new(&lease.worktree_path),
        "recovered.txt",
        "recovered result\n",
        "record recoverable result",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-recover-enqueue",
            None,
            Some("The result is complete but its first queue handoff was interrupted."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded before queue persistence",
        )
        .expect("commit-ready should be recorded");
    assert!(
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty(),
        "the fixture must begin in the interrupted handoff state"
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("the next distributor tick should recover and deliver the result");

    assert!(
        notices
            .iter()
            .any(|notice| { notice.contains("distributor integrated queue head into prerelease") }),
        "recovered commit-ready result should be delivered in the same tick: {notices:?}"
    );
    let records = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "show",
                "refs/remotes/origin/prerelease:recovered.txt",
            ],
            None,
        )
        .as_deref(),
        Some("recovered result")
    );
}

#[test]
fn readiness_recovery_marks_stale_ledger_refreshing_session_for_manual_recovery() {
    let repo = TempGitRepo::new("stale-ledger-refreshing");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-stale-ledger",
            None,
            Some("Completed before the refresh worker disappeared."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");

    let session_key = lease.session_key();
    let mut detail =
        read_agent_session_detail_record(&test_parallel_runtime(), &repo.pool_root(), &session_key)
            .expect("session detail should be mirrored");
    detail.updated_at = "2020-01-01T00:00:00+00:00".to_string();
    if let Some(history) = detail.history.last_mut() {
        history.timestamp = detail.updated_at.clone();
    }
    SqlitePlanningAuthorityAdapter::upsert_runtime_session_detail(&repo.workspace_dir(), &detail)
        .expect("stale detail should update authority projection");
    fs::write(
        repo.session_detail_path(&session_key),
        serde_json::to_string_pretty(&detail).expect("detail should serialize"),
    )
    .expect("stale detail mirror should update");

    let readiness = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    assert!(readiness.allows_parallel_mode());
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    let recovered = snapshot
        .detail
        .session
        .expect("stale session should remain selected for operator recovery");
    assert_eq!(recovered.state_label, "official_refresh_recovery_needed");
    assert_eq!(
        recovered.latest_summary,
        "official completion refresh needs recovery"
    );
    assert!(
        recovered
            .authority_refresh_outcome
            .contains("stale official ledger refresh"),
        "{}",
        recovered.authority_refresh_outcome
    );
    assert_eq!(snapshot.distributor.head_summary, "recovery needed");
    assert_eq!(snapshot.roster.active_count(), 0);
}

#[test]
fn stale_official_refresh_recovery_abandons_only_the_head_order() {
    let repo = TempGitRepo::new("stale-official-refresh-single-order");
    let service = test_parallel_mode_service();
    let first_order =
        SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(&repo.workspace_dir())
            .expect("first refresh order should reserve");
    let second_order =
        SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(&repo.workspace_dir())
            .expect("second refresh order should reserve");
    assert_eq!(first_order, 1);
    assert_eq!(second_order, 2);

    let readiness = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    assert!(readiness.allows_parallel_mode());

    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            &repo.workspace_dir(),
            first_order,
            "first-worker",
        )
        .expect("first claim should be readable"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            &repo.workspace_dir(),
            second_order,
            "second-worker",
        )
        .expect("second claim should still be executable"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn passive_readiness_inspection_does_not_recover_or_mutate_refresh_orders() {
    let repo = TempGitRepo::new("passive-readiness-refresh-order");
    let service = test_parallel_mode_service();
    let refresh_order =
        SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(&repo.workspace_dir())
            .expect("refresh order should reserve");
    let runtime_projection =
        PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true);
    let planning_projection =
        PlanningApplicationProjection::from_runtime_projection(&runtime_projection);

    let readiness = service.inspect_readiness_passively_from_planning_projection(
        &repo.workspace_dir(),
        &planning_projection,
    );

    assert!(readiness.allows_parallel_mode());
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            &repo.workspace_dir(),
            refresh_order,
            "dashboard-reader",
        )
        .expect("refresh claim should remain readable"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired,
        "a dashboard/readiness GET must not run distributor recovery"
    );
}

// official completion refresh order는 worker가 실제 완료를 보고하는 순서와 별도로
// 예약된 순서를 따라야 한다. 늦게 시작한 completion이 먼저 보고되어도 feed와
// distributor queue가 stable ordering을 유지하도록 reservation 값을 보존한다.
#[test]
fn reserved_official_completion_orders_survive_out_of_order_worker_start() {
    let repo = TempGitRepo::new("official-completion-refresh-order");
    let service = test_parallel_mode_service();
    let first = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("first slot lease should be acquired");
    let second = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-2", "Task Two", "agent-2", "task-two"),
        )
        .expect("second slot lease should be acquired");
    for lease in [&first, &second] {
        service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should transition to running");
    }
    let first_order = service
        .reserve_workspace_official_completion_refresh_order(&first.worktree_path)
        .expect("first order reservation should succeed")
        .expect("first running slot should reserve an order");
    let second_order = service
        .reserve_workspace_official_completion_refresh_order(&second.worktree_path)
        .expect("second order reservation should succeed")
        .expect("second running slot should reserve an order");
    let second_report = service
        .begin_workspace_official_completion(
            &second.worktree_path,
            "turn-2",
            Some(second_order),
            Some("second completion finished"),
            Some("cargo test passed"),
            None,
        )
        .expect("second official completion should be captured")
        .expect("second report should be returned");
    let first_report = service
        .begin_workspace_official_completion(
            &first.worktree_path,
            "turn-1",
            Some(first_order),
            Some("first completion finished"),
            Some("cargo test passed"),
            None,
        )
        .expect("first official completion should be captured")
        .expect("first report should be returned");

    assert_eq!(first_report.refresh_order, 1);
    assert_eq!(second_report.refresh_order, 2);
}

// distributor queue는 head-of-line blocking 모델이다. 첫 번째 item이 source
// worktree 손실로 blocked되면 뒤 item이 준비되어 있어도 통합을 진행하지 않고
// held queue count와 blocked reason을 supervisor에 노출해야 한다.
#[test]
fn distributor_queue_keeps_later_item_queued_behind_blocked_head() {
    let repo = TempGitRepo::new("distributor-queue-blocked-head");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let first = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("first slot lease should be acquired");
    let second = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-2", "Task Two", "agent-2", "task-two"),
        )
        .expect("second slot lease should be acquired");
    for lease in [&first, &second] {
        let slot_path = PathBuf::from(lease.worktree_path.clone());
        service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should transition to running");
        repo.commit_file_in_slot(
            &slot_path,
            &format!("{}.txt", lease.task_id),
            "done\n",
            "agent work",
        );
        service
            .begin_workspace_official_completion(
                &lease.worktree_path,
                &format!("turn-{}", lease.task_id),
                None,
                Some("Distributor queue slice completed."),
                Some("cargo test passed"),
                None,
            )
            .expect("official completion should be captured");
        service
            .mark_workspace_official_completion_refreshing(&lease.worktree_path)
            .expect("ledger refreshing should be recorded");
        service
            .mark_workspace_commit_ready(
                &lease.worktree_path,
                "official ledger refresh succeeded: queued for delivery",
            )
            .expect("commit-ready should be recorded");
        service
            .enqueue_workspace_commit_ready_result(&lease.worktree_path)
            .expect("commit-ready result should enqueue")
            .expect("queue item should be present");
    }
    fs::remove_dir_all(&first.worktree_path).expect("first slot worktree should be removed");
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("processing the queue head should not crash");
    assert!(notices.iter().any(|notice| {
        notice.contains("distributor queue head blocked")
            || notice.contains("distributor queue head is blocked")
    }));
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(snapshot.distributor.head_summary, "blocked");
    assert_eq!(snapshot.distributor.queue_depth(), 2);
    assert_eq!(
        snapshot.distributor.orchestrator_status.queue_head,
        "agent-1 / task-1 / blocked"
    );
    assert_eq!(
        snapshot.distributor.orchestrator_status.barrier_state,
        "blocked"
    );
    assert_eq!(snapshot.distributor.orchestrator_status.held_queue_count, 1);
    assert!(
        snapshot
            .distributor
            .orchestrator_status
            .blocked_reason
            .as_deref()
            .expect("blocked reason should be surfaced")
            .contains("source worktree is missing")
    );
    assert!(
        snapshot
            .distributor
            .orchestrator_status
            .integration_worktree_readiness
            .contains("dedicated integration worktree"),
        "integration worktree readiness should be surfaced: {}",
        snapshot
            .distributor
            .orchestrator_status
            .integration_worktree_readiness
    );
    assert_eq!(
        snapshot.distributor.queue_items[0].queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert_eq!(
        snapshot.distributor.queue_items[1].queue_state,
        ParallelModeQueueItemState::Queued
    );
    assert!(
        snapshot
            .distributor
            .note
            .contains("source worktree is missing"),
        "queue note should explain the blocked head: {}",
        snapshot.distributor.note
    );
    assert!(
        snapshot
            .distributor
            .head_blocked_detail
            .as_deref()
            .expect("blocked head detail should be surfaced")
            .contains("source worktree is missing")
    );
}

// mirror 파일들이 모두 사라진 재시작 상황에서도 sqlite authority store의 queue
// record가 복구 기준이 된다. source worktree 자체가 없으면 자동 통합 대신 blocked
// record와 failed session detail을 재작성해 operator recovery로 넘긴다.
#[test]
fn distributor_recovery_blocks_missing_worktree_from_store_backed_queue_record() {
    let repo = TempGitRepo::new("distributor-store-recovery-missing-worktree");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-store-recovery",
            None,
            Some("Distributor recovery slice completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    let session_key = lease_session_key(&lease);
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist before mirror loss");
    fs::remove_file(repo.slot_lease_path(1)).expect("slot lease mirror should be removed");
    fs::remove_file(repo.session_detail_path(&session_key))
        .expect("session detail mirror should be removed");
    fs::remove_file(repo.distributor_queue_path(&queue_record.queue_item_id))
        .expect("queue mirror should be removed");
    fs::remove_dir_all(&lease.worktree_path).expect("source worktree should be removed");
    let recovered = test_parallel_mode_service();
    let notices = recovered
        .process_distributor_queue(&repo.workspace_dir())
        .expect("recovery should classify the missing worktree as blocked");
    assert!(
        notices.iter().any(|notice| {
            notice.contains("blocked") && notice.contains("recovered after restart")
        }),
        "recovery notice should explain the blocked head: {notices:?}"
    );
    let recovered_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("blocked queue record should be rewritten from the authority store");
    assert_eq!(
        recovered_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        recovered_record
            .integration_note
            .contains("recovered after restart: source worktree is missing")
    );
    let recovered_detail =
        read_agent_session_detail_record(&test_parallel_runtime(), &repo.pool_root(), &session_key)
            .expect("failed session detail should be rewritten from the authority store");
    assert_eq!(recovered_detail.state_label, "failed");
    assert!(
        recovered_detail
            .history
            .last()
            .expect("failure history entry should exist")
            .summary
            .contains("recovered after restart")
    );
}

#[test]
fn recovery_does_not_treat_a_reset_mutable_source_branch_as_frozen_result_integration() {
    let repo = TempGitRepo::new("distributor-recovery-reset-source-branch");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = super::blocked::enqueue_single_commit_ready_result(
        &service,
        &repo,
        "turn-reset-source-recovery",
    );
    let queued = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    let frozen_commit = queued.effective_source_commit_sha();

    run_git(
        Path::new(&lease.worktree_path),
        &["reset", "--hard", POOL_BASELINE_BRANCH],
    );
    assert!(
        command_succeeds(
            "git",
            [
                "-C",
                lease.worktree_path.as_str(),
                "cat-file",
                "-e",
                &format!("{frozen_commit}^{{commit}}"),
            ],
        ),
        "frozen result must still be addressable by its durable OID"
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("reset source branch should block without cleanup");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("branch head drifted from expected commit")),
        "frozen result loss risk should surface as head drift: {notices:?}"
    );
    let recovered = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should remain durable");
    assert_eq!(recovered.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(repo.branch_exists(&lease.branch_name));
    assert!(repo.slot_lease_path(1).exists());
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty(),
        "recovery must not push, create a PR, or clean a reset source branch"
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "show",
                "refs/remotes/origin/prerelease:feature.txt",
            ],
            None,
        ),
        None,
        "frozen result must not be falsely reported as integrated"
    );
}

// 반대로 queue head의 변경이 이미 `prerelease`에 fast-forward로 들어간 상태라면
// 재시작 후 snapshot은 이를 blocked가 아니라 cleaning으로 재분류해야 한다. mirror
// 손실 후에도 lease를 cleanup-pending으로 되살려 slot 회수 흐름을 이어간다.
#[test]
fn supervisor_snapshot_reclassifies_integrated_queue_head_from_store_backed_recovery() {
    let repo = TempGitRepo::new("supervisor-store-recovery-integrated");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-integrated-recovery",
            None,
            Some("Integrated queue recovery slice completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    let session_key = lease_session_key(&lease);
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    run_git(
        &repo.repo_root,
        &["merge", "--ff-only", lease.branch_name.as_str()],
    );
    run_git(
        &repo.repo_root,
        &["push", DEFAULT_PUSH_REMOTE_NAME, "prerelease"],
    );
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);
    assert!(
        command_succeeds(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "merge-base",
                "--is-ancestor",
                queue_record.effective_source_commit_sha().as_str(),
                "refs/remotes/origin/prerelease",
            ],
        ),
        "frozen source tip must be a literal ancestor of the recovery target"
    );
    let frozen_states =
        crate::application::service::parallel_mode::distributor::distributor_source_cherry_states(
            &repo.workspace_dir(),
            "refs/remotes/origin/prerelease",
            &queue_record,
        )
        .expect("frozen source range should remain classifiable for recovery");
    assert!(
        frozen_states.iter().all(|(_, equivalent)| *equivalent),
        "every frozen source commit should be equivalent in the recovery target: {frozen_states:?}"
    );
    fs::remove_file(repo.slot_lease_path(1)).expect("slot lease mirror should be removed");
    fs::remove_file(repo.session_detail_path(&session_key))
        .expect("session detail mirror should be removed");
    fs::remove_file(repo.distributor_queue_path(&queue_record.queue_item_id))
        .expect("queue mirror should be removed");
    let recovered = test_parallel_mode_service();
    let readiness = recovered.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    let snapshot =
        recovered.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert_eq!(snapshot.distributor.head_summary, "cleaning");
    assert_eq!(snapshot.distributor.queue_depth(), 1);
    assert_eq!(
        snapshot.distributor.queue_items[0].queue_state,
        ParallelModeQueueItemState::Cleaning
    );
    assert!(
        snapshot
            .distributor
            .note
            .contains("recovered after restart"),
        "snapshot should surface the recovery note: {}",
        snapshot.distributor.note
    );
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::CleanupPending
    );
}

// cleanup 실패로 blocked된 record라도 source branch가 이미 integration branch에
// 들어갔다면 재시작 복구는 사람 개입 상태로 고정하지 않고 slot 반환 단계로
// 다시 보내야 한다. 그렇지 않으면 통합이 끝난 작업이 blocked head로 계속 큐를 막는다.
#[test]
fn distributor_retries_blocked_cleanup_failure_after_integrated_recovery() {
    let repo = TempGitRepo::new("distributor-blocked-cleanup-recovery");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-blocked-cleanup-recovery",
            None,
            Some("Cleanup recovery slice completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    repo.merge_agent_slot_into_akra(&slot_path);
    assert!(
        command_succeeds(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "merge-base",
                "--is-ancestor",
                lease.branch_name.as_str(),
                POOL_BASELINE_BRANCH,
            ],
        ),
        "source branch should be integrated before cleanup recovery"
    );

    let mut cleanup_pending_lease = lease.clone();
    cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &cleanup_pending_lease,
    )
    .expect("cleanup pending lease should be persisted");

    let mut queue_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queue record should exist");
    queue_record.queue_state = ParallelModeQueueItemState::Blocked;
    queue_record.integration_state = "blocked".to_string();
    queue_record.integration_note =
        "slot `slot-1` cleanup failed after distributor delivery".to_string();
    queue_record.recovery_note = Some(queue_record.integration_note.clone());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &queue_record,
    )
    .expect("blocked cleanup queue record should be persisted");

    let recovered = test_parallel_mode_service();
    let notices = recovered
        .process_distributor_queue(&repo.workspace_dir())
        .expect("cleanup recovery should process");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let snapshot =
        recovered.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert!(
        !notices
            .iter()
            .any(|notice| notice.contains("distributor queue head is blocked")),
        "cleanup recovery should not leave a blocked head: {notices:?}"
    );
    assert_eq!(snapshot.distributor.head_summary, "idle");
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
}

#[test]
fn stale_queue_record_never_adopts_a_reused_slot_session() {
    let repo = TempGitRepo::new("recovery-stale-queue-reused-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-reused", "Reused slot", "agent-1", "reused-slot"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(&slot_path, "result.txt", "done\n", "stale session result");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-stale-session",
            None,
            Some("Stale session result."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(&lease.worktree_path, "stale session ready")
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    repo.merge_agent_slot_into_akra(&slot_path);

    let mut stale_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("stale queue record should exist");
    stale_record.queue_state = ParallelModeQueueItemState::Blocked;
    stale_record.integration_state = "blocked".to_string();
    stale_record.integration_note = "slot cleanup failed after distributor delivery".to_string();
    stale_record.recovery_note = Some(stale_record.integration_note.clone());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &stale_record,
    )
    .expect("stale queue record should persist");

    let mut replacement = lease.clone();
    replacement.leased_at = "2099-01-01T00:00:00Z".to_string();
    replacement.lease_generation =
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string());
    replacement.state = ParallelModeSlotLeaseState::Running;
    replacement.running_started_at = Some("2099-01-01T00:00:01Z".to_string());
    assert_ne!(lease.session_key(), replacement.session_key());
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &replacement,
    )
    .expect("replacement live lease should persist");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("stale queue recovery should fail closed");

    assert!(
        notices.iter().any(|notice| {
            notice.contains("stale distributor session")
                && notice.contains("live session")
                && notice.contains("is preserved")
        }),
        "session conflict should be explicit: {notices:?}"
    );
    let preserved = repo.read_slot_lease(1);
    assert_eq!(preserved.session_key(), replacement.session_key());
    assert_eq!(preserved.state, ParallelModeSlotLeaseState::Running);
    assert_eq!(current_branch(&slot_path), replacement.branch_name);
    let blocked = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("stale queue record should remain inspectable");
    assert_eq!(blocked.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(
        blocked
            .integration_note
            .contains("stale distributor session")
    );
}

#[test]
fn distributor_fetch_failure_after_remote_rewrite_preserves_the_only_local_result() {
    let repo = TempGitRepo::new("recovery-fetch-failure-preserves-local-result");
    let origin_path = repo.create_bare_origin_remote();
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-local", "Local Result", "agent-local", "local-result"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(
        &slot_path,
        "only-local.txt",
        "preserve me\n",
        "record the only local result",
    );
    let source_commit = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("source commit should resolve");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-local-result",
            None,
            Some("The only copy of the result is still local."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: preserve local result",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    let queue_before = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");

    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    run_git(&repo.repo_root, &["cherry-pick", source_commit.as_str()]);
    run_git(
        &repo.repo_root,
        &["commit", "--amend", "-qm", "stale equivalent integration"],
    );
    run_git(
        &repo.repo_root,
        &["push", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    let fetch_refspec = format!(
        "+refs/heads/{POOL_BASELINE_BRANCH}:{}",
        remote_standard_tracking_ref()
    );
    run_git(
        &repo.repo_root,
        &["fetch", DEFAULT_PUSH_REMOTE_NAME, fetch_refspec.as_str()],
    );
    let tracking_ref = remote_standard_tracking_ref();
    let stale_tracking_head = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "rev-parse",
            tracking_ref.as_str(),
        ],
        None,
    )
    .expect("stale remote-tracking head should resolve");
    assert_ne!(stale_tracking_head, queue_before.source_base_commit_sha);
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);

    let remote_integration_ref = local_standard_ref();
    run_git(
        &origin_path,
        &[
            "update-ref",
            remote_integration_ref.as_str(),
            queue_before.source_base_commit_sha.as_str(),
        ],
    );
    assert_eq!(
        run_command(
            "git",
            [
                "--git-dir",
                origin_path
                    .to_str()
                    .expect("origin path should be valid utf-8"),
                "show",
                &format!("{POOL_BASELINE_BRANCH}:only-local.txt"),
            ],
            None,
        ),
        None,
        "the rewritten remote integration branch must not contain the result"
    );
    let remote_source_ref = format!("refs/heads/{}", lease.branch_name);
    assert!(
        !command_succeeds(
            "git",
            [
                "--git-dir",
                origin_path
                    .to_str()
                    .expect("origin path should be valid utf-8"),
                "show-ref",
                "--verify",
                "--quiet",
                remote_source_ref.as_str(),
            ],
        ),
        "the remote source branch must already be absent"
    );

    let unavailable_remote = repo.root.join("unavailable-origin.git");
    run_git(
        &repo.repo_root,
        &[
            "remote",
            "set-url",
            DEFAULT_PUSH_REMOTE_NAME,
            unavailable_remote
                .to_str()
                .expect("unavailable remote path should be valid utf-8"),
        ],
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("remote rewrite should be durably blocked without following the new target");

    assert!(
        notices.iter().any(|notice| {
            notice.contains("immutable distributor delivery target drifted")
                && notice.contains("credential-redacted push URL changed")
        }),
        "remote rewrite must be rejected before any target fetch or cleanup: {notices:?}"
    );
    assert!(repo.branch_exists(&lease.branch_name));
    assert_eq!(current_branch(&slot_path), lease.branch_name);
    assert_eq!(
        run_command(
            "git",
            ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
            None,
        )
        .as_deref(),
        Some(source_commit.as_str())
    );
    assert_eq!(
        fs::read_to_string(slot_path.join("only-local.txt"))
            .expect("the local result file should survive"),
        "preserve me\n"
    );
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::Running
    );
    let queue_after = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should remain durable");
    assert_eq!(queue_after.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(
        queue_after
            .integration_note
            .contains("immutable distributor delivery target drifted"),
        "durable block should retain the immutable-target failure: {}",
        queue_after.integration_note
    );
    assert_eq!(queue_after.delivery_target, queue_before.delivery_target);
}
