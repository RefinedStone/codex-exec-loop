use super::*;

#[test]
fn unavailable_pool_board_does_not_report_exhausted() {
    let pool = build_pool_board(&SqlitePlanningAuthorityAdapter::new(), "/tmp/root", None);

    assert_eq!(pool.unavailable_slots, DEFAULT_POOL_SIZE);
    assert!(!pool.exhausted);
}

#[test]
fn reconcile_marks_missing_slots_when_pool_root_has_not_been_created() {
    let repo = TempGitRepo::new("missing-slots");
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

    assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.idle_slots, 0);
    assert!(!pool.exhausted);
    assert!(pool.reconcile_status.contains("missing slot"));
}

#[test]
fn detached_akra_slot_counts_as_idle_baseline() {
    let repo = TempGitRepo::new("idle-slot");
    repo.create_detached_slot(1);
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

    assert_eq!(slot.state, ParallelModePoolSlotState::Idle);
    assert_eq!(slot.branch_name, "akra (detached)");
    assert_eq!(pool.idle_slots, 1);
    assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE - 1);
}

#[test]
fn agent_branch_slot_is_marked_awaiting_cleanup() {
    let repo = TempGitRepo::new("cleanup-slot");
    repo.create_agent_slot(1, "task-one");
    let slot_path = repo.pool_root().join(slot_id(1));
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    repo.merge_agent_slot_into_akra(&slot_path);
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

    assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
    assert!(slot.branch_name.starts_with("akra-agent/slot-1/"));
    assert_eq!(slot.owner_label, "cleanup pending");
    assert_eq!(pool.awaiting_cleanup_slots, 1);
}

#[test]
fn non_merged_agent_branch_without_lease_surfaces_operator_recovery_notice() {
    let repo = TempGitRepo::new("non-merged-slot");
    let service = test_parallel_mode_service();
    let slot_path = repo.create_agent_slot(1, "task-one");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let slot = &snapshot.pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::Blocked);
    assert_eq!(slot.owner_label, "operator recovery");
    assert!(slot.branch_name.starts_with("akra-agent/slot-1/"));
    assert!(
        slot.worktree_label
            .contains(super::super::NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL)
    );
    assert!(
        snapshot
            .pool
            .reconcile_status
            .contains("next action: inspect the slot branch")
    );
    let notice = snapshot
        .top_notice
        .as_deref()
        .expect("operator recovery notice should be surfaced");
    assert!(notice.contains("pool: blocked"));
    assert!(notice.contains("slot-1"));
    assert!(notice.contains("not integrated into `akra`"));
    assert!(notice.contains("next action: inspect the slot branch"));
}

#[test]
fn dirty_akra_baseline_slot_is_blocked_for_operator_recovery() {
    let repo = TempGitRepo::new("dirty-slot");
    let slot_path = repo.create_detached_slot(1);
    fs::write(slot_path.join("README.md"), "dirty\n").expect("slot file should be updated");
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

    assert_eq!(slot.state, ParallelModePoolSlotState::Blocked);
    assert_eq!(slot.owner_label, "operator recovery");
    assert!(slot.worktree_label.contains("unstaged changes"));
}

#[test]
fn reconcile_resets_dirty_reusable_detached_baseline_slots() {
    let repo = TempGitRepo::new("dirty-reusable-slot");
    let slot_path = repo.create_detached_slot(1);
    fs::write(slot_path.join("README.md"), "dirty\n").expect("slot file should be updated");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.blocked_slots, 0);
    assert_eq!(
        fs::read_to_string(slot_path.join("README.md")).expect("README should be readable"),
        "seed\n"
    );
}

#[test]
fn reconcile_provisions_missing_slots_into_idle_baselines() {
    let repo = TempGitRepo::new("provision-slots");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.missing_slots, 0);
    assert!(pool.reconcile_status.contains("provisioned 3"));
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        assert!(repo.pool_root().join(slot_id(slot_number)).exists());
    }
}

#[test]
fn pool_root_lives_in_repo_sibling_akra_worktrees_root() {
    let repo = TempGitRepo::new("pool-root");
    let pool_root = repo.pool_root();
    let normalized = pool_root.to_string_lossy().replace('\\', "/");

    assert!(
        normalized.contains("/repo-akra-worktrees/"),
        "pool root should live under a repo sibling akra worktrees root: {normalized}"
    );
    assert!(
        normalized.ends_with("/akra-pool"),
        "pool root should end at the akra pool directory: {normalized}"
    );
}

