use super::super::*;
use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelWorkerCommitDisposition, ParallelWorkerCommitRequest,
};
use crate::application::service::parallel_mode::{
    current_timestamp, resolve_workspace_head_sha, write_slot_lease,
};
use chrono::DateTime;

fn leave_stale_integrated_tracking_ref_and_break_fetch(
    repo: &TempGitRepo,
    slot_path: &Path,
    source_commit: &str,
    result_file: &str,
) {
    let integration_base = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be valid utf-8"),
            "rev-parse",
            &format!("{source_commit}^"),
        ],
        None,
    )
    .expect("integration base should resolve");
    repo.merge_agent_slot_into_akra(slot_path);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "rev-parse",
                remote_standard_tracking_ref().as_str(),
            ],
            None,
        )
        .as_deref(),
        Some(source_commit),
        "the local remote-tracking ref should contain the stale integration proof"
    );

    let origin_path = repo.create_bare_origin_remote();
    let remote_integration_ref = local_standard_ref();
    run_git(
        &origin_path,
        &[
            "update-ref",
            remote_integration_ref.as_str(),
            integration_base.as_str(),
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
                &format!("{POOL_BASELINE_BRANCH}:{result_file}"),
            ],
            None,
        ),
        None,
        "the rewritten remote target must not contain the local result"
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
}

// lease 획득은 slot을 단순히 예약하는 것을 넘어 authority store, legacy mirror,
// pool projection이 모두 같은 agent/task/branch를 가리키도록 만드는 첫 전이다.
// 이 테스트는 dispatch가 볼 leased count와 TUI owner label까지 함께 고정한다.
#[test]
fn acquire_slot_lease_persists_metadata_and_marks_slot_leased() {
    let repo = TempGitRepo::new("lease-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task one"),
        )
        .expect("slot lease should be acquired");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let persisted = repo.read_slot_lease(1);

    assert_eq!(lease.slot_id, "slot-1");
    assert_eq!(lease.state, ParallelModeSlotLeaseState::Leased);
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Leased);
    assert_eq!(persisted.agent_id, "agent-1");
    assert_eq!(persisted.task_id, "task-1");
    assert!(
        persisted
            .branch_name
            .starts_with("akra-agent/slot-1/task-one")
    );
    let delivery_target = persisted
        .delivery_target
        .as_ref()
        .expect("lease must freeze delivery target before worker start");
    assert_eq!(delivery_target.push_remote, DEFAULT_PUSH_REMOTE_NAME);
    assert_eq!(
        delivery_target.github_repository,
        "RefinedStone/codex-exec-loop"
    );
    assert_eq!(
        delivery_target.repository_visibility,
        crate::domain::parallel_mode::ParallelModeRepositoryVisibility::Private
    );
    assert_eq!(delivery_target.integration_branch, POOL_BASELINE_BRANCH);
    assert_eq!(
        delivery_target.integration_base_commit_sha,
        resolve_workspace_head_sha(Path::new(&lease.worktree_path))
            .expect("leased worktree head should resolve")
    );
    assert_eq!(lease.delivery_target, persisted.delivery_target);
    assert_eq!(pool.leased_slots, 1);
    assert_eq!(pool.running_slots, 0);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Leased);
    assert_eq!(pool.slots[0].owner_label, "agent-1 / task-1");
}

#[test]
fn invalid_delivery_target_configuration_never_falls_back_to_default_git_mutations() {
    for (case_name, config_key, invalid_value) in [
        (
            "integration-branch",
            "akra.parallelIntegrationBranch",
            "bad branch",
        ),
        ("push-remote", "akra.githubPushRemote", "../origin"),
    ] {
        let repo = TempGitRepo::new(&format!("invalid-target-{case_name}"));
        let baseline_before = run_command(
            "git",
            [
                "-C",
                &repo.workspace_dir(),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .expect("default integration branch should resolve before the test");
        let worktrees_before = run_command(
            "git",
            [
                "-C",
                &repo.workspace_dir(),
                "worktree",
                "list",
                "--porcelain",
            ],
            None,
        )
        .expect("worktree inventory should resolve before the test");
        run_git(&repo.repo_root, &["config", config_key, invalid_value]);

        let service = test_parallel_mode_service();
        let readiness = ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        );
        let _ =
            service.reconcile_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
        assert!(
            service
                .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
                .is_err(),
            "invalid {case_name} must block pool reset"
        );
        assert!(
            service
                .acquire_slot_lease(
                    &repo.workspace_dir(),
                    sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
                )
                .is_err(),
            "invalid {case_name} must block lease creation"
        );

        assert!(
            (1..=DEFAULT_POOL_SIZE)
                .all(|slot_number| { !repo.pool_root().join(slot_id(slot_number)).exists() })
        );
        assert_eq!(
            run_command(
                "git",
                [
                    "-C",
                    &repo.workspace_dir(),
                    "rev-parse",
                    POOL_BASELINE_BRANCH
                ],
                None,
            )
            .as_deref(),
            Some(baseline_before.as_str())
        );
        assert_eq!(
            run_command(
                "git",
                [
                    "-C",
                    &repo.workspace_dir(),
                    "worktree",
                    "list",
                    "--porcelain"
                ],
                None,
            )
            .as_deref(),
            Some(worktrees_before.as_str())
        );
        assert!(
            run_command(
                "git",
                [
                    "-C",
                    &repo.workspace_dir(),
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/heads/akra-agent/",
                ],
                None,
            )
            .unwrap_or_default()
            .is_empty()
        );
    }
}

#[test]
fn remote_only_baseline_advance_blocks_pool_mutation_before_reconcile() {
    let repo = TempGitRepo::new("remote-only-advance-before-reconcile");
    let service = test_parallel_mode_service();
    let _ = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let stale_tracking_oid = run_command(
        "git",
        [
            "-C",
            &repo.workspace_dir(),
            "rev-parse",
            remote_standard_tracking_ref().as_str(),
        ],
        None,
    )
    .expect("tracking baseline should resolve");
    let baseline_tree = run_command(
        "git",
        [
            "-C",
            &repo.workspace_dir(),
            "rev-parse",
            &format!("{stale_tracking_oid}^{{tree}}"),
        ],
        None,
    )
    .expect("baseline tree should resolve");
    let remote_only_oid = run_command(
        "git",
        [
            "-C",
            &repo.workspace_dir(),
            "commit-tree",
            baseline_tree.as_str(),
            "-p",
            stale_tracking_oid.as_str(),
            "-m",
            "remote-only advance",
        ],
        None,
    )
    .expect("remote-only commit should be created");
    run_git(
        &repo.repo_root,
        &[
            "push",
            "-q",
            DEFAULT_PUSH_REMOTE_NAME,
            &format!("{remote_only_oid}:refs/heads/{POOL_BASELINE_BRANCH}"),
        ],
    );
    run_git(
        &repo.repo_root,
        &[
            "update-ref",
            remote_standard_tracking_ref().as_str(),
            stale_tracking_oid.as_str(),
        ],
    );

    let worktrees_before = run_command(
        "git",
        [
            "-C",
            &repo.workspace_dir(),
            "worktree",
            "list",
            "--porcelain",
        ],
        None,
    )
    .expect("worktree inventory should resolve");
    let slot_heads_before = (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| {
            let slot_path = repo.pool_root().join(slot_id(slot_number));
            (
                slot_path.clone(),
                run_command(
                    "git",
                    ["-C", &slot_path.display().to_string(), "rev-parse", "HEAD"],
                    None,
                ),
            )
        })
        .collect::<Vec<_>>();

    let error = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect_err("a newly observed remote baseline must require an explicit retry");

    assert!(error.contains("advanced during freshness verification"));
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                &repo.workspace_dir(),
                "worktree",
                "list",
                "--porcelain"
            ],
            None,
        )
        .as_deref(),
        Some(worktrees_before.as_str())
    );
    for (slot_path, expected_head) in slot_heads_before {
        assert_eq!(
            run_command(
                "git",
                ["-C", &slot_path.display().to_string(), "rev-parse", "HEAD"],
                None,
            ),
            expected_head,
            "stale proof must not reset, clean, or provision a slot"
        );
    }
    service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("a second observation of the same fetched target should authorize reset");
}

#[test]
fn missing_tracking_ref_on_existing_pool_requires_stable_retry_before_mutation() {
    let repo = TempGitRepo::new("missing-tracking-ref-before-reconcile");
    let service = test_parallel_mode_service();
    let _ = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let slot_heads_before = (1..=DEFAULT_POOL_SIZE)
        .map(|slot_number| {
            let slot_path = repo.pool_root().join(slot_id(slot_number));
            let head = run_command(
                "git",
                ["-C", &slot_path.display().to_string(), "rev-parse", "HEAD"],
                None,
            );
            (slot_path, head)
        })
        .collect::<Vec<_>>();
    run_git(
        &repo.repo_root,
        &["update-ref", "-d", remote_standard_tracking_ref().as_str()],
    );

    let error = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect_err("an absent prior tracking proof must require a stable second observation");

    assert!(error.contains("absent or advanced during freshness verification"));
    for (slot_path, expected_head) in slot_heads_before {
        assert_eq!(
            run_command(
                "git",
                ["-C", &slot_path.display().to_string(), "rev-parse", "HEAD"],
                None,
            ),
            expected_head,
            "a newly recreated tracking ref must not authorize slot mutation"
        );
    }
    service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("a second observation of the recreated tracking ref should authorize reset");
}

// lease write는 authority DB를 먼저 갱신하고 filesystem mirror를 나중에 쓴다. mirror write가
// 실패하면 이미 저장된 authority lease를 되돌려야, 실패한 dispatch가 slot을 영구 점유하지 않는다.
#[test]
fn acquire_slot_lease_rolls_back_authority_when_mirror_write_fails() {
    let repo = TempGitRepo::new("lease-mirror-write-fails");
    let service = test_parallel_mode_service();
    reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    fs::write(repo.pool_root().join(".leases"), "not a directory\n")
        .expect("lease mirror namespace should be blocked by a file");
    let error = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect_err("lease acquisition should fail when mirror path is blocked");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert!(error.contains("failed to persist slot lease"));
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.slots
            .iter()
            .all(|slot| !slot.branch_name.starts_with("akra-agent/"))
    );
}

