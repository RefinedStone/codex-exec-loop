use super::super::*;

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
    assert_eq!(pool.leased_slots, 1);
    assert_eq!(pool.running_slots, 0);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Leased);
    assert_eq!(pool.slots[0].owner_label, "agent-1 / task-1");
}

// lease write는 authority DB를 먼저 갱신하고 filesystem mirror를 나중에 쓴다. mirror write가
// 실패하면 이미 저장된 authority lease를 되돌려야, 실패한 dispatch가 slot을 영구 점유하지 않는다.
#[test]
fn acquire_slot_lease_rolls_back_authority_when_mirror_write_fails() {
    let repo = TempGitRepo::new("lease-mirror-write-fails");
    let service = test_parallel_mode_service();
    reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
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

    assert!(error.contains("failed to create lease directory"));
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.slots
            .iter()
            .all(|slot| !slot.branch_name.starts_with("akra-agent/"))
    );
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

// sqlite authority store에서 lease가 사라진 뒤 legacy mirror 파일만 남은 상태는
// 오래된 호환 흔적이다. pool은 mirror를 active lease로 믿지 않고 agent branch의
// 실제 통합 여부를 기준으로 cleanup 대기 상태를 계산해야 한다.
#[test]
fn pool_ignores_stale_legacy_lease_mirror_after_store_removal() {
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
    assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
    assert_eq!(slot.owner_label, "cleanup pending");
    assert!(slot.branch_name.starts_with("akra-agent/slot-1/"));
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
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", &long_slug),
        )
        .expect("slot lease should be acquired");
    let slug = lease
        .branch_name
        .strip_prefix("akra-agent/slot-1/")
        .expect("slot branch prefix should be present");

    assert!(slug.len() <= MAX_AGENT_BRANCH_SLUG_LEN);
    assert!(slug.ends_with(short_branch_slug_hash(&sanitized_slug).as_str()));
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
        &repo.workspace_dir(),
        "slot-1",
        &long_slug,
        "task-1",
        "Task One",
    );
    run_git(&repo.repo_root, &["branch", first.as_str(), "prerelease"]);
    let second = allocate_agent_branch_name(
        &repo.workspace_dir(),
        "slot-1",
        &long_slug,
        "task-1",
        "Task One",
    );
    let slug = second
        .strip_prefix("akra-agent/slot-1/")
        .expect("slot branch prefix should be present");
    let base_slug = slug
        .strip_suffix("-2")
        .expect("collision branch should carry a numbered suffix");

    assert_ne!(first, second);
    assert!(slug.len() <= MAX_AGENT_BRANCH_SLUG_LEN);
    assert!(base_slug.ends_with(short_branch_slug_hash(&sanitized_slug).as_str()));
}

// remote-tracking branch만 있어도 이후 push에서 충돌할 수 있다. allocator는 로컬
// branch뿐 아니라 `origin/...` tracking ref도 선점된 이름으로 보고 다음 번호를
// 선택해야 한다.
#[test]
fn allocate_agent_branch_name_numbers_remote_tracking_collisions() {
    let repo = TempGitRepo::new("lease-slot-remote-branch-collision");
    repo.set_remote_tracking_branch("origin/akra-agent/slot-1/task-one", "prerelease");
    let branch_name = allocate_agent_branch_name(
        &repo.workspace_dir(),
        "slot-1",
        "task-one",
        "task-1",
        "Task One",
    );

    assert_eq!(branch_name, "akra-agent/slot-1/task-one-2");
}

// 실제 remote에만 존재하는 branch도 fetch/tracking 상태에 따라 뒤늦게 충돌할 수
// 있다. live remote ref까지 확인해 새 agent branch가 push 단계에서 reject되지
// 않도록 번호를 올린다.
#[test]
fn allocate_agent_branch_name_numbers_live_remote_collisions() {
    let repo = TempGitRepo::new("lease-slot-live-remote-branch-collision");
    let remote_path = repo.root.join("origin.git");
    run_git(&repo.root, &["init", "--bare", "origin.git"]);
    run_git(
        &repo.repo_root,
        &["remote", "add", "origin", remote_path.to_str().unwrap()],
    );
    run_git(
        &repo.repo_root,
        &[
            "push",
            "origin",
            "prerelease:refs/heads/akra-agent/slot-1/task-one",
        ],
    );
    let branch_name = allocate_agent_branch_name(
        &repo.workspace_dir(),
        "slot-1",
        "task-one",
        "task-1",
        "Task One",
    );

    assert_eq!(branch_name, "akra-agent/slot-1/task-one-2");
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

// cleanup-pending slot은 untracked scratch 파일까지 제거하고 agent branch와 lease
// mirror를 치운 뒤 idle baseline으로 돌아가야 한다. 이 테스트는 cleanup 결과와
// pool projection을 함께 확인한다.
#[test]
fn cleanup_workspace_slot_if_pending_resets_slot_to_idle() {
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
        .expect("cleanup-pending workspace should be cleaned")
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
