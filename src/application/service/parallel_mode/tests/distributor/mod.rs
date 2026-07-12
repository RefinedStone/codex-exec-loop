use super::*;

#[test]
fn enqueue_blocks_public_repository_without_exact_operator_opt_in() {
    let repo = TempGitRepo::new("distributor-public-repository-blocked");
    run_git(
        &repo.repo_root,
        &["config", "akra.parallelAllowPublicRepository", "true"],
    );
    let service = test_parallel_mode_service_with_github(Arc::new(
        FakeGithubAutomationPort::with_repository_visibility(GithubRepositoryVisibility::Public),
    ));
    let error = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect_err("public repository worker must not start without parent opt-in");

    assert!(error.contains("public GitHub repository delivery is blocked before worker start"));
    assert!(
        !repo
            .pool_root()
            .join("slot-1")
            .join(".akra-slot-lease.json")
            .exists(),
        "repo-local config must not create a public-repository lease"
    );
}

#[test]
fn enqueue_rejects_legacy_lease_without_delivery_target() {
    let repo = TempGitRepo::new("distributor-legacy-lease-target");
    let service = test_parallel_mode_service();
    let mut lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-legacy", "Legacy", "agent-legacy", "legacy"),
        )
        .expect("slot lease should be acquired");
    lease.delivery_target = None;
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("legacy lease fixture should persist in authority and mirror");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("legacy slot should transition to running");
    repo.commit_file_in_slot(
        Path::new(&lease.worktree_path),
        "legacy.txt",
        "done\n",
        "legacy worker result",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-legacy-lease",
            None,
            Some("Legacy result."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("refreshing should be recorded");
    service
        .mark_workspace_commit_ready(&lease.worktree_path, "legacy lease ready")
        .expect("commit-ready should be recorded");

    let error = service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect_err("legacy lease must fail closed before enqueue");
    assert!(error.contains("legacy slot lease has no immutable delivery target"));
}

#[test]
fn worker_remote_redirect_cannot_freeze_another_private_repository() {
    let repo = TempGitRepo::new("distributor-worker-remote-redirect");
    let github = FakeGithubAutomationPort::ready();
    let repository_identity = github.repository_identity.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-redirect", "Redirect", "agent-redirect", "redirect"),
        )
        .expect("slot lease should freeze the original target");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(
        Path::new(&lease.worktree_path),
        "redirect.txt",
        "done\n",
        "worker result",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-redirect",
            None,
            Some("Redirect attempt."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("refreshing should be recorded");
    service
        .mark_workspace_commit_ready(&lease.worktree_path, "redirect result ready")
        .expect("commit-ready should be recorded");

    let redirected_remote = repo.root.join("redirect.git");
    fs::create_dir_all(&redirected_remote).expect("redirected bare remote root should exist");
    run_git(&redirected_remote, &["init", "--bare", "-q"]);
    run_git(
        &repo.repo_root,
        &[
            "remote",
            "set-url",
            DEFAULT_PUSH_REMOTE_NAME,
            redirected_remote
                .to_str()
                .expect("redirected remote path should be valid utf-8"),
        ],
    );
    *repository_identity
        .lock()
        .expect("fake github repository identity mutex poisoned") =
        "attacker/other-private-repo".to_string();

    let error = service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect_err("worker target redirect must fail closed");
    assert!(
        error.contains("slot lease delivery target drifted after worker start"),
        "unexpected redirect rejection: {error}"
    );
    assert!(
        error.contains("credential-redacted push URL changed"),
        "redirected URL must be part of the immutable-target rejection: {error}"
    );
    assert!(
        error.contains("GitHub repository"),
        "redirected repository identity must be part of the immutable-target rejection: {error}"
    );
    assert!(load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty());
}

#[test]
fn distributor_blocks_legacy_queue_record_before_remote_delivery() {
    let repo = TempGitRepo::new("distributor-legacy-target-blocked");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let _lease = blocked::enqueue_single_commit_ready_result(&service, &repo, "turn-legacy-target");
    let mut record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    record.delivery_target = None;
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &record,
    )
    .expect("legacy queue record should persist");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("legacy record should be durably blocked");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("immutable delivery target"))
    );
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty()
    );
}