#[cfg(unix)]
#[test]
fn lifecycle_transition_restores_previous_snapshot_when_mirror_fails_before_install() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TempGitRepo::new("transition-mirror-pre-install-failure");
    let service = test_parallel_mode_service();
    let previous = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let mut next = previous.clone();
    next.state = ParallelModeSlotLeaseState::Running;
    next.running_started_at = Some("2026-07-10T00:00:00Z".to_string());
    let leases_dir = repo.pool_root().join(".leases");
    let original_permissions = fs::metadata(&leases_dir)
        .expect("lease directory metadata should read")
        .permissions();
    fs::set_permissions(&leases_dir, fs::Permissions::from_mode(0o500))
        .expect("lease directory should become read-only");

    let error = transition_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &previous,
        &next,
    )
    .expect_err("mirror creation failure must fail the lifecycle transition");
    fs::set_permissions(&leases_dir, original_permissions)
        .expect("lease directory permissions should restore");

    assert!(error.contains("rollback"), "error: {error}");
    assert_persisted_slot_lease(&repo, &previous);
}

#[cfg(unix)]
#[test]
fn lifecycle_transition_restores_previous_snapshot_after_installed_mirror_reports_error() {
    let repo = TempGitRepo::new("transition-mirror-post-install-failure");
    let service = test_parallel_mode_service();
    let previous = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let mut next = previous.clone();
    next.state = ParallelModeSlotLeaseState::Running;
    next.running_started_at = Some("2026-07-10T00:00:00Z".to_string());
    crate::adapter::outbound::filesystem::secure_fs::install_after_atomic_replace_error_hook(
        "forced post-rename mirror failure",
    );

    let error = transition_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &previous,
        &next,
    )
    .expect_err("post-rename error must roll the installed next snapshot back");

    assert!(
        error.contains("forced post-rename mirror failure"),
        "error: {error}"
    );
    assert!(
        error.contains("exact previous snapshot restored"),
        "error: {error}"
    );
    assert_persisted_slot_lease(&repo, &previous);
}

#[cfg(unix)]
#[test]
fn lifecycle_transition_recreates_a_missing_non_authoritative_mirror() {
    let repo = TempGitRepo::new("transition-missing-mirror");
    let service = test_parallel_mode_service();
    let previous = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    fs::remove_file(repo.slot_lease_path(1)).expect("lease mirror should be removed");
    let mut next = previous.clone();
    next.state = ParallelModeSlotLeaseState::Running;
    next.running_started_at = Some("2026-07-10T00:00:00Z".to_string());

    transition_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &previous,
        &next,
    )
    .expect("authority transition should atomically recreate a missing mirror");

    assert_persisted_slot_lease(&repo, &next);
}

#[cfg(unix)]
#[test]
fn lifecycle_transition_rollback_preserves_concurrent_replacement_generation() {
    let repo = TempGitRepo::new("transition-replacement-generation");
    let service = test_parallel_mode_service();
    let previous = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let mut next = previous.clone();
    next.state = ParallelModeSlotLeaseState::Running;
    next.running_started_at = Some("2026-07-10T00:00:00Z".to_string());
    let mut replacement = previous.clone();
    replacement.agent_id = "agent-2".to_string();
    replacement.task_id = "task-2".to_string();
    replacement.task_title = "Task Two".to_string();
    replacement.lease_generation =
        Some("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_string());
    let replacement_for_hook = replacement.clone();
    let workspace_dir = repo.workspace_dir();
    let mirror_path = repo.slot_lease_path(1);
    crate::adapter::outbound::filesystem::secure_fs::install_before_atomic_replace_hook(
        move || {
            SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(
                &workspace_dir,
                &replacement_for_hook,
            )
            .expect("replacement authority generation should install");
            fs::write(
                &mirror_path,
                serde_json::to_string_pretty(&replacement_for_hook)
                    .expect("replacement mirror should serialize"),
            )
            .expect("replacement mirror generation should install");
        },
    );

    let error = transition_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &previous,
        &next,
    )
    .expect_err("replacement generation must win the mirror transition race");

    assert!(
        error.contains("replacement state preserved"),
        "error: {error}"
    );
    assert_persisted_slot_lease(&repo, &replacement);
}

#[cfg(unix)]
#[test]
fn lifecycle_transition_primary_cas_preserves_preexisting_replacement_generation() {
    let repo = TempGitRepo::new("transition-preexisting-replacement");
    let service = test_parallel_mode_service();
    let previous = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let mut next = previous.clone();
    next.state = ParallelModeSlotLeaseState::Running;
    next.running_started_at = Some("2026-07-10T00:00:00Z".to_string());
    let mut replacement = previous.clone();
    replacement.agent_id = "agent-3".to_string();
    replacement.task_id = "task-3".to_string();
    replacement.task_title = "Task Three".to_string();
    replacement.lease_generation =
        Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string());
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &replacement,
    )
    .expect("replacement generation should install before transition CAS");

    let error = transition_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &previous,
        &next,
    )
    .expect_err("stale previous generation must fail before mirror mutation");

    assert!(error.contains("different generation"), "error: {error}");
    assert_persisted_slot_lease(&repo, &replacement);
}

#[cfg(unix)]
fn assert_persisted_slot_lease(repo: &TempGitRepo, expected: &ParallelModeSlotLeaseSnapshot) {
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projections should load");
    assert_eq!(
        projection.slot_leases.get(&expected.slot_id),
        Some(expected)
    );
    assert_eq!(repo.read_slot_lease(1), *expected);
}

// agent branch는 현재 `prerelease` baseline에서 시작해야 distributor가 rebase 없이
// 같은 기준으로 통합할 수 있다. lease worktree HEAD와 branch ref를 둘 다 검사해
// worktree checkout과 repo branch가 서로 어긋나지 않도록 한다.
#[test]
fn acquire_slot_lease_starts_agent_branch_at_prerelease_head() {
    let repo = TempGitRepo::new("lease-slot-prerelease-start");
    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    repo.commit_on_current_branch(
        "prerelease-only.txt",
        "pool baseline\n",
        "advance prerelease baseline",
    );
    let prerelease_head = repo.head_sha();
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let lease_head = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("leased slot head should resolve");
    let branch_head = run_command(
        "git",
        [
            "-C",
            &repo.workspace_dir(),
            "rev-parse",
            lease.branch_name.as_str(),
        ],
        None,
    )
    .expect("leased agent branch should resolve");

    assert_eq!(lease_head, prerelease_head);
    assert_eq!(branch_head, prerelease_head);
}

// A local integration branch may contain operator-owned commits that have not
// reached the configured remote. Lease creation must preserve that ref while
// starting the agent branch from the freshly fetched remote OID.
#[test]
fn acquire_slot_lease_uses_fetched_remote_oid_when_local_integration_branch_drifted() {
    let repo = TempGitRepo::new("lease-fetched-remote-base");
    let original_branch = current_branch(&repo.repo_root);
    let remote_head = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "rev-parse",
            remote_standard_tracking_ref().as_str(),
        ],
        None,
    )
    .expect("remote integration head should resolve");
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    fs::write(
        repo.repo_root.join("local-integration-only.txt"),
        "preserve local integration work\n",
    )
    .expect("local integration file should be written");
    run_git(&repo.repo_root, &["add", "local-integration-only.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "local integration branch drift"],
    );
    let local_head = repo.head_sha();
    assert_ne!(local_head, remote_head);
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);

    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("lease should use the fetched remote baseline");
    let lease_head = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("lease head should resolve");

    assert_eq!(lease_head, remote_head);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .as_deref(),
        Some(local_head.as_str()),
        "local integration branch must remain untouched"
    );
    assert!(
        !Path::new(&lease.worktree_path)
            .join("local-integration-only.txt")
            .exists()
    );
}

// Read-only inspection cannot know whether a remote-tracking ref is fresh. A
// lease-less agent branch stays blocked until reconcile obtains a same-tick fetch
// proof; the successful reconcile can then clean it safely.
#[test]
fn pool_blocks_legacy_mirror_orphan_until_reconcile_fetches_target() {
    let repo = TempGitRepo::new("stale-lease-mirror");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    assert!(repo.slot_lease_path(1).exists());
    SqlitePlanningAuthorityAdapter::remove_runtime_slot_lease(
        &repo.workspace_dir(),
        &lease.slot_id,
    )
    .expect("authority-store lease should be removed");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let slot = &pool.slots[0];

    assert!(repo.slot_lease_path(1).exists());
    assert_eq!(slot.state, ParallelModePoolSlotState::Blocked);
    assert_eq!(slot.owner_label, "operator recovery");
    assert!(
        slot.worktree_label
            .contains("integration proof unavailable")
    );
    assert!(slot.branch_name.starts_with("akra-agent/slot-1/"));

    let reconciled = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(reconciled.slots[0].state, ParallelModePoolSlotState::Idle);
    assert!(!repo.branch_exists(&lease.branch_name));
    assert!(!repo.slot_lease_path(1).exists());
}

// Git branch 이름은 길이 제한을 넘지 않으면서도 원래 task slug와 안정적으로
// 연결되어야 한다. 긴 slug를 hash suffix로 줄이면 사람이 읽을 앞부분과 충돌 방지
// 정보가 함께 남는다.
#[test]
fn acquire_slot_lease_truncates_long_branch_slug_with_stable_hash() {
    let repo = TempGitRepo::new("lease-slot-long-branch");
    let service = test_parallel_mode_service();
    let long_slug = format!("{}tail", "very-long-task-segment-".repeat(8));
    let sanitized_slug = sanitize_task_slug(&long_slug).expect("long slug should sanitize");

    assert!(sanitized_slug.len() > MAX_AGENT_BRANCH_SLUG_LEN);
    let lease = service
        .acquire_slot_lease_with_test_branch_instance_id(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", &long_slug),
            TEST_BRANCH_INSTANCE_ID,
        )
        .expect("slot lease should be acquired");
    let slug = lease
        .branch_name
        .strip_prefix("akra-agent/slot-1/")
        .expect("slot branch prefix should be present");

    assert!(slug.len() <= MAX_AGENT_BRANCH_SLUG_LEN);
    let bounded_slug = slug
        .strip_suffix(&format!("-{TEST_BRANCH_INSTANCE_ID}"))
        .expect("slot branch should end with its lease instance id");
    assert!(bounded_slug.ends_with(short_branch_slug_hash(&sanitized_slug).as_str()));
    assert!(repo.branch_exists(&lease.branch_name));
}

