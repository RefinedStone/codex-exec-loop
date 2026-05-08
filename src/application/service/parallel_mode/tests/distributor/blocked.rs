use super::*;

// push는 가능하지만 `gh` 실행과 인증이 degraded인 상태를 만든다. distributor는
// branch push까지는 진행하되 PR 자동화를 이어갈 수 없으므로 queue head를
// blocked로 남기고, slot lease와 session history를 실패 상태로 보존해야 한다.
#[test]
fn distributor_queue_blocks_after_push_when_github_automation_is_unavailable() {
    let repo = TempGitRepo::new("distributor-queue-gh-blocked");
    let github = FakeGithubAutomationPort::with_capabilities(GithubAutomationCapabilities::new(
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Ready,
            "test push remote ready",
            None,
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Degraded,
            "gh is missing in this test",
            Some("install gh".to_string()),
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhAuth,
            ParallelModeCapabilityState::Degraded,
            "gh auth cannot run without gh",
            Some("restore gh".to_string()),
        ),
    ));
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
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
            "turn-gh-blocked",
            None,
            Some("Distributor queue wiring completed."),
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
        .expect("commit-ready result should be enqueued")
        .expect("queue item should be created");
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("GitHub automation is unavailable"))
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![format!("push:{}:false", lease.branch_name)]
    );
    let queue_records = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root());
    assert_eq!(queue_records.len(), 1);
    assert_eq!(
        queue_records[0].queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_records[0]
            .integration_note
            .contains("GitHub automation is unavailable")
    );
    assert!(repo.slot_lease_path(1).exists());
    let detail = read_agent_session_detail_record(
        &test_parallel_runtime(),
        &repo.pool_root(),
        &lease_session_key(&lease),
    )
    .expect("session detail should be persisted");
    assert_eq!(detail.state_label, "failed");
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "assigned",
            "running",
            "reported_complete",
            "ledger_refreshing",
            "commit_ready",
            "merge_queued",
            "pushing",
            "failed"
        ]
    );
}

// source branch push가 실패하면 PR ensure를 실행하지 않아야 한다. 빈 원격 branch에 대한
// 빈/잘못된 PR을 만들면 operator가 실제 실패 원인을 GitHub 표면에서 추적하기 어려워진다.
#[test]
fn distributor_blocks_source_push_rejection_without_ensuring_pull_request() {
    let repo = TempGitRepo::new("distributor-source-push-rejection");
    let github = FakeGithubAutomationPort::with_source_push_error("remote rejected signed commit");
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
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
            "turn-source-push-rejected",
            None,
            Some("Distributor source push rejection completed."),
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
        .expect("commit-ready result should be enqueued")
        .expect("queue item should be created");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(notices.iter().any(|notice| {
        notice.contains("could not be pushed") && notice.contains("remote rejected signed commit")
    }));
    let operations = operations
        .lock()
        .expect("fake github operations mutex poisoned")
        .clone();
    assert_eq!(
        operations,
        vec![format!("push:{}:false", lease.branch_name)]
    );
    assert!(
        operations
            .iter()
            .all(|operation| !operation.starts_with("ensure-pr:")),
        "PR ensure must not run after source push failure: {operations:?}"
    );
    let queue_records = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root());
    assert_eq!(queue_records.len(), 1);
    assert_eq!(
        queue_records[0].queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_records[0]
            .integration_note
            .contains("remote rejected signed commit")
    );
}

// blocked queue head가 clean한 slot worktree branch mismatch 때문에 멈춘 경우,
// operator가 slot을 prerelease 기반으로 되돌려 놓으면 distributor가 같은 queue
// record를 다시 처리할 수 있어야 한다. 핵심은 새 queue item을 만들지 않고 기존
// recovery_note에 "복구된 retry" 이력을 남긴 채 Done으로 전이하는 것이다.
#[test]
fn distributor_retries_blocked_head_after_clean_slot_branch_recovery() {
    let repo = TempGitRepo::new("distributor-recovers-mismatched-slot-branch");
    let github = FakeGithubAutomationPort::ready();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
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
            "turn-queue-recovery",
            None,
            Some("Distributor queue recovery completed."),
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

    run_git(
        &slot_path,
        &[
            "checkout",
            "-B",
            "akra-agent/slot-1/stale-clean-checkout",
            "prerelease",
        ],
    );
    let mut record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    record.queue_state = ParallelModeQueueItemState::Blocked;
    record.integration_state = "blocked".to_string();
    record.integration_note = "slot worktree branch mismatch".to_string();
    record.recovery_note = Some("slot worktree branch mismatch".to_string());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &record,
    )
    .expect("blocked queue record should be stored");

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should recover and process");

    assert!(notices.iter().any(|notice| {
        notice.contains("distributor integrated queue head into prerelease")
            || notice.contains("distributor returned slot to idle")
    }));
    assert_eq!(current_branch(&slot_path), "HEAD");
    let recovered_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queue record should remain available");
    assert_eq!(
        recovered_record.queue_state,
        ParallelModeQueueItemState::Done
    );
    assert!(
        recovered_record
            .recovery_note
            .as_deref()
            .is_some_and(|note| note.contains("recovered mismatched clean slot"))
    );
}