#[test]
fn distributor_blocks_queue_without_frozen_source_base_before_remote_delivery() {
    let repo = TempGitRepo::new("distributor-legacy-source-base-blocked");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    blocked::enqueue_single_commit_ready_result(&service, &repo, "turn-legacy-source-base");
    let mut record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    record.source_base_commit_sha.clear();
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &record,
    )
    .expect("legacy queue record should persist");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("legacy source range should be durably blocked");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("lease identity no longer matches distributor session"))
    );
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty()
    );
}

#[test]
fn passive_supervisor_keeps_durable_queue_visible_without_claiming_tui_mode() {
    let repo = TempGitRepo::new("passive-supervisor-durable-queue");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    blocked::enqueue_single_commit_ready_result(&service, &repo, "turn-passive-projection");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let snapshot =
        service.build_passive_supervisor_snapshot(&repo.workspace_dir(), Some(&readiness));

    assert_eq!(snapshot.state, ParallelModeSupervisorState::Prepare);
    assert_eq!(snapshot.roster.active_count(), 1);
    assert_eq!(snapshot.distributor.queue_depth(), 1);
    assert_ne!(snapshot.distributor.head_summary, "inactive");
    assert!(
        snapshot
            .top_notice
            .as_deref()
            .is_some_and(|notice| { notice.contains("native TUI mode is not shared") })
    );

    let blocked_readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Blocked,
        vec![],
        Some("git readiness is blocked".to_string()),
    );
    let blocked_snapshot =
        service.build_passive_supervisor_snapshot(&repo.workspace_dir(), Some(&blocked_readiness));
    assert_eq!(blocked_snapshot.state, ParallelModeSupervisorState::Prepare);
    assert_eq!(blocked_snapshot.roster.active_count(), 1);
    assert_eq!(blocked_snapshot.distributor.queue_depth(), 1);
    assert!(
        blocked_snapshot
            .top_notice
            .as_deref()
            .is_some_and(|notice| {
                notice.contains("native TUI mode is not shared")
                    && notice.contains("git readiness is blocked")
            })
    );
}

#[test]
fn distributor_blocks_push_remote_drift_before_remote_delivery() {
    let repo = TempGitRepo::new("distributor-target-drift-blocked");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let _lease = blocked::enqueue_single_commit_ready_result(&service, &repo, "turn-target-drift");
    run_git(
        &repo.repo_root,
        &["config", "akra.githubPushRemote", "upstream"],
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("target drift should be durably blocked");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("delivery target drifted"))
    );
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty()
    );
}

#[test]
fn distributor_blocks_same_identity_push_url_swap_before_remote_delivery() {
    let repo = TempGitRepo::new("distributor-push-url-swap-blocked");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    blocked::enqueue_single_commit_ready_result(&service, &repo, "turn-push-url-swap");

    let redirected_remote = repo.root.join("attacker-controlled.git");
    fs::create_dir_all(&redirected_remote).expect("redirected remote root should be created");
    run_git(&redirected_remote, &["init", "--bare", "-q"]);
    run_git(
        &repo.repo_root,
        &[
            "remote",
            "set-url",
            "--push",
            DEFAULT_PUSH_REMOTE_NAME,
            redirected_remote
                .to_str()
                .expect("redirected remote path should be utf-8"),
        ],
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("push URL drift should be durably blocked");

    assert!(notices.iter().any(|notice| {
        notice.contains("delivery target drifted")
            && notice.contains("credential-redacted push URL changed")
    }));
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty(),
        "no remote operation may start after a same-identity URL swap"
    );
}

#[test]
fn distributor_blocks_legacy_queue_without_frozen_push_url() {
    let repo = TempGitRepo::new("distributor-legacy-push-url-blocked");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    blocked::enqueue_single_commit_ready_result(&service, &repo, "turn-legacy-push-url");
    let mut record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    record
        .delivery_target
        .as_mut()
        .expect("delivery target should exist")
        .credential_redacted_push_url = None;
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &record,
    )
    .expect("legacy target fixture should persist");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("legacy push URL proof should be durably blocked");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("no credential-redacted immutable push URL"))
    );
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty()
    );
}