// 이미 같은 slug의 local branch가 있으면 allocator는 numbered suffix를 붙이되
// 전체 slug limit을 다시 넘기면 안 된다. 긴 slug의 hash suffix가 `-2` 충돌 번호와
// 함께 유지되는지 확인한다.
#[test]
fn allocate_agent_branch_name_numbers_collisions_without_exceeding_slug_limit() {
    let repo = TempGitRepo::new("lease-slot-branch-collision");
    let long_slug = format!("{}tail", "collision-prone-task-segment-".repeat(6));
    let sanitized_slug = sanitize_task_slug(&long_slug).expect("long slug should sanitize");

    assert!(sanitized_slug.len() > MAX_AGENT_BRANCH_SLUG_LEN);
    let first = allocate_agent_branch_name(
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        "slot-1",
        &long_slug,
        "task-1",
        "Task One",
        TEST_BRANCH_INSTANCE_ID,
        &[],
    )
    .expect("configured remote should be valid");
    run_git(&repo.repo_root, &["branch", first.as_str(), "prerelease"]);
    let second = allocate_agent_branch_name(
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        "slot-1",
        &long_slug,
        "task-1",
        "Task One",
        TEST_BRANCH_INSTANCE_ID,
        &[],
    )
    .expect("configured remote should be valid");
    let slug = second
        .strip_prefix("akra-agent/slot-1/")
        .expect("slot branch prefix should be present");
    let base_slug = slug
        .strip_suffix("-2")
        .expect("collision branch should carry a numbered suffix");

    assert_ne!(first, second);
    assert!(slug.len() <= MAX_AGENT_BRANCH_SLUG_LEN);
    let bounded_slug = base_slug
        .strip_suffix(&format!("-{TEST_BRANCH_INSTANCE_ID}"))
        .expect("collision branch should retain its lease instance id");
    assert!(bounded_slug.ends_with(short_branch_slug_hash(&sanitized_slug).as_str()));
}

#[test]
fn allocate_agent_branch_name_is_unique_across_concurrent_lease_instances() {
    let repo = TempGitRepo::new("lease-slot-cross-process-branch-identity");
    let first = allocate_agent_branch_name(
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        "slot-1",
        "task-one",
        "task-1",
        "Task One",
        "0000000000000001",
        &[],
    )
    .expect("first lease branch should allocate");
    let second = allocate_agent_branch_name(
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        "slot-1",
        "task-one",
        "task-1",
        "Task One",
        "0000000000000002",
        &[],
    )
    .expect("second lease branch should allocate independently");

    assert_eq!(first, "akra-agent/slot-1/task-one-0000000000000001");
    assert_eq!(second, "akra-agent/slot-1/task-one-0000000000000002");
    assert_ne!(first, second);
}

// remote-tracking branch만 있어도 이후 push에서 충돌할 수 있다. allocator는 로컬
// branch뿐 아니라 `origin/...` tracking ref도 선점된 이름으로 보고 다음 번호를
// 선택해야 한다.
#[test]
fn allocate_agent_branch_name_numbers_remote_tracking_collisions() {
    let repo = TempGitRepo::new("lease-slot-remote-branch-collision");
    repo.set_remote_tracking_branch(
        &format!("origin/akra-agent/slot-1/task-one-{TEST_BRANCH_INSTANCE_ID}"),
        "prerelease",
    );
    let branch_name = allocate_agent_branch_name(
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        "slot-1",
        "task-one",
        "task-1",
        "Task One",
        TEST_BRANCH_INSTANCE_ID,
        &[],
    )
    .expect("configured remote should be valid");

    assert_eq!(
        branch_name,
        format!("akra-agent/slot-1/task-one-{TEST_BRANCH_INSTANCE_ID}-2")
    );
}

// 실제 remote에만 존재하는 branch도 fetch/tracking 상태에 따라 뒤늦게 충돌할 수
// 있다. live remote ref까지 확인해 새 agent branch가 push 단계에서 reject되지
// 않도록 번호를 올린다.
#[test]
fn allocate_agent_branch_name_numbers_live_remote_collisions() {
    let repo = TempGitRepo::new("lease-slot-live-remote-branch-collision");
    let branch_name = format!("akra-agent/slot-1/task-one-{TEST_BRANCH_INSTANCE_ID}");
    repo.set_remote_only_branch(&branch_name, "refs/heads/prerelease");
    let tracking_ref = format!("refs/remotes/origin/{branch_name}");
    assert!(!command_succeeds(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "show-ref",
            "--verify",
            "--quiet",
            tracking_ref.as_str(),
        ],
    ));
    let lease = test_parallel_mode_service()
        .acquire_slot_lease_with_test_branch_instance_id(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            TEST_BRANCH_INSTANCE_ID,
        )
        .expect("slot lease should inspect the live frozen remote");

    assert_eq!(lease.branch_name, format!("{branch_name}-2"));
}

#[test]
fn acquire_slot_lease_fails_closed_when_live_remote_branches_cannot_be_listed() {
    let repo = TempGitRepo::new("lease-slot-live-remote-listing-failure");
    let service = test_parallel_mode_service_with_github(Arc::new(
        FakeGithubAutomationPort::with_remote_branch_listing_error("remote listing unavailable"),
    ));

    let error = service
        .acquire_slot_lease_with_test_branch_instance_id(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            TEST_BRANCH_INSTANCE_ID,
        )
        .expect_err("remote listing failure must block branch allocation");

    assert!(error.contains("live agent branches could not be inspected"));
    assert!(!repo.branch_exists(&format!(
        "akra-agent/slot-1/task-one-{TEST_BRANCH_INSTANCE_ID}"
    )));
    assert!(!repo.slot_lease_path(1).exists());
}

// slot이 running으로 전환되면 lease state와 started timestamp가 authority store에
// persist되어야 하고, pool board에서도 leased가 아니라 running capacity로 보여야
// 한다. agent process 시작 이후의 기준 상태를 고정한다.
#[test]
fn mark_slot_running_updates_persisted_lease_and_pool_state() {
    let repo = TempGitRepo::new("running-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let running_lease = service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let persisted = repo.read_slot_lease(1);

    assert_eq!(running_lease.state, ParallelModeSlotLeaseState::Running);
    assert!(running_lease.running_started_at.is_some());
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Running);
    assert!(persisted.running_started_at.is_some());
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.running_slots, 1);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Running);
}

// 드물게 terminal completion은 받았지만 TurnStarted 이벤트를 보지 못하면, 완료 파이프라인은
// slot을 Running으로 승격해 결과 capture가 이어지게 해야 한다. 그렇지 않으면 Leased slot이
// orphan 상태로 남아 dispatch capacity를 잃는다.
#[test]
fn terminal_success_without_turn_started_promotes_lease_to_running() {
    let repo = TempGitRepo::new("terminal-success-missing-start");
    let service = test_parallel_mode_service();
    let turn_service =
        crate::application::service::parallel_mode::turn::ParallelModeTurnService::new(
            service.clone(),
        );
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let outcome =
        turn_service.finalize_stream_completion(&lease.worktree_path, false, false, false, false);
    let persisted = repo.read_slot_lease(1);

    assert!(outcome.invalidate_supervisor_snapshot);
    assert!(
        outcome
            .runtime_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("inferred from terminal completion"))
    );
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Running);
}

// TUI/app-server callbacks는 slot id보다 workspace path를 알고 있는 경우가 많다.
// workspace 기반 running 전이는 canonical path lookup으로 같은 lease를 찾아 store와
// mirror를 업데이트해야 한다.
#[test]
fn mark_workspace_slot_running_updates_matching_lease() {
    let repo = TempGitRepo::new("workspace-running-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let running_lease = service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("workspace lease transition should succeed")
        .expect("workspace should have an active lease");
    let persisted = repo.read_slot_lease(1);

    assert_eq!(running_lease.state, ParallelModeSlotLeaseState::Running);
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Running);
    assert!(persisted.running_started_at.is_some());
}

// callbacks가 worktree 하위 디렉터리에서 발생해도 lease를 찾아야 한다. nested
// workspace 입력을 canonical slot root로 되돌려 같은 lease와 workspace path를
// 반환하는지 검증한다.
#[test]
fn resolve_workspace_slot_lease_matches_nested_worktree_directory() {
    let repo = TempGitRepo::new("nested-worktree-resolution");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let nested_workspace = PathBuf::from(&lease.worktree_path).join("nested");
    fs::create_dir_all(&nested_workspace).expect("nested worktree directory should exist");
    let resolution = resolve_workspace_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        nested_workspace
            .to_str()
            .expect("nested workspace should be valid utf-8"),
    )
    .expect("workspace lease lookup should succeed")
    .expect("workspace lease should resolve");

    assert_eq!(resolution.lease.slot_id, lease.slot_id);
    assert_eq!(
        resolution.workspace_path,
        fs::canonicalize(&lease.worktree_path).expect("slot worktree should canonicalize")
    );
}

#[test]
fn resolve_workspace_slot_lease_returns_none_for_unleased_workspace() {
    let repo = TempGitRepo::new("unleased-workspace-resolution");

    let resolution = resolve_workspace_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    )
    .expect("unleased repo workspace should resolve cleanly");

    assert!(resolution.is_none());
}

#[test]
fn resolve_workspace_slot_lease_rejects_non_repository_workspace() {
    let repo = TempGitRepo::new("non-repo-workspace-resolution");
    let non_repo = repo.root.join("plain-directory");
    fs::create_dir_all(&non_repo).expect("plain directory should be created");

    let error = resolve_workspace_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        non_repo.to_str().expect("plain path should be utf-8"),
    )
    .expect_err("plain directory should not resolve as a slot lease");

    assert_eq!(error, "repository inspection failed");
}

#[test]
fn resolve_workspace_slot_lease_rejects_duplicate_matching_authority_leases() {
    let repo = TempGitRepo::new("duplicate-workspace-lease-resolution");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let mut duplicate = lease.clone();
    duplicate.slot_id = "slot-2".to_string();
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&repo.workspace_dir(), &duplicate)
        .expect("duplicate path lease should be persisted");

    let error =
        resolve_workspace_slot_lease(&SqlitePlanningAuthorityAdapter::new(), &lease.worktree_path)
            .expect_err("duplicate lease paths should be rejected");

    assert!(error.contains("matched multiple slot leases"));
}

