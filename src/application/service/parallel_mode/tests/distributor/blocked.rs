use super::*;

fn push_ready_pr_unavailable_capabilities() -> GithubAutomationCapabilities {
    GithubAutomationCapabilities::new(
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
    )
}

fn enqueue_single_commit_ready_result(
    service: &ParallelModeService,
    repo: &TempGitRepo,
    turn_id: &str,
) -> ParallelModeSlotLeaseSnapshot {
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
            turn_id,
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
    lease
}

fn write_slot_git_metadata(slot_path: &Path, file_name: &str) {
    let git_dir = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be valid utf-8"),
            "rev-parse",
            "--git-dir",
        ],
        None,
    )
    .expect("slot git dir should resolve");
    let git_dir = PathBuf::from(git_dir);
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        slot_path.join(git_dir)
    };
    fs::write(git_dir.join(file_name), "synthetic pending operation\n")
        .expect("slot git metadata should be writable");
}

// PR workflow가 required인 상태에서 `gh` 실행과 인증이 degraded라면 distributor는
// branch push까지는 진행하되 PR 자동화를 이어갈 수 없으므로 queue head를
// blocked로 남기고, slot lease와 session history를 실패 상태로 보존해야 한다.
#[test]
fn distributor_queue_blocks_after_push_when_pull_request_workflow_is_required_and_unavailable() {
    let repo = TempGitRepo::new("distributor-queue-gh-blocked");
    run_git(
        &repo.repo_root,
        &["config", "akra.githubPrMode", "required"],
    );
    let github =
        FakeGithubAutomationPort::with_capabilities(push_ready_pr_unavailable_capabilities());
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
            .any(|notice| notice.contains("pull request workflow is required but unavailable"))
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
            .contains("pull request workflow is required but unavailable")
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

// 기본 auto 모드에서는 git push가 가능하고 PR workflow만 불가능한 환경을 direct delivery로
// 처리해야 한다. 이 경로가 있어야 gh 인증이 없는 사용자의 로컬 git push 가능 상태와 같은
// 동작을 제공할 수 있다.
#[test]
fn distributor_auto_mode_direct_integrates_when_pull_request_workflow_is_unavailable() {
    let repo = TempGitRepo::new("distributor-auto-direct-delivery");
    let github =
        FakeGithubAutomationPort::with_capabilities(push_ready_pr_unavailable_capabilities());
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-auto-direct");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor skipped pull request workflow")),
        "auto mode should explain the PR skip: {notices:?}"
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
        "direct delivery should integrate after skipping PR: {notices:?}"
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            "push-integration:prerelease".to_string(),
        ]
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(queue_record.queue_state, ParallelModeQueueItemState::Done);
    assert!(queue_record.pull_request_number.is_none());
    assert!(
        queue_record
            .integration_note
            .contains("direct delivery completed without PR automation")
    );
}

// disabled 모드는 gh/인증이 준비되어 있어도 PR surface를 사용하지 않는다. 이 설정은
// PR 없이 로컬 인증 git push만으로 prerelease 통합을 운영하려는 저장소를 위한 명시적 override다.
#[test]
fn distributor_disabled_mode_skips_pull_request_workflow_even_when_available() {
    let repo = TempGitRepo::new("distributor-disabled-direct-delivery");
    run_git(
        &repo.repo_root,
        &["config", "akra.githubPrMode", "disabled"],
    );
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-disabled-direct");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor skipped pull request workflow")),
        "disabled mode should report the PR skip: {notices:?}"
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            "push-integration:prerelease".to_string(),
        ]
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(queue_record.queue_state, ParallelModeQueueItemState::Done);
    assert!(queue_record.pull_request_number.is_none());
}

// direct delivery로 전환하더라도 이미 durable record에 남아 있던 PR metadata는
// 보존해야 한다. 그래야 재시도 중 정책이 바뀌어도 integration 이후 기존 PR close 경로가
// 실행되고, 열린 PR을 잃어버리지 않는다.
#[test]
fn distributor_skip_preserves_existing_pull_request_metadata_for_close() {
    let repo = TempGitRepo::new("distributor-skip-preserves-pr");
    run_git(
        &repo.repo_root,
        &["config", "akra.githubPrMode", "disabled"],
    );
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-preserve-pr");
    let mut queue_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queue record should exist");
    queue_record.pull_request_number = Some(123);
    queue_record.pull_request_url = Some("https://example.invalid/pr/123".to_string());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &queue_record,
    )
    .expect("queue record with existing PR metadata should persist");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor skipped pull request workflow")),
        "disabled mode should still report the PR skip: {notices:?}"
    );
    let operations = operations
        .lock()
        .expect("fake github operations mutex poisoned")
        .clone();
    assert!(
        operations.contains(&format!("push:{}:false", lease.branch_name)),
        "source push should still run before direct integration: {operations:?}"
    );
    assert!(
        operations.contains(&"push-integration:prerelease".to_string()),
        "integration branch push should run: {operations:?}"
    );
    assert!(
        operations.contains(&"close-pr:123".to_string()),
        "existing PR metadata should drive PR close: {operations:?}"
    );
    assert!(
        operations
            .iter()
            .all(|operation| !operation.starts_with("ensure-pr:")),
        "skip mode must not create or re-ensure a PR: {operations:?}"
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(queue_record.queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(queue_record.pull_request_number, Some(123));
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

#[test]
fn distributor_blocks_when_slot_lease_disappears_before_delivery() {
    let repo = TempGitRepo::new("distributor-missing-slot-lease");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-missing-lease");
    SqlitePlanningAuthorityAdapter::remove_runtime_slot_lease(
        &repo.workspace_dir(),
        &lease.slot_id,
    )
    .expect("slot lease should be removed from authority");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("slot lease disappeared before distributor integration")),
        "missing lease should block before GitHub delivery: {notices:?}"
    );
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty(),
        "delivery must not push when the live slot lease disappeared"
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
}