// 성공한 distributor run은 worker의 commit-ready 결과를 `prerelease`에 통합하고,
// GitHub PR lifecycle을 정리한 뒤 slot을 idle pool로 되돌리는 전체 happy path다.
// 이 테스트는 queue record, session history, fake GitHub 호출 순서, branch cleanup,
// 실제 `prerelease` tree까지 함께 묶어 end-to-end 계약을 고정한다.
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
        None,
        "canonical integration checkout must remain untouched"
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
        )
        .as_deref(),
        Some("done")
    );
    assert_eq!(
        fs::read_to_string(repo.repo_root.join("operator-note.tmp"))
            .expect("canonical untracked note should be preserved"),
        "local note\n"
    );
}

#[test]
fn cleanup_generation_mismatch_never_fails_the_replacement_session() {
    let repo = TempGitRepo::new("distributor-cleanup-replacement-lease");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let original = blocked::enqueue_single_commit_ready_result(
        &service,
        &repo,
        "turn-cleanup-replacement-lease",
    );
    let mut replacement = original.clone();
    replacement.leased_at = "2099-01-01T00:00:00Z".to_string();
    replacement.lease_generation =
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string());
    replacement.state = ParallelModeSlotLeaseState::Running;
    replacement.running_started_at = Some("2099-01-01T00:00:01Z".to_string());
    assert_ne!(original.session_key(), replacement.session_key());
    let expected_replacement = replacement.clone();
    let workspace_dir = repo.workspace_dir();
    let pool_root = repo.pool_root();
    install_before_distributor_cleanup_lock_hook(move || {
        let authority = SqlitePlanningAuthorityAdapter::new();
        let runtime = test_parallel_runtime();
        write_slot_lease(
            &authority,
            &runtime,
            &workspace_dir,
            &pool_root,
            &replacement,
        )
        .expect("replacement live lease should persist before cleanup revalidation");
        record_running_session_detail(
            &authority,
            &runtime,
            &workspace_dir,
            &pool_root,
            &replacement,
        )
        .expect("replacement live session detail should persist");
    });

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("cleanup generation mismatch should become a durable queue block");

    assert!(
        notices
            .iter()
            .any(|notice| notice
                .contains("lease generation changed before locked distributor cleanup")),
        "generation mismatch should be explicit: {notices:?}"
    );
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projection should remain readable");
    assert_eq!(
        projection.slot_leases.get(&expected_replacement.slot_id),
        Some(&expected_replacement)
    );
    let replacement_detail = projection
        .session_details
        .iter()
        .find(|detail| detail.session_key == expected_replacement.session_key())
        .expect("replacement session detail should remain present");
    assert_eq!(replacement_detail.state_label, "running");
    assert_eq!(replacement_detail.completion_state_label, "in_progress");
    assert!(replacement_detail.distributor_outcome.is_none());
}

#[test]
fn process_distributor_queue_integrates_the_complete_reviewed_multi_commit_range() {
    let repo = TempGitRepo::new("distributor-multi-commit-range");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-range", "Task Range", "agent-range", "task-range"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "first.txt", "first\n", "first reviewed commit");
    repo.commit_file_in_slot(
        &slot_path,
        "second.txt",
        "second\n",
        "second reviewed commit",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-range",
            None,
            Some("Multi-commit result completed."),
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
            "official ledger refresh succeeded: multi-commit delivery approved",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("multi-commit result should enqueue")
        .expect("queue item should be created");

    let queued = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    let range = format!(
        "{}..{}",
        queued.source_base_commit_sha, queued.source_commit_sha
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "rev-list",
                "--count",
                range.as_str(),
            ],
            None,
        )
        .as_deref(),
        Some("2")
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("multi-commit queue should process");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor returned slot to idle"))
    );
    for (path, expected) in [("first.txt", "first"), ("second.txt", "second")] {
        assert_eq!(
            run_command(
                "git",
                [
                    "-C",
                    repo.workspace_dir().as_str(),
                    "show",
                    &format!("refs/remotes/origin/prerelease:{path}"),
                ],
                None,
            )
            .as_deref(),
            Some(expected),
            "the remote integration branch must contain every reviewed source commit"
        );
    }
}