#[test]
fn resolve_workspace_slot_lease_rejects_branch_drift_for_matching_path() {
    let repo = TempGitRepo::new("workspace-branch-drift-resolution");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    run_git(&slot_path, &["checkout", "-b", "manual-drift"]);

    let error =
        resolve_workspace_slot_lease(&SqlitePlanningAuthorityAdapter::new(), &lease.worktree_path)
            .expect_err("branch drift should not resolve as the active lease");

    assert!(error.contains("is on `manual-drift`"));
    assert!(error.contains(&format!("expects `{}`", lease.branch_name)));
}

// cleanup-ready helper는 branch가 `prerelease`에 통합되기 전에는 아무 것도 바꾸면
// 안 된다. merge 후에만 running lease를 cleanup-pending으로 전환해 premature slot
// 회수를 막는다.
#[test]
fn mark_workspace_slot_cleanup_pending_if_ready_waits_for_integrated_branch() {
    let repo = TempGitRepo::new("workspace-cleanup-ready");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    let pending_before_merge = service
        .mark_workspace_slot_cleanup_pending_if_ready(&lease.worktree_path)
        .expect("cleanup-ready check should succeed before merge");
    assert!(pending_before_merge.is_none());
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::Running
    );

    repo.merge_agent_slot_into_akra(&slot_path);
    let pending_after_merge = service
        .mark_workspace_slot_cleanup_pending_if_ready(&lease.worktree_path)
        .expect("cleanup-ready check should succeed after merge")
        .expect("workspace should transition once branch is integrated");

    assert_eq!(
        pending_after_merge.state,
        ParallelModeSlotLeaseState::CleanupPending
    );
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::CleanupPending
    );
}

#[test]
fn completion_cleanup_apis_preserve_local_result_when_target_fetch_fails() {
    let repo = TempGitRepo::new("completion-cleanup-fetch-proof");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(
        &slot_path,
        "completion-proof.txt",
        "preserve completion result\n",
        "completion proof result",
    );
    let source_commit = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("source commit should resolve");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    leave_stale_integrated_tracking_ref_and_break_fetch(
        &repo,
        &slot_path,
        &source_commit,
        "completion-proof.txt",
    );

    let mark_error = service
        .mark_workspace_slot_cleanup_pending_if_ready(&lease.worktree_path)
        .expect_err("cleanup-pending transition must require a successful target fetch");
    assert!(mark_error.contains("could not be fetched safely"));
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::Running
    );

    let mut cleanup_pending = repo.read_slot_lease(1);
    cleanup_pending.state = ParallelModeSlotLeaseState::CleanupPending;
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &cleanup_pending,
    )
    .expect("cleanup-pending fixture should persist");
    let cleanup_error = service
        .cleanup_workspace_slot_if_pending(&lease.worktree_path)
        .expect_err("destructive cleanup must require a successful target fetch");
    assert!(cleanup_error.contains("could not be fetched safely"));
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
    assert!(slot_path.join("completion-proof.txt").exists());
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::CleanupPending
    );
}

// An ignored build artifact created by a completed worker is outside the frozen source commit.
// It is purged only after integration and the explicit CleanupPending transition.
#[test]
fn cleanup_workspace_slot_if_pending_purges_ignored_output_after_integration() {
    let repo = TempGitRepo::new("workspace-cleanup-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    fs::write(slot_path.join("scratch.tmp"), "transient\n")
        .expect("untracked file should be written");
    let cleaned_lease = service
        .cleanup_workspace_slot_if_pending(&lease.worktree_path)
        .expect("integrated cleanup-pending workspace should clean ignored output")
        .expect("workspace should have an active cleanup-pending lease");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert_eq!(cleaned_lease.slot_id, "slot-1");
    assert_eq!(
        cleaned_lease.state,
        ParallelModeSlotLeaseState::CleanupPending
    );
    assert!(!slot_path.join("scratch.tmp").exists());
    assert!(!repo.branch_exists(&lease.branch_name));
    assert!(!repo.slot_lease_path(1).exists());
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.running_slots, 0);
    assert_eq!(pool.awaiting_cleanup_slots, 0);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Idle);
}

// Durable lease metadata is not sufficient authority to mutate an arbitrary
// worktree. A forged lease that points at the main checkout and a user branch
// must fail before reset, clean, branch deletion, or lease removal.
#[test]
fn cleanup_rejects_forged_main_worktree_lease_identity() {
    let repo = TempGitRepo::new("cleanup-forged-main-worktree");
    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", "-b", "user-review-branch"]);
    repo.commit_on_current_branch("user-result.txt", "preserve user result\n", "user result");
    let user_tip = repo.head_sha();
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    run_git(&repo.repo_root, &["cherry-pick", user_tip.as_str()]);
    run_git(
        &repo.repo_root,
        &["push", "-q", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    run_git(&repo.repo_root, &["checkout", "user-review-branch"]);

    let forged_lease = ParallelModeSlotLeaseSnapshot::new(
        "slot-1",
        "forged-task",
        "Forged Task",
        "forged-agent",
        "user-review-branch",
        repo.workspace_dir(),
        ParallelModeSlotLeaseState::CleanupPending,
        current_timestamp(),
        Some(current_timestamp()),
    );
    fs::create_dir_all(repo.pool_root()).expect("pool root fixture should exist");
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &forged_lease,
    )
    .expect("forged lease fixture should persist");

    let error = test_parallel_mode_service()
        .cleanup_workspace_slot_if_pending(&repo.workspace_dir())
        .expect_err("main worktree must never resolve as a generated pool slot");

    assert!(error.contains("is not the generated path for pool slot"));
    assert_eq!(current_branch(&repo.repo_root), "user-review-branch");
    assert_eq!(repo.head_sha(), user_tip);
    assert_eq!(
        fs::read_to_string(repo.repo_root.join("user-result.txt"))
            .expect("user result must remain readable"),
        "preserve user result\n"
    );
    assert!(repo.branch_exists("user-review-branch"));
    assert!(repo.slot_lease_path(1).exists());
    assert_ne!(original_branch, "user-review-branch");
}

// `git cherry` does not report merge commits. When both parent patches have
// equivalents on the integration branch but the merge commit adds its own
// resolution, cleanup must preserve the source branch and its last local result.
#[test]
fn cleanup_preserves_patch_equivalent_merge_with_unique_resolution() {
    let repo = TempGitRepo::new("cleanup-merge-resolution-proof");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(
        &slot_path,
        "agent-parent.txt",
        "agent parent\n",
        "agent parent patch",
    );
    let agent_parent = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("agent parent should resolve");

    let original_branch = current_branch(&repo.repo_root);
    run_git(
        &repo.repo_root,
        &["checkout", "-b", "merge-side", POOL_BASELINE_BRANCH],
    );
    repo.commit_on_current_branch("side-parent.txt", "side parent\n", "side parent patch");
    let side_parent = repo.head_sha();
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);

    run_git(
        &slot_path,
        &["merge", "--no-ff", "--no-commit", "merge-side"],
    );
    fs::write(
        slot_path.join("merge-resolution.txt"),
        "unique resolution\n",
    )
    .expect("merge resolution should be written");
    run_git(&slot_path, &["add", "merge-resolution.txt"]);
    run_git(
        &slot_path,
        &["commit", "-qm", "merge with unique resolution"],
    );
    let merge_tip = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("merge tip should resolve");

    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    run_git(
        &repo.repo_root,
        &["cherry-pick", agent_parent.as_str(), side_parent.as_str()],
    );
    run_git(
        &repo.repo_root,
        &["push", "-q", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);

    let mut cleanup_pending = repo.read_slot_lease(1);
    cleanup_pending.state = ParallelModeSlotLeaseState::CleanupPending;
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &cleanup_pending,
    )
    .expect("cleanup-pending fixture should persist");

    let error = service
        .cleanup_workspace_slot_if_pending(&lease.worktree_path)
        .expect_err("merge-only resolution must not be deleted by patch equivalence");

    assert!(error.contains("could not be reset"));
    assert!(repo.branch_exists(&lease.branch_name));
    assert_eq!(current_branch(&slot_path), lease.branch_name);
    assert_eq!(
        run_command(
            "git",
            ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
            None,
        )
        .as_deref(),
        Some(merge_tip.as_str())
    );
    assert_eq!(
        fs::read_to_string(slot_path.join("merge-resolution.txt"))
            .expect("merge resolution must remain readable"),
        "unique resolution\n"
    );
    assert!(repo.slot_lease_path(1).exists());
}

// Merge history is safe to clean when the exact source tip is already reachable
// from the freshly fetched integration target. The merge-range fail-closed guard
// must not block this stronger proof.
#[test]
fn cleanup_allows_exact_ancestor_merge_history() {
    let repo = TempGitRepo::new("cleanup-exact-ancestor-merge");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(
        &slot_path,
        "agent-parent.txt",
        "agent parent\n",
        "agent parent patch",
    );

    let original_branch = current_branch(&repo.repo_root);
    run_git(
        &repo.repo_root,
        &["checkout", "-b", "exact-side", POOL_BASELINE_BRANCH],
    );
    repo.commit_on_current_branch("side-parent.txt", "side parent\n", "side parent patch");
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);
    run_git(
        &slot_path,
        &["merge", "--no-ff", "-m", "merge exact side", "exact-side"],
    );

    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("exact-ancestor merge should transition to cleanup pending");

    let cleaned = service
        .cleanup_workspace_slot_if_pending(&lease.worktree_path)
        .expect("exact-ancestor merge should clean safely")
        .expect("cleanup should return the released lease");

    assert_eq!(cleaned.slot_id, lease.slot_id);
    assert!(!repo.branch_exists(&lease.branch_name));
    assert!(!repo.slot_lease_path(1).exists());
    assert_eq!(current_branch(&slot_path), "HEAD");
    assert!(slot_path.join("agent-parent.txt").exists());
    assert!(slot_path.join("side-parent.txt").exists());
}

