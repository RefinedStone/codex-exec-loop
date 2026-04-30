use super::*;

#[test]
fn process_distributor_queue_delivers_commit_ready_result_into_akra_and_cleans_slot() {
    let repo = TempGitRepo::new("distributor-queue-success");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
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
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-42")
        .expect("thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-queue-success",
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

    let queued_item = service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should be enqueued")
        .expect("queue item should be created");
    assert_eq!(queued_item.queue_state, ParallelModeQueueItemState::Queued);

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    fs::write(repo.repo_root.join("operator-note.tmp"), "local note\n")
        .expect("untracked integration note should be writable");
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head into prerelease"))
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor returned slot to idle"))
    );

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("cleaned session detail should remain available");

    assert_eq!(snapshot.roster.active_count(), 0);
    assert_eq!(snapshot.distributor.head_summary, "idle");
    assert!(
        snapshot.distributor.completion_feed[3]
            .summary
            .contains("prerelease"),
        "merge-queued feed should reflect distributor integration: {}",
        snapshot.distributor.completion_feed[3].summary
    );
    assert_eq!(
        snapshot.distributor.completion_feed[4].summary,
        "slot cleaned and returned to the idle pool"
    );
    assert_eq!(detail.state_label, "cleaned");
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "assigned",
            "starting",
            "running",
            "reported_complete",
            "ledger_refreshing",
            "commit_ready",
            "merge_queued",
            "pushing",
            "pr_pending",
            "merge_pending",
            "integrating",
            "merged",
            "cleanup_pending",
            "cleaned"
        ]
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            format!("ensure-pr:prerelease:{}", lease.branch_name),
            "inspect-pr:77".to_string(),
            "push-integration:prerelease".to_string(),
            "inspect-pr:77".to_string(),
            "close-pr:77".to_string(),
        ]
    );
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "show",
                "prerelease:feature.txt",
            ],
            None,
        )
        .as_deref(),
        Some("done")
    );
}

#[test]
fn process_distributor_queue_integrates_prerelease_based_lease_branch() {
    let repo = TempGitRepo::new("distributor-prerelease-based-branch");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    repo.commit_on_current_branch(
        "prerelease-only.txt",
        "current baseline\n",
        "advance prerelease before lease",
    );
    let prerelease_head = run_command(
        "git",
        [
            "-C",
            repo.repo_root.to_str().expect("repo root should be utf-8"),
            "rev-parse",
            "prerelease",
        ],
        None,
    )
    .expect("prerelease should resolve before lease");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    assert_eq!(
        run_command(
            "git",
            ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
            None,
        )
        .as_deref(),
        Some(prerelease_head.as_str())
    );

    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-prerelease-base")
        .expect("thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    let source_commit = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("source commit should resolve");
    let source_parent = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD^"],
        None,
    )
    .expect("source commit parent should resolve");
    assert_eq!(source_parent, prerelease_head);

    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-prerelease-based-branch",
            None,
            Some("Prerelease based distributor slice completed."),
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
            "official ledger refresh succeeded: prerelease based delivery approved",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should be enqueued")
        .expect("queue item should be created");

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
        "processing should integrate the prerelease-based branch: {notices:?}"
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "show",
                "prerelease:prerelease-only.txt",
            ],
            None,
        )
        .as_deref(),
        Some("current baseline")
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "show",
                "prerelease:feature.txt",
            ],
            None,
        )
        .as_deref(),
        Some("done")
    );
    let integrated_as_ancestor = command_succeeds(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "merge-base",
            "--is-ancestor",
            source_commit.as_str(),
            "prerelease",
        ],
    );
    let patch_equivalent = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "cherry",
            "prerelease",
            source_commit.as_str(),
        ],
        None,
    )
    .is_some_and(|output| output.starts_with("- "));
    assert!(
        integrated_as_ancestor || patch_equivalent,
        "source commit should be integrated or patch-equivalent in prerelease"
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            format!("ensure-pr:prerelease:{}", lease.branch_name),
            "inspect-pr:77".to_string(),
            "push-integration:prerelease".to_string(),
            "inspect-pr:77".to_string(),
            "close-pr:77".to_string(),
        ]
    );
}