fn assert_close_inspection_drift_preserves_remote_review_surface(
    prefix: &str,
    github: FakeGithubAutomationPort,
    expected_drift: &str,
) {
    let repo = TempGitRepo::new(prefix);
    let operations = github.operations.clone();
    let source_branch_cleanup_calls = github.source_branch_cleanup_calls.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease =
        blocked::enqueue_single_commit_ready_result(&service, &repo, &format!("turn-{prefix}"));

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("drifted close inspection should finish conservatively");

    assert!(
        notices.iter().any(|notice| {
            notice.contains(expected_drift) && notice.contains("preserved for operator review")
        }),
        "operator notice should explain preserved review surface: {notices:?}"
    );
    let operations = operations
        .lock()
        .expect("fake github operations mutex poisoned")
        .clone();
    assert!(operations.contains(&"push-integration:prerelease".to_string()));
    assert!(
        operations
            .iter()
            .all(|operation| !operation.starts_with("close-pr:")),
        "a drifted pull request must remain open: {operations:?}"
    );
    assert_eq!(
        *source_branch_cleanup_calls
            .lock()
            .expect("fake github source cleanup counter mutex poisoned"),
        0,
        "remote source cleanup must be skipped with a drifted PR"
    );
    assert!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root
                    .to_str()
                    .expect("repo root should be valid utf-8"),
                "ls-remote",
                "--heads",
                DEFAULT_PUSH_REMOTE_NAME,
                &format!("refs/heads/{}", lease.branch_name),
            ],
            None,
        )
        .is_some(),
        "the remote source branch should remain available for operator review"
    );
    let record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(record.queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(record.integration_state, "done");
    assert!(
        record
            .recovery_note
            .as_deref()
            .is_some_and(|note| note.contains(expected_drift) && note.contains("preserved"))
    );
    assert!(record.integration_note.contains("review surface"));
}

#[test]
fn distributor_preserves_pr_and_source_branch_when_close_base_drifted() {
    assert_close_inspection_drift_preserves_remote_review_surface(
        "close-base-drift",
        FakeGithubAutomationPort::with_close_inspect_base_branch("main"),
        "base `main` != `prerelease`",
    );
}

#[test]
fn distributor_preserves_pr_and_source_branch_when_close_head_drifted() {
    assert_close_inspection_drift_preserves_remote_review_surface(
        "close-head-drift",
        FakeGithubAutomationPort::with_close_inspect_head_branch("feature/reassigned"),
        "head `feature/reassigned` !=",
    );
}

#[test]
fn distributor_preserves_pr_and_source_branch_when_close_sha_drifted() {
    assert_close_inspection_drift_preserves_remote_review_surface(
        "close-sha-drift",
        FakeGithubAutomationPort::with_close_inspect_head_commit_sha("deadbeef"),
        "head commit `deadbee` !=",
    );
}