#[test]
fn cleanup_preserves_late_commit_created_after_integration_proof() {
    let repo = TempGitRepo::new("cleanup-late-commit-cas");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(&slot_path, "result.txt", "result\n", "agent result");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter running state");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter cleanup pending");
    let baseline_oid = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "rev-parse",
            remote_standard_tracking_ref().as_str(),
        ],
        None,
    )
    .expect("integration target should resolve");
    let repo_root = repo.workspace_dir();
    let pool_root = repo.pool_root();
    let canonical_repo_root = repo.canonical_repo_root();
    let identity = PoolSlotCleanupIdentity::new(
        &repo_root,
        &canonical_repo_root,
        &pool_root,
        &lease.slot_id,
        &slot_path,
        &lease.branch_name,
    );
    fs::write(
        slot_path.join("completed-build.tmp"),
        "completed build output\n",
    )
    .expect("pre-cleanup ignored output should write");

    let cleaned = cleanup_slot_to_ref_with_hooks(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &identity,
        &baseline_oid,
        || {
            assert!(
                !slot_path.join("completed-build.tmp").exists(),
                "ignored output must be purged before the late-writer hook runs"
            );
            repo.commit_file_in_slot(
                &slot_path,
                "late-commit.txt",
                "late commit\n",
                "late commit after proof",
            );
        },
        || {},
    );

    assert!(!cleaned);
    assert_eq!(current_branch(&slot_path), lease.branch_name);
    assert!(repo.branch_exists(&lease.branch_name));
    assert!(!slot_path.join("completed-build.tmp").exists());
    assert!(slot_path.join("late-commit.txt").exists());
    assert!(repo.slot_lease_path(1).exists());
}

#[test]
fn cleanup_preserves_replacement_leased_generation_written_after_cleanup_proof() {
    let repo = TempGitRepo::new("cleanup-replacement-generation-cas");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(&slot_path, "result.txt", "result\n", "agent result");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter running state");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter cleanup pending");
    let baseline_oid = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "rev-parse",
            remote_standard_tracking_ref().as_str(),
        ],
        None,
    )
    .expect("integration target should resolve");
    let repo_root = repo.workspace_dir();
    let pool_root = repo.pool_root();
    let canonical_repo_root = repo.canonical_repo_root();
    let identity = PoolSlotCleanupIdentity::new(
        &repo_root,
        &canonical_repo_root,
        &pool_root,
        &lease.slot_id,
        &slot_path,
        &lease.branch_name,
    );
    let mut replacement = lease.clone();
    replacement.agent_id = "agent-2".to_string();
    replacement.task_id = "task-2".to_string();
    replacement.task_title = "Task Two".to_string();
    replacement.state = ParallelModeSlotLeaseState::Leased;
    replacement.running_started_at = None;
    replacement.leased_at = "2099-01-01T00:00:00Z".to_string();
    replacement.lease_generation =
        Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string());
    let replacement_for_hook = replacement.clone();

    let cleaned = cleanup_slot_to_ref_with_hooks(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &identity,
        &baseline_oid,
        || {
            write_slot_lease(
                &SqlitePlanningAuthorityAdapter::new(),
                &test_parallel_runtime(),
                &repo_root,
                &pool_root,
                &replacement_for_hook,
            )
            .expect("replacement generation should persist before cleanup CAS");
        },
        || {},
    );

    assert!(!cleaned);
    assert_eq!(current_branch(&slot_path), lease.branch_name);
    assert!(repo.branch_exists(&lease.branch_name));
    let projection = SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo_root)
        .expect("replacement authority projection should remain");
    assert_eq!(
        projection.slot_leases.get(&replacement.slot_id),
        Some(&replacement)
    );
    let mirror: ParallelModeSlotLeaseSnapshot = serde_json::from_str(
        &fs::read_to_string(repo.slot_lease_path(1))
            .expect("replacement mirror should remain readable"),
    )
    .expect("replacement mirror should contain lease JSON");
    assert_eq!(mirror, replacement);
}

#[cfg(unix)]
#[test]
fn cleanup_blocks_hostile_smudge_filter_without_executing_or_mutating_slot() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TempGitRepo::new("cleanup-hostile-smudge-filter");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    fs::write(slot_path.join(".gitattributes"), "*.txt filter=hostile\n")
        .expect("filter attributes should write");
    fs::write(slot_path.join("filtered.txt"), "safe committed content\n")
        .expect("filtered source should write");
    run_git(&slot_path, &["add", ".gitattributes", "filtered.txt"]);
    run_git(&slot_path, &["commit", "-qm", "agent filtered result"]);
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter running state");
    repo.merge_agent_slot_into_akra(&slot_path);
    let cleanup_pending = service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter cleanup pending");

    let marker = repo.root.join("hostile-smudge-executed");
    let script = repo.root.join("hostile-smudge.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf executed >> '{}'\ncat\n",
            marker.display()
        ),
    )
    .expect("hostile smudge script should write");
    let mut permissions = fs::metadata(&script)
        .expect("hostile smudge metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions)
        .expect("hostile smudge script should become executable");
    run_git(
        &repo.repo_root,
        &[
            "config",
            "filter.hostile.smudge",
            script.to_str().expect("script path should be UTF-8"),
        ],
    );
    run_git(
        &repo.repo_root,
        &["config", "filter.hostile.required", "true"],
    );
    let source_head = resolve_workspace_head_sha(&slot_path).expect("source head should resolve");

    let error = service
        .cleanup_workspace_slot_if_pending(&lease.worktree_path)
        .expect_err("hostile smudge config must block cleanup before checkout");

    assert!(error.contains("filter.hostile.smudge"), "error: {error}");
    assert!(!marker.exists(), "hostile smudge command must not execute");
    assert_eq!(current_branch(&slot_path), lease.branch_name);
    assert_eq!(
        resolve_workspace_head_sha(&slot_path).as_deref(),
        Some(source_head.as_str())
    );
    assert!(repo.branch_exists(&lease.branch_name));
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("cleanup-pending lease should remain");
    assert_eq!(
        projection.slot_leases.get(&lease.slot_id),
        Some(&cleanup_pending)
    );
}

#[test]
fn cleanup_preserves_late_ignored_write_after_safe_detach() {
    let repo = TempGitRepo::new("cleanup-late-ignored-write");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(&slot_path, "result.txt", "result\n", "agent result");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter running state");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter cleanup pending");
    let baseline_oid = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "rev-parse",
            remote_standard_tracking_ref().as_str(),
        ],
        None,
    )
    .expect("integration target should resolve");
    let repo_root = repo.workspace_dir();
    let pool_root = repo.pool_root();
    let canonical_repo_root = repo.canonical_repo_root();
    let identity = PoolSlotCleanupIdentity::new(
        &repo_root,
        &canonical_repo_root,
        &pool_root,
        &lease.slot_id,
        &slot_path,
        &lease.branch_name,
    );

    let cleaned = cleanup_slot_to_ref_with_hooks(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &identity,
        &baseline_oid,
        || {},
        || {
            fs::write(slot_path.join("late-ignored.tmp"), "late ignored write\n")
                .expect("late ignored file should be written");
        },
    );

    assert!(!cleaned);
    assert_eq!(current_branch(&slot_path), "HEAD");
    assert!(repo.branch_exists(&lease.branch_name));
    assert_eq!(
        fs::read_to_string(slot_path.join("late-ignored.tmp"))
            .expect("late ignored file must remain readable"),
        "late ignored write\n"
    );
    assert!(repo.slot_lease_path(1).exists());
}

#[test]
fn cleanup_never_purges_ignored_output_without_the_matching_durable_lease() {
    let repo = TempGitRepo::new("cleanup-ignored-without-authority-lease");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(&slot_path, "result.txt", "result\n", "agent result");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter running state");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter cleanup pending");
    SqlitePlanningAuthorityAdapter::remove_runtime_slot_lease(
        &repo.workspace_dir(),
        &lease.slot_id,
    )
    .expect("authority lease should be removed to reproduce split brain");
    fs::write(
        slot_path.join("orphaned-build.tmp"),
        "preserve without lease\n",
    )
    .expect("ignored split-brain output should write");
    let baseline_oid = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "rev-parse",
            remote_standard_tracking_ref().as_str(),
        ],
        None,
    )
    .expect("integration target should resolve");
    let repo_root = repo.workspace_dir();
    let pool_root = repo.pool_root();
    let canonical_repo_root = repo.canonical_repo_root();
    let identity = PoolSlotCleanupIdentity::new(
        &repo_root,
        &canonical_repo_root,
        &pool_root,
        &lease.slot_id,
        &slot_path,
        &lease.branch_name,
    );

    assert!(!cleanup_slot_to_ref_with_hooks(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &identity,
        &baseline_oid,
        || {},
        || {},
    ));
    assert_eq!(current_branch(&slot_path), lease.branch_name);
    assert!(repo.branch_exists(&lease.branch_name));
    assert_eq!(
        fs::read_to_string(slot_path.join("orphaned-build.tmp"))
            .expect("ignored split-brain output must remain inspectable"),
        "preserve without lease\n"
    );
}

#[test]
fn atomic_branch_delete_rejects_source_ref_movement() {
    let repo = TempGitRepo::new("cleanup-source-ref-cas");
    let slot_path = repo.create_agent_slot(1, "source-cas");
    let branch_name = current_branch(&slot_path);
    let frozen_source = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be utf-8"),
            "rev-parse",
            "HEAD",
        ],
        None,
    )
    .expect("frozen source should resolve");
    repo.commit_file_in_slot(
        &slot_path,
        "late-source.txt",
        "late source\n",
        "move source after freeze",
    );
    let moved_source = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be utf-8"),
            "rev-parse",
            "HEAD",
        ],
        None,
    )
    .expect("moved source should resolve");

    assert!(!delete_cleaned_slot_branch_if_unchanged(
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &branch_name,
        &frozen_source,
    ));
    assert!(repo.branch_exists(&branch_name));
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "rev-parse",
                branch_name.as_str(),
            ],
            None,
        )
        .as_deref(),
        Some(moved_source.as_str())
    );
    assert!(slot_path.join("late-source.txt").exists());
}