#[test]
fn distributor_blocks_when_slot_has_pending_operation_metadata() {
    let repo = TempGitRepo::new("distributor-slot-pending-operation");
    run_git(
        &repo.repo_root,
        &["config", "akra.githubPrMode", "disabled"],
    );
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-slot-pending");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    write_slot_git_metadata(&slot_path, "CHERRY_PICK_HEAD");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("has pending merge or rebase metadata")),
        "pending slot metadata should block integration: {notices:?}"
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![format!("push:{}:false", lease.branch_name)]
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_record
            .integration_note
            .contains("pending merge or rebase metadata")
    );
}

#[test]
fn distributor_blocks_when_source_branch_head_drifts_after_enqueue() {
    let repo = TempGitRepo::new("distributor-source-head-drift");
    run_git(
        &repo.repo_root,
        &["config", "akra.githubPrMode", "disabled"],
    );
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-head-drift");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(
        &slot_path,
        "unexpected.txt",
        "drift\n",
        "unexpected extra work",
    );
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("branch head drifted from expected commit")),
        "head drift should block before cherry-pick: {notices:?}"
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![format!("push:{}:false", lease.branch_name)]
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_record
            .integration_note
            .contains("branch head drifted from expected commit")
    );
}

#[test]
fn distributor_blocks_when_integration_branch_push_is_rejected_without_remote_equivalence() {
    let repo = TempGitRepo::new("distributor-integration-push-rejection");
    run_git(
        &repo.repo_root,
        &["config", "akra.githubPrMode", "disabled"],
    );
    let github = FakeGithubAutomationPort::with_integration_push_error("non-fast-forward");
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-integration-push");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices.iter().any(|notice| {
            notice.contains("`prerelease` could not be pushed to `origin`")
                && notice.contains("non-fast-forward")
        }),
        "integration push rejection should block when remote equivalence cannot be proven: {notices:?}"
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            "push-integration:prerelease".to_string(),
        ]
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_record
            .integration_note
            .contains("`prerelease` could not be pushed to `origin`")
    );
}

#[test]
fn distributor_blocks_when_pull_request_close_fails_after_integration_push() {
    let repo = TempGitRepo::new("distributor-pr-close-failure");
    let github = FakeGithubAutomationPort::with_close_error("close rejected by policy");
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-pr-close-failure");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices.iter().any(|notice| {
            notice.contains("pull request #77 could not be closed")
                && notice.contains("close rejected by policy")
        }),
        "PR close failure should block after local integration push: {notices:?}"
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
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_record
            .integration_note
            .contains("pull request #77 could not be closed")
    );
}

#[test]
fn distributor_blocks_when_cleanup_cannot_delete_branch_checked_out_elsewhere() {
    let repo = TempGitRepo::new("distributor-cleanup-branch-held");
    let github = FakeGithubAutomationPort::ready();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = enqueue_single_commit_ready_result(&service, &repo, "turn-cleanup-held");
    let duplicate_worktree = repo.root.join("branch-holder");
    run_git(
        &repo.repo_root,
        &[
            "worktree",
            "add",
            "--force",
            duplicate_worktree
                .to_str()
                .expect("duplicate worktree path should be valid utf-8"),
            &lease.branch_name,
        ],
    );
    let mut queue_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queue record should exist");
    queue_record.queue_state = ParallelModeQueueItemState::Cleaning;
    queue_record.integration_state = "done".to_string();
    queue_record.integration_note =
        "branch integrated into prerelease and the slot is entering cleanup".to_string();
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &queue_record,
    )
    .expect("cleaning queue record should persist");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("cleanup failed after distributor delivery")),
        "held branch should make slot cleanup block: {notices:?}"
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_record
            .integration_note
            .contains("cleanup failed after distributor delivery")
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