#[test]
fn build_supervisor_snapshot_prefers_active_distributor_queue_head_for_selected_detail() {
    let repo = TempGitRepo::new("distributor-detail-selection");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let queued = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("queue-head slot lease should be acquired");
    let queued_slot_path = PathBuf::from(queued.worktree_path.clone());
    service
        .record_workspace_slot_thread_prepared(&queued.worktree_path, "thread-queue")
        .expect("queue-head thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&queued.worktree_path)
        .expect("queue-head slot should transition to running");
    repo.commit_file_in_slot(&queued_slot_path, "queued.txt", "done\n", "queue head work");
    service
        .begin_workspace_official_completion(
            &queued.worktree_path,
            "turn-queue-head",
            None,
            Some("Queued result is waiting for distributor delivery."),
            Some("cargo test passed"),
            None,
        )
        .expect("queue-head official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&queued.worktree_path)
        .expect("queue-head ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &queued.worktree_path,
            "official ledger refresh succeeded: distributor delivery approved",
        )
        .expect("queue-head commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&queued.worktree_path)
        .expect("queue-head result should enqueue")
        .expect("queue-head item should be created");

    thread::sleep(Duration::from_millis(10));

    let running = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-2", "Task Two", "agent-2", "task-two"),
        )
        .expect("second slot lease should be acquired");
    service
        .record_workspace_slot_thread_prepared(&running.worktree_path, "thread-running")
        .expect("second thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&running.worktree_path)
        .expect("second slot should transition to running");

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("selected detail should exist");

    assert_eq!(snapshot.distributor.head_summary, "queued");
    assert_eq!(snapshot.distributor.queue_depth(), 1);
    assert_eq!(snapshot.distributor.queue_items[0].source_agent, "agent-1");
    assert_eq!(detail.agent_id, "agent-1");
    assert_eq!(detail.task_id, "task-1");
    assert_eq!(detail.thread_id.as_deref(), Some("thread-queue"));
    assert_eq!(detail.state_label, "merge_queued");
    assert_eq!(
        detail.distributor_outcome.as_deref(),
        Some("distributor accepted the result and queued it for GitHub delivery")
    );
}

#[test]
fn build_supervisor_snapshot_populates_idle_orchestrator_without_session_detail() {
    let repo = TempGitRepo::new("distributor-empty-orchestrator");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert_eq!(snapshot.distributor.head_summary, "idle");
    assert_eq!(snapshot.distributor.orchestrator_status.queue_head, "none");
    assert_eq!(
        snapshot.distributor.orchestrator_status.barrier_state,
        "idle"
    );
    assert!(
        snapshot
            .distributor
            .orchestrator_status
            .integration_worktree_readiness
            .contains("prerelease"),
        "idle snapshot should still inspect integration readiness: {}",
        snapshot
            .distributor
            .orchestrator_status
            .integration_worktree_readiness
    );
}

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

    let queue_records = load_distributor_queue_records(&repo.pool_root());
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

    let detail = read_agent_session_detail_record(&repo.pool_root(), &lease_session_key(&lease))
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
    let mut record = load_distributor_queue_records(&repo.pool_root())
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
    let recovered_record = load_distributor_queue_records(&repo.pool_root())
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

    let mut record = load_distributor_queue_records(&repo.pool_root())
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
    let recovered_record = load_distributor_queue_records(&repo.pool_root())
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
    let recovered_record = load_distributor_queue_records(&repo.pool_root())
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

    let mut queue_record = load_distributor_queue_records(&repo.pool_root())
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
            .contains("prerelease"),
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
    let queue_record = load_distributor_queue_records(&repo.pool_root())
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

    let recovered_record = load_distributor_queue_records(&repo.pool_root())
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

    let recovered_detail = read_agent_session_detail_record(&repo.pool_root(), &session_key)
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
    let queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    run_git(
        &repo.repo_root,
        &["merge", "--ff-only", lease.branch_name.as_str()],
    );
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);
    fs::remove_file(repo.slot_lease_path(1)).expect("slot lease mirror should be removed");
    fs::remove_file(repo.session_detail_path(&session_key))
        .expect("session detail mirror should be removed");
    fs::remove_file(repo.distributor_queue_path(&queue_record.queue_item_id))
        .expect("queue mirror should be removed");

    let recovered = test_parallel_mode_service();
    let readiness = recovered.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeSnapshot::ready("prompt".into(), "queue".into(), None)
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

#[test]
fn distributor_snapshot_surfaces_rebase_provenance_for_blocked_head() {
    let repo = TempGitRepo::new("distributor-rebase-provenance");
    let service = test_parallel_mode_service_with_github(Arc::new(
        FakeGithubAutomationPort::with_force_push_error("force-with-lease rejected"),
    ));
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
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-rebase-provenance",
            None,
            Some("Distributor rebase provenance slice completed."),
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

    let original_queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    let original_commit_sha = original_queue_record.commit_sha;

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    fs::write(repo.repo_root.join("baseline.txt"), "baseline advanced\n")
        .expect("baseline file should be written");
    run_git(&repo.repo_root, &["add", "baseline.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "advance prerelease baseline"],
    );
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("processing the queue head should succeed");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
        "processing should surface the cherry-pick integration outcome: {notices:?}"
    );

    let queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(queue_record.queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(queue_record.commit_sha, original_commit_sha);
    assert_eq!(queue_record.integration_state, "done");

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(snapshot.distributor.head_summary, "idle");
    assert!(snapshot.distributor.head_rebase_provenance.is_none());
}

#[test]
fn distributor_queue_blocks_rebase_conflict_for_operator_recovery() {
    let repo = TempGitRepo::new("distributor-rebase-conflict");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    fs::write(repo.repo_root.join("conflict.txt"), "base\n")
        .expect("baseline conflict file should be written");
    run_git(&repo.repo_root, &["add", "conflict.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "seed conflict baseline"],
    );
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

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
    repo.commit_file_in_slot(
        &slot_path,
        "conflict.txt",
        "agent change\n",
        "agent updates conflict",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-rebase-conflict",
            None,
            Some("Distributor rebase conflict recovery slice completed."),
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

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    fs::write(repo.repo_root.join("conflict.txt"), "baseline change\n")
        .expect("advanced baseline conflict file should be written");
    run_git(&repo.repo_root, &["add", "conflict.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "advance conflicting prerelease baseline"],
    );
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("processing the queue head should succeed");
    assert!(
        notices.iter().any(|notice| {
            notice.contains("distributor queue head blocked")
                && notice.contains("could not cherry-pick into `prerelease` cleanly")
        }),
        "processing should surface the cherry-pick conflict block: {notices:?}"
    );

    let queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("blocked queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_record
            .integration_note
            .contains("could not cherry-pick into `prerelease` cleanly")
    );
    assert_eq!(queue_record.integration_state, "blocked");
    assert_eq!(queue_record.conflict_files, vec!["conflict.txt"]);

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(snapshot.distributor.head_summary, "blocked");
    assert_eq!(snapshot.distributor.queue_depth(), 1);
    assert_eq!(
        snapshot.distributor.orchestrator_status.queue_head,
        "agent-1 / task-1 / blocked"
    );
    assert_eq!(
        snapshot.distributor.orchestrator_status.barrier_state,
        "blocked"
    );
    assert_eq!(
        snapshot.distributor.orchestrator_status.conflict_files,
        vec!["conflict.txt"]
    );
    assert!(
        snapshot
            .distributor
            .orchestrator_status
            .blocked_reason
            .as_deref()
            .expect("blocked reason should be surfaced")
            .contains("could not cherry-pick into `prerelease` cleanly")
    );
    assert_eq!(
        snapshot.distributor.queue_items[0].queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        snapshot
            .distributor
            .head_blocked_detail
            .as_deref()
            .expect("blocked head detail should be surfaced")
            .contains("could not cherry-pick into `prerelease` cleanly")
    );
    assert!(
        snapshot.distributor.head_rebase_provenance.is_none(),
        "failed rebase should not report successful rebase provenance"
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            format!("ensure-pr:prerelease:{}", lease.branch_name),
            "inspect-pr:77".to_string(),
        ]
    );
}