#[test]
fn generic_slot_reset_preserves_ignored_write_and_reports_failure() {
    let repo = TempGitRepo::new("generic-reset-late-write");
    let slot_path = repo.create_detached_slot(1);
    fs::write(slot_path.join("late-reset.tmp"), "late reset write\n")
        .expect("ignored late write should be created");
    let report =
        reset_slot_worktree_to_ref(&test_parallel_runtime(), &slot_path, POOL_BASELINE_BRANCH);

    assert!(!report.succeeded());
    assert_eq!(
        fs::read_to_string(slot_path.join("late-reset.tmp"))
            .expect("ignored late write must remain readable"),
        "late reset write\n"
    );
}

fn assert_managed_ancestor_alias_is_rejected(
    repo: &TempGitRepo,
    create_alias: impl FnOnce(&Path, &Path),
) {
    let canonical_repo_root = repo.canonical_repo_root();
    let pool_root = derive_default_pool_root(&canonical_repo_root);
    let managed_sibling_root = pool_root
        .parent()
        .and_then(Path::parent)
        .expect("managed sibling root should resolve");
    let external_root = repo.root.join("external-managed-root");
    fs::create_dir_all(&external_root).expect("external root should be created");
    create_alias(&external_root, managed_sibling_root);
    let slot_path = pool_root.join(slot_id(1));
    fs::create_dir_all(slot_path.parent().expect("slot parent should exist"))
        .expect("slot parent should be created through alias");
    let branch_name = "akra-agent/slot-1/aliased-tree";
    run_git(
        &repo.repo_root,
        &[
            "worktree",
            "add",
            "-b",
            branch_name,
            slot_path.to_str().expect("slot path should be utf-8"),
            POOL_BASELINE_BRANCH,
        ],
    );
    let lease = ParallelModeSlotLeaseSnapshot::new(
        "slot-1",
        "alias-task",
        "Alias Task",
        "alias-agent",
        branch_name,
        slot_path.display().to_string(),
        ParallelModeSlotLeaseState::CleanupPending,
        current_timestamp(),
        Some(current_timestamp()),
    );
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &pool_root,
        &lease,
    )
    .expect("aliased lease fixture should persist");
    let repo_root = repo.workspace_dir();
    let identity = PoolSlotCleanupIdentity::new(
        &repo_root,
        &canonical_repo_root,
        &pool_root,
        &lease.slot_id,
        &slot_path,
        &lease.branch_name,
    );
    let error = identity
        .validate(&test_parallel_runtime())
        .expect_err("managed ancestor alias must fail closed");

    assert!(error.contains("symlink or reparse point"));
    assert!(!cleanup_slot_to_ref_with_hooks(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &identity,
        POOL_BASELINE_BRANCH,
        || {},
        || {},
    ));
    assert!(repo.branch_exists(branch_name));
    assert!(slot_path.join("README.md").exists());
    assert!(repo.slot_lease_path(1).exists());
}

#[cfg(unix)]
#[test]
fn cleanup_rejects_symlinked_managed_ancestor() {
    use std::os::unix::fs::symlink;

    let repo = TempGitRepo::new("cleanup-managed-symlink");
    assert_managed_ancestor_alias_is_rejected(&repo, |target, link| {
        symlink(target, link).expect("managed sibling symlink should be created");
    });
}

#[cfg(windows)]
#[test]
fn cleanup_rejects_junction_managed_ancestor() {
    let repo = TempGitRepo::new("cleanup-managed-junction");
    assert_managed_ancestor_alias_is_rejected(&repo, |target, link| {
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .status()
            .expect("junction creation should spawn");
        assert!(
            status.success(),
            "managed sibling junction should be created"
        );
    });
}

// agent start가 실패했고 worktree가 아직 깨끗하면 lease를 안전하게 되돌릴 수 있다.
// branch와 mirror를 제거하고 pool capacity를 idle로 복구하는 실패 시작 경로다.
#[test]
fn release_workspace_slot_lease_after_failed_start_resets_clean_slot_to_idle() {
    let repo = TempGitRepo::new("release-unstarted-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let released_lease = service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect("clean unstarted slot should be released")
        .expect("workspace should have an active lease");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert_eq!(released_lease.slot_id, "slot-1");
    assert_eq!(released_lease.state, ParallelModeSlotLeaseState::Leased);
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Idle);
}

// 같은 실패 시작 경로라도 worktree가 dirty하면 자동 release가 사용자의 산출물을
// 잃을 수 있다. dirty file이 있는 경우 lease와 branch를 보존하고 명시적 오류를
// 돌려 operator recovery로 넘긴다.
#[test]
fn release_workspace_slot_lease_after_failed_start_rejects_dirty_worktree() {
    let repo = TempGitRepo::new("release-dirty-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    fs::write(
        Path::new(&lease.worktree_path).join("dirty.txt"),
        "scratch\n",
    )
    .expect("worktree should become dirty");
    let error = service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect_err("dirty unstarted slot should stay leased");

    assert!(error.contains("could not be released after startup failure"));
    assert!(repo.slot_lease_path(1).exists());
    assert!(repo.branch_exists(&lease.branch_name));
}

// cleanup-pending 전이는 running 상태와 branch integration이 모두 필요하다. 이
// guard가 없으면 아직 실행 전이거나 미통합인 branch를 slot cleanup 대상으로 잘못
// 분류할 수 있다.
#[test]
fn mark_slot_cleanup_pending_requires_running_state_and_merged_branch() {
    let repo = TempGitRepo::new("cleanup-pending-guards");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    let not_running_error = service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect_err("cleanup pending should require the running state");
    assert!(not_running_error.contains("has not entered running state"));

    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    let not_merged_error = service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect_err("cleanup pending should require an integrated branch");
    assert!(not_merged_error.contains("is not integrated into `prerelease` yet"));
}

#[test]
fn slot_lifecycle_cleanup_transition_rejects_stale_tracking_without_fetch_proof() {
    let repo = TempGitRepo::new("slot-cleanup-fetch-proof");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(
        &slot_path,
        "lifecycle-proof.txt",
        "preserve lifecycle result\n",
        "lifecycle proof result",
    );
    let source_commit = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("source commit should resolve");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    leave_stale_integrated_tracking_ref_and_break_fetch(
        &repo,
        &slot_path,
        &source_commit,
        "lifecycle-proof.txt",
    );

    let error = service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect_err("slot lifecycle must reject a stale tracking ref without fresh proof");

    assert!(error.contains("could not be fetched safely"));
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::Running
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
    assert!(slot_path.join("lifecycle-proof.txt").exists());
}

// 정상 cleanup-pending 전이는 persisted lease와 pool board를 동시에 바꾼다. running
// count가 내려가고 awaiting cleanup count가 올라가야 distributor가 slot 회수를
// 다음 단계로 진행할 수 있다.
#[test]
fn mark_slot_cleanup_pending_updates_persisted_lease_and_pool_state() {
    let repo = TempGitRepo::new("cleanup-pending-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.merge_agent_slot_into_akra(&slot_path);
    let cleanup_pending_lease = service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let persisted = repo.read_slot_lease(1);

    assert_eq!(
        cleanup_pending_lease.state,
        ParallelModeSlotLeaseState::CleanupPending
    );
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::CleanupPending);
    assert_eq!(pool.awaiting_cleanup_slots, 1);
    assert_eq!(pool.running_slots, 0);
    assert_eq!(
        pool.slots[0].state,
        ParallelModePoolSlotState::AwaitingCleanup
    );
    assert_eq!(pool.slots[0].owner_label, "agent-1 / task-1");
}

// cleanup-pending으로 표시된 뒤 agent branch에 새로운 미통합 commit이 생기면
// reconcile이 slot을 지우면 안 된다. branch와 lease mirror를 보존해 late work가
// operator 확인 없이 사라지지 않도록 한다.
#[test]
fn reconcile_does_not_cleanup_pending_slot_with_new_unintegrated_commit() {
    let repo = TempGitRepo::new("cleanup-pending-reverify");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    let branch_name = lease.branch_name.clone();
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter running state");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter cleanup pending");
    repo.commit_file_in_slot(
        &slot_path,
        "late-change.txt",
        "late work\n",
        "late cleanup pending change",
    );
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let slot = pool
        .slots
        .iter()
        .find(|slot| slot.slot_id == lease.slot_id)
        .expect("slot should be present");

    assert!(repo.branch_exists(&branch_name));
    assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
    assert!(repo.slot_lease_path(1).exists());
}

fn acquire_running_worker_lease(
    service: &ParallelModeService,
    repo: &TempGitRepo,
    task_id: &str,
) -> ParallelModeSlotLeaseSnapshot {
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request(task_id, "Host commit task", "agent-host", "host-commit"),
        )
        .expect("worker slot should lease");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("worker slot should become running");
    lease
}

#[test]
fn host_owned_worker_commit_stages_one_commit_and_leaves_a_clean_worktree() {
    let repo = TempGitRepo::new("host-worker-commit");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-commit");
    let base = resolve_workspace_head_sha(Path::new(&lease.worktree_path))
        .expect("leased slot HEAD should resolve");
    fs::write(
        Path::new(&lease.worktree_path).join("worker-result.txt"),
        "completed by worker\n",
    )
    .expect("worker result should write");

    let outcome = service
        .prepare_workspace_worker_commit(&lease)
        .expect("host should commit the worker result");

    assert_eq!(
        outcome.disposition,
        ParallelWorkerCommitDisposition::Created
    );
    assert_ne!(outcome.commit_sha, base);
    let commit_identity = run_command(
        "git",
        [
            "-C",
            lease.worktree_path.as_str(),
            "show",
            "-s",
            "--format=%an%x00%ae%x00%cn%x00%ce%x00%at%x00%ct",
            outcome.commit_sha.as_str(),
        ],
        None,
    )
    .expect("host commit identity and timestamp should resolve");
    let commit_timestamp = DateTime::parse_from_rfc3339(&lease.leased_at)
        .expect("lease timestamp should be RFC3339")
        .timestamp()
        .to_string();
    assert_eq!(
        commit_identity.split('\0').collect::<Vec<_>>(),
        vec![
            "Akra Parallel Worker",
            "akra@localhost.invalid",
            "Akra Parallel Worker",
            "akra@localhost.invalid",
            commit_timestamp.as_str(),
            commit_timestamp.as_str(),
        ],
        "caller-owned GIT_AUTHOR/COMMITTER values set after sanitization must survive"
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                lease.worktree_path.as_str(),
                "rev-list",
                "--count",
                &format!("{base}..{}", outcome.commit_sha),
            ],
            None,
        )
        .as_deref(),
        Some("1")
    );
    assert!(
        run_command(
            "git",
            [
                "-C",
                lease.worktree_path.as_str(),
                "status",
                "--porcelain=v1",
            ],
            None,
        )
        .is_none(),
        "host commit should leave no staged, unstaged, or untracked files"
    );
}