// 운영 장애에서 보인 "여러 결과가 queued인데 head 이후가 멈춘다"는 증상을 고정한다.
// fake PR facade는 쓰되 source branch와 integration branch push는 temp bare `origin`
// 에 실제 수행해, queue drain이 로컬 통합만 끝내고 remote delivery를 빠뜨리지 않는지 검증한다.
#[test]
fn process_distributor_queue_drains_three_commit_ready_results_to_origin_prerelease() {
    let repo = TempGitRepo::new("distributor-three-origin-push");
    let origin_path = repo.create_bare_origin_remote();
    run_git(
        &repo.repo_root,
        &["push", "-u", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    let github = GitBackedGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let mut leases = Vec::new();

    for index in 1..=3 {
        let task_id = format!("task-{index}");
        let task_title = format!("Task {index}");
        let agent_id = format!("agent-{index}");
        let task_slug = format!("task-{index}");
        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request(&task_id, &task_title, &agent_id, &task_slug),
            )
            .expect("slot lease should be acquired");
        let slot_path = PathBuf::from(lease.worktree_path.clone());
        service
            .record_workspace_slot_thread_prepared(&lease.worktree_path, &format!("thread-{index}"))
            .expect("thread prepared should be recorded");
        service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should transition to running");
        repo.commit_file_in_slot(
            &slot_path,
            &format!("feature-{index}.txt"),
            &format!("done {index}\n"),
            &format!("agent {index} work"),
        );
        service
            .begin_workspace_official_completion(
                &lease.worktree_path,
                &format!("turn-{index}"),
                None,
                Some(&format!("Task {index} completed.")),
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
                &format!(
                    "official ledger refresh succeeded: task {index} distributor delivery approved"
                ),
            )
            .expect("commit-ready should be recorded");
        let queued_item = service
            .enqueue_workspace_commit_ready_result(&lease.worktree_path)
            .expect("commit-ready result should be enqueued")
            .expect("queue item should be created");
        assert_eq!(queued_item.queue_state, ParallelModeQueueItemState::Queued);
        leases.push(lease);
        thread::sleep(Duration::from_millis(2));
    }

    let queued_snapshot =
        service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(queued_snapshot.roster.active_count(), 3);
    assert_eq!(queued_snapshot.distributor.head_summary, "queued");
    assert_eq!(queued_snapshot.distributor.queue_depth(), 3);
    assert_eq!(
        queued_snapshot
            .distributor
            .orchestrator_status
            .held_queue_count,
        2
    );

    for expected_done_count in 1..=3 {
        let notices = service
            .process_distributor_queue(&repo.workspace_dir())
            .expect("distributor queue should process");
        assert!(
            notices
                .iter()
                .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
            "queue tick should integrate the head: {notices:?}"
        );
        assert!(
            notices
                .iter()
                .any(|notice| notice.contains("distributor returned slot to idle")),
            "queue tick should return the integrated slot: {notices:?}"
        );
        assert_eq!(
            load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
                .iter()
                .filter(|record| record.queue_state == ParallelModeQueueItemState::Done)
                .count(),
            expected_done_count
        );
    }

    let drained_projection =
        service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(drained_projection.roster.active_count(), 0);
    assert_eq!(drained_projection.pool.blocked_slots, 0);
    assert_eq!(drained_projection.pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(drained_projection.distributor.head_summary, "idle");
    assert_eq!(drained_projection.distributor.queue_depth(), 0);
    let records = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root());
    assert_eq!(records.len(), 3);
    assert!(
        records
            .iter()
            .all(|record| record.queue_state == ParallelModeQueueItemState::Done),
        "every queue record should finish after three distributor ticks: {records:?}"
    );

    for (index, lease) in leases.iter().enumerate() {
        let number = index + 1;
        let feature_file = format!("feature-{number}.txt");
        let expected_contents = format!("done {number}");
        assert!(!repo.slot_lease_path(number).exists());
        assert!(!repo.branch_exists(&lease.branch_name));
        assert_eq!(
            run_command(
                "git",
                [
                    "--git-dir",
                    origin_path
                        .to_str()
                        .expect("origin path should be valid utf-8"),
                    "show",
                    &format!("{}:{feature_file}", lease.branch_name),
                ],
                None,
            )
            .as_deref(),
            None,
            "source branch should be conditionally deleted after verified integration"
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
                    &format!("{POOL_BASELINE_BRANCH}:{feature_file}"),
                ],
                None,
            )
            .as_deref(),
            Some(expected_contents.as_str())
        );
    }

    assert_eq!(
        operations
            .lock()
            .expect("git-backed github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", leases[0].branch_name),
            format!("ensure-pr:prerelease:{}", leases[0].branch_name),
            "inspect-pr:900".to_string(),
            "inspect-pr:900".to_string(),
            "push-integration:prerelease".to_string(),
            "inspect-pr:900".to_string(),
            "close-pr:900".to_string(),
            format!("push:{}:false", leases[1].branch_name),
            format!("ensure-pr:prerelease:{}", leases[1].branch_name),
            "inspect-pr:901".to_string(),
            "inspect-pr:901".to_string(),
            "push-integration:prerelease".to_string(),
            "inspect-pr:901".to_string(),
            "close-pr:901".to_string(),
            format!("push:{}:false", leases[2].branch_name),
            format!("ensure-pr:prerelease:{}", leases[2].branch_name),
            "inspect-pr:902".to_string(),
            "inspect-pr:902".to_string(),
            "push-integration:prerelease".to_string(),
            "inspect-pr:902".to_string(),
            "close-pr:902".to_string(),
        ]
    );
}