#[test]
fn reconcile_creates_local_akra_branch_before_provisioning_slots() {
    let repo = TempGitRepo::new("create-akra");
    repo.delete_local_akra_branch();
    assert!(!repo.branch_exists("akra"));

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert!(repo.branch_exists("akra"));
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert!(pool.reconcile_status.contains("created `akra`"));
}

#[test]
fn reconcile_resets_empty_akra_baseline_to_current_head() {
    let repo = TempGitRepo::new("reset-akra");
    let old_akra_head = repo.head_sha();
    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");
    let current_head = repo.head_sha();
    assert_ne!(old_akra_head, current_head);

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                "akra",
            ],
            None,
        )
        .expect("akra should resolve"),
        current_head
    );
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
}

#[test]
fn reconcile_resets_clean_detached_slots_after_empty_akra_baseline_moves() {
    let repo = TempGitRepo::new("reset-detached-slots");
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);

    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");

    let refreshed_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert_eq!(refreshed_pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(refreshed_pool.blocked_slots, 0);
    assert!(refreshed_pool.slots.iter().all(|slot| {
        !slot
            .worktree_label
            .contains("detached away from `akra` baseline")
    }));
}

#[test]
fn build_pool_board_uses_remote_akra_when_local_branch_is_missing() {
    let repo = TempGitRepo::new("remote-akra");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let head_sha = repo.head_sha();
    repo.delete_local_akra_branch();
    repo.set_remote_tracking_branch("origin/akra", &head_sha);

    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert_eq!(pool.blocked_slots, 0);
    assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.reconcile_status.contains("missing"),
        "unexpected reconcile status: {}",
        pool.reconcile_status
    );
}

#[test]
fn detect_canonical_repo_root_uses_workspace_relative_common_dir() {
    let repo = TempGitRepo::new("canonical-root");
    let nested_workspace = repo.repo_root.join("nested").join("deeper");
    fs::create_dir_all(&nested_workspace).expect("nested workspace should exist");

    let canonical_repo_root = detect_canonical_repo_root(
        &SqlitePlanningAuthorityAdapter::new(),
        nested_workspace.to_str().expect("valid nested path"),
    )
    .expect("canonical repo root should resolve");

    assert_eq!(
        canonical_repo_root,
        fs::canonicalize(&repo.repo_root).expect("repo root should canonicalize")
    );
}

#[test]
fn inspect_readiness_reports_authority_store_from_canonical_repo_root() {
    let repo = TempGitRepo::new("authority-readiness");
    let linked_worktree = repo.create_linked_worktree("feature/authority-readiness");
    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        linked_worktree
            .to_str()
            .expect("valid linked worktree path"),
        RESULT_OUTPUT_FILE_PATH,
        Some("# Result Output Prompt\n"),
    )
    .expect("authority store should seed active result output");
    let worktree_directions_path = linked_worktree.join(RESULT_OUTPUT_FILE_PATH);
    fs::create_dir_all(
        worktree_directions_path
            .parent()
            .expect("worktree result output path should have a parent directory"),
    )
    .expect("worktree planning directory should exist");
    fs::write(&worktree_directions_path, "# divergent result\n")
        .expect("linked-worktree result output should diverge");
    let service = test_parallel_mode_service();

    let snapshot = service.inspect_readiness(
        linked_worktree
            .to_str()
            .expect("valid linked worktree path"),
        &PlanningRuntimeSnapshot::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    let capability = snapshot
        .capability(ParallelModeCapabilityKey::AuthorityStore)
        .expect("authority store capability should exist");

    assert_eq!(capability.state, ParallelModeCapabilityState::Ready);
    assert!(capability.detail.contains("shadow store"));
    assert!(capability.detail.contains(&repo.workspace_dir()));
    assert!(!capability.detail.contains("version = 0"));
}

#[test]
fn reconcile_cleans_merged_agent_slot_back_to_idle() {
    let repo = TempGitRepo::new("cleanup-execution");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    let branch_name = lease.branch_name.clone();
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    fs::write(slot_path.join("scratch.tmp"), "transient\n")
        .expect("untracked file should be written");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );
    let slot = &pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::Idle);
    assert!(slot.branch_name.starts_with("akra"));
    assert!(!slot_path.join("scratch.tmp").exists());
    assert!(!repo.branch_exists(&branch_name));
    assert!(!repo.slot_lease_path(1).exists());
    assert!(pool.reconcile_status.contains("cleaned 1"));
}

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
    run_git(&repo.repo_root, &["branch", first.as_str(), "akra"]);

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
    assert!(not_merged_error.contains("is not integrated into `akra` yet"));
}

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