#[cfg(unix)]
#[test]
fn host_owned_worker_commit_rejects_external_hardlinks_and_special_files() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let repo = TempGitRepo::new("host-worker-hardlink-special-files");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-hardlink");
    let external_secret = repo.root.join("outside-worker-secret");
    fs::write(&external_secret, "sensitive outside content\n")
        .expect("external secret fixture should write");
    fs::hard_link(
        &external_secret,
        Path::new(&lease.worktree_path).join("leaked-secret.txt"),
    )
    .expect("hardlink fixture should be created");

    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("hard-linked external file must fail closed");
    assert!(error.contains("hard-linked file"), "error: {error}");

    fs::remove_file(Path::new(&lease.worktree_path).join("leaked-secret.txt"))
        .expect("hardlink fixture should be removed");
    let fifo_path = Path::new(&lease.worktree_path).join("worker.pipe");
    let fifo = CString::new(fifo_path.as_os_str().as_bytes()).expect("FIFO path should be valid");
    // SAFETY: mkfifo receives a valid NUL-terminated path and a fixed permission mode.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("FIFO input must fail before git add can open it");
    assert!(error.contains("unsupported type"), "error: {error}");
}

#[test]
fn host_owned_worker_commit_rejects_clean_nested_repository_gitlinks() {
    let repo = TempGitRepo::new("host-worker-nested-gitlink");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-gitlink");
    let nested = Path::new(&lease.worktree_path).join("nested-repository");
    fs::create_dir_all(&nested).expect("nested repository directory should be created");
    run_git(&nested, &["init", "-q"]);
    run_git(&nested, &["config", "user.name", "Nested"]);
    run_git(&nested, &["config", "user.email", "nested@example.invalid"]);
    fs::write(nested.join("nested.txt"), "nested\n").expect("nested file should write");
    run_git(&nested, &["add", "nested.txt"]);
    run_git(&nested, &["commit", "-qm", "nested commit"]);

    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("clean nested repository must not become an implicit gitlink");
    assert!(
        error.contains("unsupported type") || error.contains("Git gitlink"),
        "error: {error}"
    );
}

#[test]
fn host_owned_worker_commit_enforces_file_and_aggregate_byte_limits() {
    let repo = TempGitRepo::new("host-worker-size-limits");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-size-limit");
    let oversized = fs::File::create(Path::new(&lease.worktree_path).join("oversized.bin"))
        .expect("oversized sparse file should create");
    oversized
        .set_len(64 * 1024 * 1024 + 1)
        .expect("oversized sparse file should resize");
    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("oversized file must fail before staging");
    assert!(error.contains("64 MiB per-file limit"), "error: {error}");

    fs::remove_file(Path::new(&lease.worktree_path).join("oversized.bin"))
        .expect("oversized file should be removed");
    for index in 0..5 {
        let file = fs::File::create(
            Path::new(&lease.worktree_path).join(format!("aggregate-{index}.bin")),
        )
        .expect("aggregate sparse file should create");
        file.set_len(52 * 1024 * 1024)
            .expect("aggregate sparse file should resize");
    }
    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("aggregate changed bytes must be bounded");
    assert!(error.contains("256 MiB aggregate limit"), "error: {error}");
}

#[test]
fn host_owned_worker_commit_rejects_no_change_and_zero_tree_existing_commit() {
    let repo = TempGitRepo::new("host-worker-no-change");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-no-change");
    let base = resolve_workspace_head_sha(Path::new(&lease.worktree_path))
        .expect("leased slot HEAD should resolve");

    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("unchanged worker result must not become commit-ready");
    assert!(error.contains("no tree changes"), "error: {error}");

    run_git(
        Path::new(&lease.worktree_path),
        &["commit", "--allow-empty", "-qm", "empty worker commit"],
    );
    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("empty existing commit must not count as a worker result");
    assert!(error.contains("no tree changes"), "error: {error}");
    assert_ne!(
        resolve_workspace_head_sha(Path::new(&lease.worktree_path)).as_deref(),
        Some(base.as_str()),
        "the rejected existing commit remains inspectable"
    );
}

#[test]
fn host_owned_worker_commit_accepts_a_clean_existing_linear_commit() {
    let repo = TempGitRepo::new("host-worker-existing-commit");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-existing");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(
        &slot_path,
        "existing-result.txt",
        "already committed\n",
        "existing worker result",
    );
    let existing_head =
        resolve_workspace_head_sha(&slot_path).expect("existing HEAD should resolve");

    let outcome = service
        .prepare_workspace_worker_commit(&lease)
        .expect("clean existing worker commit should remain compatible");

    assert_eq!(
        outcome.disposition,
        ParallelWorkerCommitDisposition::Existing
    );
    assert_eq!(outcome.commit_sha, existing_head);
}

#[test]
fn host_owned_worker_commit_rejects_existing_merge_history() {
    let repo = TempGitRepo::new("host-worker-existing-merge");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-merge");
    let slot_path = Path::new(&lease.worktree_path);
    run_git(slot_path, &["checkout", "-qb", "worker-side-branch"]);
    fs::write(slot_path.join("side.txt"), "side\n").expect("side result should write");
    run_git(slot_path, &["add", "side.txt"]);
    run_git(slot_path, &["commit", "-qm", "side result"]);
    run_git(slot_path, &["checkout", "-q", lease.branch_name.as_str()]);
    fs::write(slot_path.join("main.txt"), "main\n").expect("main result should write");
    run_git(slot_path, &["add", "main.txt"]);
    run_git(slot_path, &["commit", "-qm", "main result"]);
    run_git(
        slot_path,
        &[
            "merge",
            "--no-ff",
            "-qm",
            "merge side result",
            "worker-side-branch",
        ],
    );

    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("merge history must not become a host-owned worker result");
    assert!(error.contains("merge commit"), "error: {error}");
}

#[test]
fn host_owned_worker_commit_rejects_hidden_index_entries_and_preserves_the_running_slot() {
    for (flag, label) in [
        ("--assume-unchanged", "assume-unchanged"),
        ("--skip-worktree", "skip-worktree"),
    ] {
        let repo = TempGitRepo::new(&format!("host-worker-{label}"));
        let service = test_parallel_mode_service();
        let lease =
            acquire_running_worker_lease(&service, &repo, &format!("task-host-worker-{label}"));
        let slot_path = Path::new(&lease.worktree_path);
        let base = resolve_workspace_head_sha(slot_path).expect("leased slot HEAD should resolve");
        run_git(slot_path, &["update-index", flag, "README.md"]);
        fs::write(slot_path.join("README.md"), format!("hidden by {label}\n"))
            .expect("hidden tracked result should write");
        fs::write(slot_path.join("visible-result.txt"), "visible result\n")
            .expect("visible worker result should write");

        let error = service
            .prepare_workspace_worker_commit(&lease)
            .expect_err("hidden index entries must block host commit");

        assert!(
            error.contains("assume-unchanged or skip-worktree"),
            "error: {error}"
        );
        assert_eq!(
            resolve_workspace_head_sha(slot_path).as_deref(),
            Some(base.as_str()),
            "host must not advance the branch after a hidden index entry"
        );
        service
            .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
            .expect("parallel reset should preserve a running blocked slot");
        assert_eq!(
            fs::read_to_string(slot_path.join("README.md"))
                .expect("hidden tracked result should remain inspectable"),
            format!("hidden by {label}\n")
        );
        assert_eq!(
            repo.read_slot_lease(1).state,
            ParallelModeSlotLeaseState::Running,
            "blocked host commit must not make the slot reusable"
        );
    }
}

#[test]
fn host_owned_worker_commit_allows_ignored_top_level_build_output() {
    let repo = TempGitRepo::new("host-worker-ignored-output");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-ignored-output");
    let slot_path = Path::new(&lease.worktree_path);
    fs::write(slot_path.join("worker-result.txt"), "visible result\n")
        .expect("visible worker result should write");
    fs::write(slot_path.join("generated.tmp"), "ignored output\n")
        .expect("ignored worker output should write");

    let outcome = service
        .prepare_workspace_worker_commit(&lease)
        .expect("ignored build output must remain outside the source commit");

    assert_eq!(
        outcome.disposition,
        ParallelWorkerCommitDisposition::Created
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                lease.worktree_path.as_str(),
                "show",
                "--format=",
                "--name-only",
                outcome.commit_sha.as_str(),
            ],
            None,
        )
        .as_deref(),
        Some("worker-result.txt"),
        "ignored build output must not enter the frozen source commit"
    );
    assert!(slot_path.join("generated.tmp").exists());
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::Running
    );
}

