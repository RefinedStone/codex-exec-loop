use super::super::*;

/*
이 테스트는 distributor가 queue head를 `prerelease` 위로 통합할 때 baseline이 이미 전진한 상황을
재현한다. agent commit은 원래 queue record의 commit_sha로 고정되어 있고, 통합은 cherry-pick으로
끝나므로 성공 뒤 queue record는 Done이지만 head rebase provenance는 남지 않아야 한다. force push
오류를 가진 fake GitHub port를 쓰는 이유는 push 경로의 실패가 이미 완료된 local integration
snapshot을 오염시키지 않는지 함께 확인하기 위해서다.
*/
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

/*
이 테스트는 같은 파일을 baseline과 agent branch가 각각 수정해 cherry-pick conflict가 나는 경로를
고정한다. 핵심은 conflict를 자동 해결하지 않고 queue record를 Blocked로 남기며, supervisor
snapshot의 head summary, barrier state, conflict_files, blocked detail이 모두 같은 원인을 가리키는지
검증하는 것이다. 마지막 fake GitHub operation 검증은 conflict 이전의 push/PR 확보 단계까지만
실행되었고 통합 이후 단계가 진행되지 않았음을 보여 준다.
*/
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