// PR ensure 실패는 GitHub API의 일시적 조건이나 base branch 문제 때문에 operator
// recovery 후 재시도 가능한 blocked 상태가 된다. 이 테스트는 block 사유가 PR
// ensure 계열이면 다음 distributor 실행에서 통합 경로로 재진입하는지 고정한다.
#[test]
fn distributor_retries_blocked_pull_request_ensure_after_recovery() {
    let repo = TempGitRepo::new("distributor-recovers-pr-ensure-block");
    let github = FakeGithubAutomationPort::ready();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
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
            "turn-pr-ensure-recovery",
            None,
            Some("Distributor PR recovery completed."),
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
    let mut record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    record.queue_state = ParallelModeQueueItemState::Blocked;
    record.integration_state = "blocked".to_string();
    record.integration_note =
        "pull request ensure failed for `akra-agent/slot-1/task-one`: base invalid".to_string();
    record.recovery_note = Some(record.integration_note.clone());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &record,
    )
    .expect("blocked queue record should be stored");

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should recover and process");

    assert!(notices.iter().any(|notice| {
        notice.contains("distributor integrated queue head into prerelease")
            || notice.contains("distributor returned slot to idle")
    }));
    let recovered_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queue record should remain available");
    assert_eq!(
        recovered_record.queue_state,
        ParallelModeQueueItemState::Done
    );
    assert!(
        recovered_record
            .recovery_note
            .as_deref()
            .is_some_and(|note| note.contains("retryable distributor block"))
    );
}

// source branch push 실패도 네트워크/auth 같은 일시 조건 때문에 생길 수 있다. 조건이 복구된
// 다음 tick에서는 같은 queue item을 retryable block으로 보고 delivery를 다시 진행해야 한다.
#[test]
fn distributor_retries_blocked_source_branch_push_after_recovery() {
    let repo = TempGitRepo::new("distributor-recovers-source-push-block");
    let github = FakeGithubAutomationPort::ready();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
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
            "turn-source-push-recovery",
            None,
            Some("Distributor source push recovery completed."),
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
    let mut record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    record.queue_state = ParallelModeQueueItemState::Blocked;
    record.integration_state = "blocked".to_string();
    record.integration_note = format!(
        "source branch `{}` could not be pushed to `origin`: temporary remote failure",
        lease.branch_name
    );
    record.recovery_note = Some(record.integration_note.clone());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &record,
    )
    .expect("blocked queue record should be stored");

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should recover and process");

    assert!(notices.iter().any(|notice| {
        notice.contains("distributor integrated queue head into prerelease")
            || notice.contains("distributor returned slot to idle")
    }));
    let recovered_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queue record should remain available");
    assert_eq!(
        recovered_record.queue_state,
        ParallelModeQueueItemState::Done
    );
    assert!(
        recovered_record
            .recovery_note
            .as_deref()
            .is_some_and(|note| note.contains("retryable distributor block"))
    );
}

// worker branch의 source commit이 이미 prerelease에 patch-equivalent 형태로 들어간
// 경우에는 SHA가 달라도 같은 변경을 다시 merge하려 하면 안 된다. distributor는
// patch-id 동등성을 통합 완료로 취급하고 slot을 idle로 반환해야 한다.
#[test]
fn distributor_treats_patch_equivalent_source_commit_as_integrated() {
    let repo = TempGitRepo::new("distributor-patch-equivalent");
    let github = FakeGithubAutomationPort::ready();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
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
            "turn-patch-equivalent",
            None,
            Some("Patch equivalent delivery completed."),
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

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    fs::write(repo.repo_root.join("feature.txt"), "done\n")
        .expect("equivalent feature file should be written");
    run_git(&repo.repo_root, &["add", "feature.txt"]);
    run_git(&repo.repo_root, &["commit", "-qm", "equivalent work"]);
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(notices.iter().any(|notice| {
        notice.contains("distributor integrated queue head into prerelease")
            || notice.contains("distributor returned slot to idle")
    }));
    let recovered_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queue record should remain available");
    assert_eq!(
        recovered_record.queue_state,
        ParallelModeQueueItemState::Done
    );
    assert!(
        recovered_record
            .integration_note
            .contains("slot returned to idle")
    );
}