#[test]
fn host_owned_worker_commit_overrides_submodule_ignore_and_rejects_hidden_state() {
    let repo = TempGitRepo::new("host-worker-hidden-submodule");
    let submodule_source = repo.root.join("submodule-source");
    fs::create_dir_all(&submodule_source).expect("submodule source should be created");
    run_git(&submodule_source, &["init", "-q"]);
    run_git(
        &submodule_source,
        &["config", "user.name", "Submodule Test"],
    );
    run_git(
        &submodule_source,
        &["config", "user.email", "submodule@example.invalid"],
    );
    fs::write(submodule_source.join("tracked.txt"), "clean\n")
        .expect("submodule tracked file should write");
    fs::write(submodule_source.join(".gitignore"), "*.tmp\n")
        .expect("submodule ignore file should write");
    run_git(&submodule_source, &["add", "tracked.txt", ".gitignore"]);
    run_git(&submodule_source, &["commit", "-qm", "submodule seed"]);

    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", "-q", POOL_BASELINE_BRANCH]);
    run_git(
        &repo.repo_root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            submodule_source
                .to_str()
                .expect("submodule source path should be UTF-8"),
            "vendor/child",
        ],
    );
    run_git(
        &repo.repo_root,
        &[
            "config",
            "-f",
            ".gitmodules",
            "submodule.vendor/child.ignore",
            "all",
        ],
    );
    run_git(&repo.repo_root, &["add", ".gitmodules", "vendor/child"]);
    run_git(&repo.repo_root, &["commit", "-qm", "add ignored submodule"]);
    run_git(
        &repo.repo_root,
        &["push", "-q", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    repo.set_remote_tracking_branch(&remote_standard_branch_name(), POOL_BASELINE_BRANCH);
    run_git(&repo.repo_root, &["checkout", "-q", &original_branch]);

    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-hidden-submodule");
    let slot_path = Path::new(&lease.worktree_path);
    let mut submodule_update_args = vec!["-c", "protocol.file.allow=always"];
    #[cfg(windows)]
    submodule_update_args.extend(["-c", "core.autocrlf=true"]);
    submodule_update_args.extend(["submodule", "update", "--init", "--", "vendor/child"]);
    run_git(slot_path, &submodule_update_args);
    let child = slot_path.join("vendor/child");
    #[cfg(windows)]
    assert!(
        fs::read(child.join("tracked.txt"))
            .expect("Windows submodule file should be readable")
            .windows(2)
            .any(|bytes| bytes == b"\r\n"),
        "Windows fixture must exercise the inherited CRLF checkout boundary"
    );
    let child_text = child
        .to_str()
        .expect("submodule worktree path should be UTF-8");
    let normal_core_worktree = run_command(
        "git",
        ["-C", child_text, "config", "--get", "core.worktree"],
        None,
    )
    .expect("initialized submodule should expose its generated core.worktree");
    let no_change_error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("clean submodule fixture has no worker result yet");
    assert!(
        no_change_error.contains("no tree changes"),
        "normal submodule core.worktree should pass inspection: {no_change_error}"
    );

    let outside_worktree = repo.root.join("outside-submodule-worktree");
    fs::create_dir_all(&outside_worktree).expect("outside worktree fixture should create");
    run_git(
        &child,
        &[
            "config",
            "core.worktree",
            outside_worktree
                .to_str()
                .expect("outside worktree path should be UTF-8"),
        ],
    );
    let outside_error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("submodule core.worktree outside the leased root must fail closed");
    assert!(
        outside_error.contains("core.worktree resolves outside"),
        "error: {outside_error}"
    );
    run_git(
        &child,
        &["config", "core.worktree", normal_core_worktree.as_str()],
    );
    #[cfg(unix)]
    {
        let marker = repo.root.join("nested-submodule-filter-executed");
        fs::write(child.join(".gitattributes"), "tracked.txt filter=hostile\n")
            .expect("nested filter attributes should write");
        fs::write(child.join("tracked.txt"), "candidate filtered change\n")
            .expect("nested filtered candidate should write");
        run_git(
            &child,
            &[
                "config",
                "filter.hostile.clean",
                &format!("sh -c 'printf executed > {}'", marker.display()),
            ],
        );

        let error = service
            .prepare_workspace_worker_commit(&lease)
            .expect_err("nested executable filter must block before submodule status");
        assert!(error.contains("filter.hostile.clean"), "error: {error}");
        assert!(
            !marker.exists(),
            "nested submodule status executed the hostile clean filter"
        );
        run_git(&child, &["config", "--unset", "filter.hostile.clean"]);
        fs::remove_file(child.join(".gitattributes"))
            .expect("nested filter attributes should remove");
    }
    fs::write(child.join("tracked.txt"), "hidden tracked change\n")
        .expect("submodule tracked change should write");
    fs::write(child.join("untracked.txt"), "hidden untracked change\n")
        .expect("submodule untracked change should write");
    fs::write(child.join("generated.tmp"), "hidden ignored change\n")
        .expect("submodule ignored change should write");
    fs::write(
        slot_path.join("worker-result.txt"),
        "visible parent result\n",
    )
    .expect("parent worker result should write");

    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("submodule ignore configuration must not hide dirty state");

    assert!(
        error.contains("tracked submodule contains"),
        "error: {error}"
    );
    run_git(&child, &["checkout", "--", "tracked.txt"]);
    fs::remove_file(child.join("untracked.txt"))
        .expect("submodule untracked fixture should remove");
    let ignored_only_error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("ignored-only submodule state must remain visible to the host");
    assert!(
        ignored_only_error.contains("tracked submodule contains"),
        "error: {ignored_only_error}"
    );
    service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("parallel reset should preserve a running dirty-submodule slot");
    assert_eq!(
        fs::read_to_string(child.join("generated.tmp"))
            .expect("submodule ignored change should remain inspectable"),
        "hidden ignored change\n"
    );
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::Running,
        "dirty submodule must keep the slot out of the reusable pool"
    );
}

#[test]
fn host_owned_worker_commit_blocks_wrong_lease_target_branch_and_stale_head() {
    let repo = TempGitRepo::new("host-worker-identity-guards");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-guards");
    fs::write(
        Path::new(&lease.worktree_path).join("guarded.txt"),
        "guarded\n",
    )
    .expect("guarded worker result should write");

    let mut wrong_lease = lease.clone();
    wrong_lease.task_id = "different-task".to_string();
    let error = service
        .prepare_workspace_worker_commit(&wrong_lease)
        .expect_err("replaced lease identity must block host commit");
    assert!(error.contains("lease identity changed"), "error: {error}");

    let mut wrong_base = lease.clone();
    wrong_base
        .delivery_target
        .as_mut()
        .expect("lease target should exist")
        .integration_base_commit_sha = "f".repeat(40);
    let error = service
        .prepare_workspace_worker_commit(&wrong_base)
        .expect_err("changed frozen target must block host commit");
    assert!(error.contains("lease identity changed"), "error: {error}");

    run_git(
        Path::new(&lease.worktree_path),
        &["checkout", "-qb", "unexpected-worker-branch"],
    );
    let error = service
        .prepare_workspace_worker_commit(&lease)
        .expect_err("wrong checked-out branch must block host commit");
    assert!(error.contains("expects"), "error: {error}");

    run_git(
        Path::new(&lease.worktree_path),
        &["checkout", "-q", lease.branch_name.as_str()],
    );
    let stale_head = resolve_workspace_head_sha(Path::new(&lease.worktree_path))
        .expect("pre-existing head should resolve");
    run_git(Path::new(&lease.worktree_path), &["add", "guarded.txt"]);
    run_git(
        Path::new(&lease.worktree_path),
        &["commit", "-qm", "advance worker head"],
    );
    let target = lease
        .delivery_target
        .as_ref()
        .expect("lease target should exist");
    let runtime = GitParallelModeRuntimeAdapter::new();
    let error = runtime
        .prepare_parallel_worker_commit(ParallelWorkerCommitRequest {
            workspace_directory: &lease.worktree_path,
            expected_branch_name: &lease.branch_name,
            expected_base_commit_sha: &target.integration_base_commit_sha,
            expected_head_commit_sha: &stale_head,
            commit_message: "akra: stale ref must fail",
            commit_timestamp: &lease.leased_at,
        })
        .expect_err("stale pre-commit HEAD must block ref movement");
    assert!(error.contains("HEAD changed"), "error: {error}");
}

#[cfg(unix)]
#[test]
fn host_owned_worker_commit_never_executes_hooks_fsmonitor_or_filters() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TempGitRepo::new("host-worker-hostile-git-config");
    let service = test_parallel_mode_service();
    let lease = acquire_running_worker_lease(&service, &repo, "task-host-hostile");
    let marker = repo.root.join("host-command-executed");
    let script = repo.root.join("hostile-command.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf executed >> '{}'\nexit 0\n",
            marker.display()
        ),
    )
    .expect("hostile command should write");
    let mut permissions = fs::metadata(&script)
        .expect("hostile command metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("hostile command should become executable");

    let hooks = repo.repo_root.join(".git/hooks");
    fs::copy(&script, hooks.join("pre-commit")).expect("pre-commit hook should install");
    fs::copy(&script, hooks.join("reference-transaction"))
        .expect("reference transaction hook should install");
    for hook in [
        hooks.join("pre-commit"),
        hooks.join("reference-transaction"),
    ] {
        let mut permissions = fs::metadata(&hook)
            .expect("hook metadata should read")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(hook, permissions).expect("hook should become executable");
    }
    run_git(
        &repo.repo_root,
        &[
            "config",
            "core.fsmonitor",
            script.to_str().expect("script path should be UTF-8"),
        ],
    );
    fs::write(
        Path::new(&lease.worktree_path).join("safe-result.md"),
        "safe result\n",
    )
    .expect("safe worker result should write");

    service
        .prepare_workspace_worker_commit(&lease)
        .expect("host commit should bypass hooks and fsmonitor");
    assert!(
        !marker.exists(),
        "host commit executed an untrusted hook or fsmonitor"
    );

    run_git(&repo.repo_root, &["config", "--unset", "core.fsmonitor"]);
    fs::remove_file(hooks.join("pre-commit")).expect("pre-commit hook should remove");
    fs::remove_file(hooks.join("reference-transaction"))
        .expect("reference transaction hook should remove");
    let filtered_lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request(
                "task-host-filter",
                "Host filter task",
                "agent-filter",
                "host-filter",
            ),
        )
        .expect("second worker slot should lease");
    service
        .mark_workspace_slot_running(&filtered_lease.worktree_path)
        .expect("filtered worker slot should become running");
    fs::write(
        Path::new(&filtered_lease.worktree_path).join(".gitattributes"),
        "*.txt filter=hostile\n",
    )
    .expect("filter attributes should write");
    fs::write(
        Path::new(&filtered_lease.worktree_path).join("filtered.txt"),
        "must not be filtered\n",
    )
    .expect("filtered candidate should write");
    run_git(
        &repo.repo_root,
        &[
            "config",
            "filter.hostile.clean",
            script.to_str().expect("script path should be UTF-8"),
        ],
    );
    run_git(
        &repo.repo_root,
        &[
            "config",
            "filter.hostile.process",
            script.to_str().expect("script path should be UTF-8"),
        ],
    );
    run_git(
        &repo.repo_root,
        &["config", "filter.hostile.required", "true"],
    );

    let error = service
        .prepare_workspace_worker_commit(&filtered_lease)
        .expect_err("active clean/process filter must block host staging");
    assert!(error.contains("filter.hostile.clean"), "error: {error}");
    assert!(
        !marker.exists(),
        "filter command executed before fail-closed staging"
    );
}