// 실제 장애에서 오래된 queue head는 PR이 닫혔다는 이유로 blocked가 됐지만,
// 그 source commit의 patch는 이미 prerelease에 들어가 있었다. 이 경우 distributor는
// stale head를 cleanup으로 수렴시키고 뒤의 두 commit-ready task를 계속 rebase-merge해야 한다.
#[test]
fn process_distributor_queue_recovers_patch_equivalent_closed_head_and_merges_two_results() {
    let repo = TempGitRepo::new("distributor-recovers-closed-equivalent-head");
    let origin_path = repo.create_bare_origin_remote();
    run_git(
        &repo.repo_root,
        &["push", "-u", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    let github = GitBackedGithubAutomationPort::ready();
    let service = test_parallel_mode_service_with_github(Arc::new(github));

    let stale_lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("stale-task", "Stale Task", "agent-stale", "stale-task"),
        )
        .expect("stale slot lease should be acquired");
    let stale_slot_path = PathBuf::from(stale_lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&stale_lease.worktree_path)
        .expect("stale slot should transition to running");
    repo.commit_file_in_slot(
        &stale_slot_path,
        "stale-feature.txt",
        "already integrated\n",
        "stale agent work",
    );
    let stale_commit = run_command(
        "git",
        [
            "-C",
            stale_lease.worktree_path.as_str(),
            "rev-parse",
            "HEAD",
        ],
        None,
    )
    .expect("stale source commit should resolve");
    service
        .begin_workspace_official_completion(
            &stale_lease.worktree_path,
            "turn-stale",
            None,
            Some("Stale task completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("stale official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&stale_lease.worktree_path)
        .expect("stale ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &stale_lease.worktree_path,
            "official ledger refresh succeeded: stale delivery approved",
        )
        .expect("stale commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&stale_lease.worktree_path)
        .expect("stale commit-ready result should be enqueued")
        .expect("stale queue item should be created");

    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    run_git(&repo.repo_root, &["cherry-pick", stale_commit.as_str()]);
    run_git(
        &repo.repo_root,
        &[
            "commit",
            "--amend",
            "-qm",
            "manual equivalent stale integration",
        ],
    );
    run_git(
        &repo.repo_root,
        &["push", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );

    let mut stale_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("stale queue record should exist");
    stale_record.queue_state = ParallelModeQueueItemState::Blocked;
    stale_record.integration_state = "blocked".to_string();
    stale_record.pull_request_number = Some(1383);
    stale_record.integration_note =
        "recovered after restart: pull request #1383 is `CLOSED` before integration".to_string();
    stale_record.recovery_note = Some(stale_record.integration_note.clone());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &stale_record,
    )
    .expect("stale blocked queue record should be stored");
    thread::sleep(Duration::from_millis(2));

    let mut leases = Vec::new();
    for index in 1..=2 {
        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request(
                    &format!("task-{index}"),
                    &format!("Task {index}"),
                    &format!("agent-{index}"),
                    &format!("task-{index}"),
                ),
            )
            .expect("slot lease should be acquired");
        let slot_path = PathBuf::from(lease.worktree_path.clone());
        service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should transition to running");
        repo.commit_file_in_slot(
            &slot_path,
            &format!("feature-{index}.txt"),
            &format!("done {index}\n"),
            &format!("agent {index} work"),
        );
        service
            .begin_workspace_official_completion(
                &lease.worktree_path,
                &format!("turn-{index}"),
                None,
                Some(&format!("Task {index} completed.")),
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
                &format!(
                    "official ledger refresh succeeded: task {index} distributor delivery approved"
                ),
            )
            .expect("commit-ready should be recorded");
        service
            .enqueue_workspace_commit_ready_result(&lease.worktree_path)
            .expect("commit-ready result should be enqueued")
            .expect("queue item should be created");
        leases.push(lease);
        thread::sleep(Duration::from_millis(2));
    }

    let cleanup_notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("stale equivalent head should recover through cleanup");
    assert!(
        cleanup_notices
            .iter()
            .all(|notice| !notice.contains("blocked")),
        "patch-equivalent closed head should not block later queue items: {cleanup_notices:?}"
    );
    assert!(
        cleanup_notices
            .iter()
            .any(|notice| notice.contains("distributor returned slot to idle")),
        "stale head should be cleaned before later queue items merge: {cleanup_notices:?}"
    );

    for expected_done_count in 2..=3 {
        let notices = service
            .process_distributor_queue(&repo.workspace_dir())
            .expect("later distributor queue item should process");
        assert!(
            notices
                .iter()
                .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
            "queue tick should integrate the head: {notices:?}"
        );
        assert!(
            notices
                .iter()
                .any(|notice| notice.contains("distributor returned slot to idle")),
            "queue tick should return the integrated slot: {notices:?}"
        );
        assert_eq!(
            load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
                .iter()
                .filter(|record| record.queue_state == ParallelModeQueueItemState::Done)
                .count(),
            expected_done_count
        );
    }

    let records = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root());
    assert_eq!(records.len(), 3);
    assert!(
        records
            .iter()
            .all(|record| record.queue_state == ParallelModeQueueItemState::Done),
        "stale head and two later task records should all finish: {records:?}"
    );
    assert!(!repo.slot_lease_path(1).exists());
    for (index, lease) in leases.iter().enumerate() {
        let number = index + 1;
        assert!(!repo.slot_lease_path(number + 1).exists());
        assert!(!repo.branch_exists(&lease.branch_name));
        assert_eq!(
            run_command(
                "git",
                [
                    "--git-dir",
                    origin_path
                        .to_str()
                        .expect("origin path should be valid utf-8"),
                    "show",
                    &format!("{POOL_BASELINE_BRANCH}:feature-{number}.txt"),
                ],
                None,
            )
            .as_deref(),
            Some(format!("done {number}").as_str())
        );
    }
}

// lease branch는 현재 `prerelease` head에서 시작해야 한다. 사용자가 prerelease를
// 먼저 전진시킨 뒤 lease를 얻는 상황에서 source parent가 최신 baseline인지,
// distributor 통합 후 기존 prerelease-only 파일과 worker 파일이 모두 유지되는지
// 확인한다.
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
                "refs/remotes/origin/prerelease:feature.txt",
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
            "refs/remotes/origin/prerelease",
        ],
    );
    let patch_equivalent = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "cherry",
            "refs/remotes/origin/prerelease",
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
            "inspect-pr:77".to_string(),
            "push-integration:prerelease".to_string(),
            "inspect-pr:77".to_string(),
            "close-pr:77".to_string(),
        ]
    );
}

