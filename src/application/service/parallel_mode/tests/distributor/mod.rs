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

mod recovery;
