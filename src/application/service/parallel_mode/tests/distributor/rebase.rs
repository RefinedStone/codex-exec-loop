// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::*;

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn distributor_snapshot_surfaces_rebase_provenance_for_blocked_head() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let repo = TempGitRepo::new("distributor-rebase-provenance");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let service = test_parallel_mode_service_with_github(Arc::new(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        FakeGithubAutomationPort::with_force_push_error("force-with-lease rejected"),
    ));
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let lease = service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("slot lease should be acquired");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .mark_workspace_slot_running(&lease.worktree_path)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-rebase-provenance",
            None,
            Some("Distributor rebase provenance slice completed."),
            Some("cargo test passed"),
            None,
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("official completion should be captured");
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("ledger refreshing should be recorded");
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("commit-ready should be recorded");
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("commit-ready result should enqueue")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("queue item should be created");

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let original_queue_record = load_distributor_queue_records(&repo.pool_root())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .next()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("queue record should exist");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let original_commit_sha = original_queue_record.commit_sha;

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    fs::write(repo.repo_root.join("baseline.txt"), "baseline advanced\n")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("baseline file should be written");
    run_git(&repo.repo_root, &["add", "baseline.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "advance prerelease baseline"],
    );
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let notices = service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .process_distributor_queue(&repo.workspace_dir())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("processing the queue head should succeed");
    assert!(
        notices
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .iter()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
        "processing should surface the cherry-pick integration outcome: {notices:?}"
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let queue_record = load_distributor_queue_records(&repo.pool_root())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .next()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("queue record should persist");
    assert_eq!(queue_record.queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(queue_record.commit_sha, original_commit_sha);
    assert_eq!(queue_record.integration_state, "done");

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(snapshot.distributor.head_summary, "idle");
    assert!(snapshot.distributor.head_rebase_provenance.is_none());
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn distributor_queue_blocks_rebase_conflict_for_operator_recovery() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let repo = TempGitRepo::new("distributor-rebase-conflict");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let github = FakeGithubAutomationPort::ready();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let operations = github.operations.clone();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    fs::write(repo.repo_root.join("conflict.txt"), "base\n")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("baseline conflict file should be written");
    run_git(&repo.repo_root, &["add", "conflict.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "seed conflict baseline"],
    );
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let lease = service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("slot lease should be acquired");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .mark_workspace_slot_running(&lease.worktree_path)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("slot should transition to running");
    repo.commit_file_in_slot(
        &slot_path,
        "conflict.txt",
        "agent change\n",
        "agent updates conflict",
    );
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-rebase-conflict",
            None,
            Some("Distributor rebase conflict recovery slice completed."),
            Some("cargo test passed"),
            None,
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("official completion should be captured");
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("ledger refreshing should be recorded");
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("commit-ready should be recorded");
    service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("commit-ready result should enqueue")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("queue item should be created");

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    fs::write(repo.repo_root.join("conflict.txt"), "baseline change\n")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("advanced baseline conflict file should be written");
    run_git(&repo.repo_root, &["add", "conflict.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "advance conflicting prerelease baseline"],
    );
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let notices = service
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .process_distributor_queue(&repo.workspace_dir())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("processing the queue head should succeed");
    assert!(
        notices.iter().any(|notice| {
            notice.contains("distributor queue head blocked")
                && notice.contains("could not cherry-pick into `prerelease` cleanly")
        }),
        "processing should surface the cherry-pick conflict block: {notices:?}"
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let queue_record = load_distributor_queue_records(&repo.pool_root())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .next()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .expect("blocked queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_record
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .integration_note
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .contains("could not cherry-pick into `prerelease` cleanly")
    );
    assert_eq!(queue_record.integration_state, "blocked");
    assert_eq!(queue_record.conflict_files, vec!["conflict.txt"]);

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .distributor
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .orchestrator_status
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .blocked_reason
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .as_deref()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .expect("blocked reason should be surfaced")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .contains("could not cherry-pick into `prerelease` cleanly")
    );
    assert_eq!(
        snapshot.distributor.queue_items[0].queue_state,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        snapshot
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .distributor
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .head_blocked_detail
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .as_deref()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .expect("blocked head detail should be surfaced")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .contains("could not cherry-pick into `prerelease` cleanly")
    );
    assert!(
        snapshot.distributor.head_rebase_provenance.is_none(),
        "failed rebase should not report successful rebase provenance"
    );
    assert_eq!(
        operations
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .lock()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .expect("fake github operations mutex poisoned")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            format!("ensure-pr:prerelease:{}", lease.branch_name),
            "inspect-pr:77".to_string(),
        ]
    );
}