#[test]
fn process_distributor_queue_uses_dedicated_worktree_without_resetting_diverged_local_history() {
    let repo = TempGitRepo::new("distributor-remote-already-integrated");
    repo.create_bare_origin_remote();
    run_git(&repo.repo_root, &["push", "-u", "origin", "prerelease"]);
    let github = FakeGithubAutomationPort::with_integration_push_error("non-fast-forward");
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
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-remote-integrated")
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
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-remote-integrated",
            None,
            Some("Remote already integrated slice completed."),
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
            "official ledger refresh succeeded: remote integration recovery approved",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should be enqueued")
        .expect("queue item should be created");

    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    run_git(&repo.repo_root, &["cherry-pick", source_commit.as_str()]);
    run_git(
        &repo.repo_root,
        &["commit", "--amend", "-qm", "external rebase merge"],
    );
    run_git(&repo.repo_root, &["push", "origin", "prerelease"]);
    let remote_prerelease = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "rev-parse",
            "prerelease",
        ],
        None,
    )
    .expect("remote-equivalent prerelease head should resolve");
    run_git(&repo.repo_root, &["reset", "--hard", "HEAD~1"]);
    let local_prerelease_before_delivery = repo.head_sha();
    assert_ne!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "rev-parse",
                "prerelease",
            ],
            None,
        )
        .as_deref(),
        Some(remote_prerelease.as_str()),
        "local prerelease should be behind the remote-equivalent head before delivery"
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor returned slot to idle")),
        "remote patch-equivalence recovery should ignore canonical branch divergence: {notices:?}"
    );
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should persist");
    assert_eq!(queue_record.queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "rev-parse",
                "prerelease",
            ],
            None,
        )
        .as_deref(),
        Some(local_prerelease_before_delivery.as_str()),
        "preflight must preserve the local integration branch instead of hard-resetting it"
    );
    assert!(
        !operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .contains(&"push-integration:prerelease".to_string()),
        "remote patch-equivalence recovery should not push integration again"
    );
}

// hidden worker의 official completion 성공 경로는 slot worktree에서 호출된다. 이때 뒤따르는
// orchestrator tick은 slot checkout이나 canonical checkout이 아닌 전용 integration worktree에서 처리한다.
#[test]
fn official_completion_success_orchestrator_tick_uses_dedicated_integration_worktree() {
    let repo = TempGitRepo::new("official-success-canonical-orchestrator");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);
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
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-canonical-tick")
        .expect("thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-canonical-tick",
            None,
            Some("Canonical orchestrator tick completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");

    let turn_service =
        crate::application::service::parallel_mode::turn::ParallelModeTurnService::new(
            service.clone(),
        );
    let finalize_outcome = turn_service.finalize_official_completion_success(
        &lease.worktree_path,
        "official ledger refresh succeeded: canonical tick approved",
    );
    let notices = match finalize_outcome {
        crate::application::service::parallel_mode::turn::ParallelOfficialCompletionFinalizeOutcome::Durable {
            notices,
            ..
        } => notices,
        crate::application::service::parallel_mode::turn::ParallelOfficialCompletionFinalizeOutcome::Failed {
            notices,
            stage,
        } => panic!("official completion should finalize durably, not fail at {stage:?}: {notices:?}"),
    };

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
        "slot-originated success should process the queue through the integration worktree: {notices:?}"
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
        None
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
        )
        .as_deref(),
        Some("done")
    );
}

// canonical checkout branch와 무관하게 official completion은 전용 integration worktree에서
// delivery를 끝내야 한다.
#[test]
fn official_completion_ignores_canonical_checkout_branch() {
    let repo = TempGitRepo::new("official-success-blocked-retry");
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
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-blocked-retry")
        .expect("thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "retry.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-blocked-retry",
            None,
            Some("Official completion should queue and wait for the integration worktree."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");

    let turn_service =
        crate::application::service::parallel_mode::turn::ParallelModeTurnService::new(
            service.clone(),
        );
    let finalize_outcome = turn_service.finalize_official_completion_success(
        &lease.worktree_path,
        "official ledger refresh succeeded: distributor delivery approved",
    );
    let notices = match finalize_outcome {
        crate::application::service::parallel_mode::turn::ParallelOfficialCompletionFinalizeOutcome::Durable {
            notices,
            ..
        } => notices,
        crate::application::service::parallel_mode::turn::ParallelOfficialCompletionFinalizeOutcome::Failed {
            notices,
            stage,
        } => panic!("official completion should finalize durably, not fail at {stage:?}: {notices:?}"),
    };

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("commit-ready result entered the distributor queue")),
        "official completion should enqueue the result: {notices:?}"
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
        "dedicated worktree should complete delivery without canonical checkout preparation: {notices:?}"
    );
    assert!(
        !operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty(),
        "dedicated integration delivery should execute GitHub operations"
    );
    let projection =
        service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(projection.distributor.head_summary, "idle");
    assert_eq!(projection.distributor.queue_depth(), 0);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "show",
                "refs/remotes/origin/prerelease:retry.txt",
            ],
            None,
        )
        .as_deref(),
        Some("done")
    );
}

// supervisor detail pane은 단순히 가장 최근 running session을 고르면 안 된다.
// distributor queue head가 있으면 아직 delivery를 기다리는 queued item이 운영상
// 더 중요하므로, 뒤이어 실행된 다른 agent보다 queue head session을 선택해야 한다.
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

// queue가 비어 있어도 distributor 영역은 완전히 사라지지 않는다. idle 상태에서도
// orchestrator status와 integration worktree readiness를 보여줘야 TUI가 다음
// delivery를 받을 준비 상태를 설명할 수 있다.
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
            .contains("no active delivery"),
        "idle snapshot should still inspect integration readiness: {}",
        snapshot
            .distributor
            .orchestrator_status
            .integration_worktree_readiness
    );
}

// 차단, rebase conflict, recovery 시나리오는 happy path보다 실패 원인이 길어
// 별도 파일로 둔다. 이 상위 모듈은 성공 경로와 supervisor projection의 공통
// distributor 계약만 남긴다.
mod blocked;
mod rebase;
mod recovery;
