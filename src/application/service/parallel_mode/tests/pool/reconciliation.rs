use super::*;
use crate::application::service::parallel_mode::NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL;

// pool directory가 아직 만들어지지 않은 상태는 장애가 아니라 초기 준비 상태다.
// board builder는 slot을 임의로 만들지 않고 missing으로만 보고해야 하며, 이때
// exhausted를 켜지 않아 dispatcher가 "용량 소진"과 "아직 provision 안 됨"을 구분한다.
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

// detached `prerelease` worktree는 재사용 가능한 idle baseline이다. branch 이름이
// 실제 local branch가 아니라 detached baseline임을 드러내면서도 slot 하나의
// capacity로 계산되어야 한다.
#[test]
fn detached_prerelease_slot_counts_as_idle_baseline() {
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
    assert_eq!(slot.branch_name, "prerelease (detached)");
    assert_eq!(pool.idle_slots, 1);
    assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE - 1);
}

// linked worktree git dir에 REBASE_HEAD만 stale하게 남을 수 있다. 실제 rebase 중이면
// rebase-merge/rebase-apply metadata가 함께 있으므로, clean detached baseline slot은
// 단독 REBASE_HEAD 때문에 blocked로 오인되면 안 된다.
#[test]
fn detached_prerelease_slot_with_stale_rebase_head_counts_as_idle_baseline() {
    let repo = TempGitRepo::new("stale-rebase-head-slot");
    let slot_path = repo.create_detached_slot(1);
    let git_dir = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be utf-8"),
            "rev-parse",
            "--git-dir",
        ],
        None,
    )
    .expect("slot git dir should resolve");
    fs::write(
        Path::new(git_dir.trim()).join("REBASE_HEAD"),
        repo.head_sha(),
    )
    .expect("stale REBASE_HEAD should be written");
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
    assert_eq!(slot.branch_name, "prerelease (detached)");
    assert_eq!(pool.idle_slots, 1);
    assert_eq!(pool.blocked_slots, 0);
}

// agent branch가 이미 `prerelease`에 merge된 뒤 lease mirror가 없으면 새 작업을
// 배정하기 전에 cleanup이 필요한 상태다. board는 이를 blocked가 아니라
// awaiting cleanup으로 분류해 자동 정리 대상임을 표현한다.
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

// lease 없이 남은 agent branch가 아직 merge되지 않았다면 자동으로 지우면 안 된다.
// supervisor는 slot label, reconcile status, top notice에 모두 operator recovery
// 경로를 노출해 사용자가 branch 내용을 먼저 확인하도록 유도한다.
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
            .contains(NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL)
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
    assert!(notice.contains("not integrated into `prerelease`"));
    assert!(notice.contains("next action: inspect the slot branch"));
}

// board-only 경로는 사용자의 dirty baseline worktree를 고치지 않는다. detached
// prerelease slot에 unstaged change가 있으면 즉시 blocked로 표시해 reconcile 실행
// 전에도 위험 상태가 TUI에 보이도록 한다.
#[test]
fn dirty_prerelease_baseline_slot_is_blocked_for_operator_recovery() {
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

// reconcile 경로는 idle detached baseline이 dirty해도 버릴 수 있는 cache로 본다.
// 실제 작업 lease가 없는 재사용 slot은 reset되어 다시 seed baseline으로 돌아가야
// 다음 agent에게 오염된 worktree가 배정되지 않는다.
#[test]
fn reconcile_resets_dirty_reusable_detached_baseline_slots() {
    let repo = TempGitRepo::new("dirty-reusable-slot");
    let slot_path = repo.create_detached_slot(1);
    fs::write(slot_path.join("README.md"), "dirty\n").expect("slot file should be updated");
    fs::write(slot_path.join("scratch.tmp"), "transient\n")
        .expect("untracked slot residue should be written");
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.blocked_slots, 0);
    assert_eq!(
        fs::read_to_string(slot_path.join("README.md")).expect("README should be readable"),
        "seed\n"
    );
    assert!(!slot_path.join("scratch.tmp").exists());
}

// 한 slot이 running인 동안에도 다른 idle baseline들은 표준 remote branch로 정리될 수
// 있어야 한다. 이 테스트는 실행 중인 lease를 보존하면서 reusable slot만 reset하고,
// canonical 표준 ref가 현재 작업 branch HEAD로 이동하지 않는지도 함께 확인한다.
#[test]
fn reconcile_resets_reusable_detached_slots_while_another_slot_is_running() {
    let repo = TempGitRepo::new("dirty-reusable-slot-with-running-lease");
    let service = test_parallel_mode_service();
    let origin_prerelease_head = run_command(
        "git",
        [
            "-C",
            repo.repo_root.to_str().expect("repo root should be utf-8"),
            "rev-parse",
            &remote_standard_tracking_ref(),
        ],
        None,
    )
    .expect("origin prerelease should resolve");
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.commit_on_current_branch("baseline.txt", "new baseline\n", "advance baseline");
    let current_head = repo.head_sha();
    assert_ne!(origin_prerelease_head, current_head);
    let reusable_slot_path = repo.pool_root().join(slot_id(2));
    fs::write(reusable_slot_path.join("README.md"), "dirty\n")
        .expect("idle slot should become dirty");
    let refreshed_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(refreshed_pool.running_slots, 1);
    assert_eq!(refreshed_pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert_eq!(refreshed_pool.blocked_slots, 0);
    assert_eq!(
        fs::read_to_string(reusable_slot_path.join("README.md"))
            .expect("README should be readable"),
        "seed\n"
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .expect("prerelease should resolve"),
        origin_prerelease_head
    );
}

// parallel mode를 off -> on으로 켜더라도 Running lease는 live execution 증거다.
// reset은 projection을 지우거나 worktree를 되돌리지 않고 blocked report만 남긴다.
#[test]
fn parallel_entry_from_off_preserves_live_running_slot_and_resets_idle_slots() {
    let repo = TempGitRepo::new("parallel-enable-reset-active");
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
    repo.commit_file_in_slot(&slot_path, "stale.txt", "stale\n", "stale agent work");
    fs::write(slot_path.join("scratch.tmp"), "discard me\n").expect("scratch file should write");

    let report = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("parallel enable reset should report live blockers");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE - 1);
    assert_eq!(
        report.slot_reports[0].action,
        ParallelModePoolResetSlotAction::PreserveLive
    );
    assert_eq!(
        report.slot_reports[0].outcome,
        ParallelModePoolResetSlotOutcome::Blocked
    );
    assert_eq!(snapshot.roster.active_count(), 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(slot_path.join("stale.txt").exists());
    assert!(slot_path.join("scratch.tmp").exists());
    assert!(current_branch(&slot_path).starts_with("akra-agent/slot-1/"));
}

// TUI 프로세스에서 처음 `:parallel`을 켜는 초기 설정은 이전 실행의 stale
// projection을 신뢰하지 않는다. failed/cleaned worker가 Running lease를 남겨도
// disposable pool 전체를 현재 prerelease baseline으로 강제 정렬해야 한다.
#[test]
fn parallel_initial_setup_forces_live_running_slots_back_to_baseline() {
    let repo = TempGitRepo::new("parallel-initial-force-reset-active");
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
    repo.commit_file_in_slot(&slot_path, "stale.txt", "stale\n", "stale agent work");
    fs::write(slot_path.join("scratch.tmp"), "discard me\n").expect("scratch file should write");

    let report = service
        .reset_pool_on_parallel_initial_setup_report(&repo.workspace_dir())
        .expect("initial setup reset should force disposable slots");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(report.policy, ParallelModePoolResetPolicy::ForceDisposable);
    assert_eq!(report.live_blocker_count(), 0);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE);
    assert_eq!(snapshot.roster.active_count(), 0);
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!slot_path.join("stale.txt").exists());
    assert!(!slot_path.join("scratch.tmp").exists());
    assert_eq!(current_branch(&slot_path), "HEAD");
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                slot_path.to_str().expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("slot head should resolve"),
        repo.head_sha()
    );
}

// 초기 설정 전에 사용자가 pool worktree를 수동 삭제했더라도 durable runtime projection은
// 같은 repo authority DB에 남을 수 있다. 첫 `:parallel`은 missing slot을 새로 만들기 전에
// 이 stale lease/session/queue/dispatch command/block을 버려야 한다.
#[test]
fn parallel_initial_setup_clears_stale_runtime_when_pool_worktrees_are_missing() {
    let repo = TempGitRepo::new("parallel-initial-clears-missing-runtime");
    let service = test_parallel_mode_service();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    record_assigned_session_detail(
        &adapter,
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("stale session detail should be recorded");
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &PlanningAuthorityDistributorQueueRecord {
            queue_item_id: "stale-queue-1".to_string(),
            queue_order_key: 1,
            session_key: lease.session_key(),
            slot_id: lease.slot_id.clone(),
            agent_id: lease.agent_id.clone(),
            task_id: lease.task_id.clone(),
            task_title: lease.task_title.clone(),
            source_branch: "prerelease".to_string(),
            source_commit_sha: repo.head_sha(),
            branch_name: lease.branch_name.clone(),
            worktree_path: lease.worktree_path.clone(),
            commit_sha: repo.head_sha(),
            original_commit_sha: None,
            planning_refresh_state: "failed".to_string(),
            integration_state: "blocked".to_string(),
            conflict_files: Vec::new(),
            recovery_note: Some("stale queue from previous runtime".to_string()),
            validation_summary: "stale validation".to_string(),
            authority_refresh_outcome: "stale official refresh".to_string(),
            github_capabilities: None,
            pull_request_number: None,
            pull_request_url: None,
            queue_state: ParallelModeQueueItemState::Blocked,
            integration_note: "stale blocked queue".to_string(),
            enqueued_at: "2026-05-08T08:55:15.467459643+00:00".to_string(),
            updated_at: "2026-05-08T09:10:40.820469463+00:00".to_string(),
        },
    )
    .expect("stale distributor queue should persist");
    SqlitePlanningAuthorityAdapter::enqueue_runtime_dispatch_command(
        &repo.workspace_dir(),
        &ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            Some("stale-head".to_string()),
            Some(1),
            "2026-05-08T09:10:40.820469463+00:00",
        ),
    )
    .expect("stale dispatch command should persist");
    SqlitePlanningAuthorityAdapter::upsert_runtime_task_dispatch_block(
        &repo.workspace_dir(),
        &ParallelModeTaskDispatchBlockSnapshot::new(
            lease.task_id.as_str(),
            "2026-05-09T08:31:40Z",
            "2026-05-09T20:32:57.657951438+00:00",
            ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
        ),
    )
    .expect("stale dispatch block should persist");
    run_git(
        &repo.repo_root,
        &[
            "worktree",
            "remove",
            "--force",
            lease.worktree_path.as_str(),
        ],
    );

    let report = service
        .reset_pool_on_parallel_initial_setup_report(&repo.workspace_dir())
        .expect("initial setup reset should clear stale runtime for missing slots");
    let snapshot = service.reconcile_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projections should load");

    assert_eq!(report.policy, ParallelModePoolResetPolicy::ForceDisposable);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE);
    assert_eq!(snapshot.roster.active_count(), 0);
    assert_eq!(snapshot.pool.idle_slots, DEFAULT_POOL_SIZE);
    assert!(runtime_projection.slot_leases.is_empty());
    assert!(runtime_projection.session_details.is_empty());
    assert!(runtime_projection.distributor_queue_records.is_empty());
    assert!(runtime_projection.dispatch_commands.is_empty());
    assert!(runtime_projection.task_dispatch_blocks.is_empty());
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.session_detail_path(&lease.session_key()).exists());
}

// Dirty tracked files in no-lease reusable slots must not stop off -> on pool reset. Git checkout
// without --force can fail before reset --hard runs when a slot has committed and uncommitted edits
// to the same tracked file, which leaves the slot detached at stale work.
#[test]
fn parallel_entry_from_off_forces_dirty_no_lease_slot_back_to_baseline() {
    let repo = TempGitRepo::new("parallel-enable-reset-dirty-tracked");
    let service = test_parallel_mode_service();
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);
    let slot_path = repo.pool_root().join(slot_id(1));
    fs::write(slot_path.join("README.md"), "dirty local version\n")
        .expect("dirty tracked file should write");

    let reset_count = service
        .reset_pool_on_parallel_enable(&repo.workspace_dir())
        .expect("parallel enable reset should force dirty tracked slots back to baseline");

    assert_eq!(reset_count, DEFAULT_POOL_SIZE);
    assert!(!repo.slot_lease_path(1).exists());
    assert_eq!(
        fs::read_to_string(slot_path.join("README.md")).expect("readme should be readable"),
        "seed\n"
    );
    assert_eq!(current_branch(&slot_path), "HEAD");
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                slot_path.to_str().expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("slot head should resolve"),
        repo.head_sha()
    );
}

// Running lease는 slot worktree가 더 이상 해당 agent branch에 있지 않아도 자동 reset으로
// 없애지 않는다. branch drift는 destructive reset보다 operator recovery로 남겨야 한다.
#[test]
fn parallel_entry_from_off_preserves_running_branch_drift_and_resets_idle_slots() {
    let repo = TempGitRepo::new("parallel-enable-reset-stale-running");
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
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    fs::write(slot_path.join("scratch.tmp"), "discard me\n").expect("scratch file should write");

    let report = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("running branch drift should be reported as live blocker");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE - 1);
    assert_eq!(snapshot.roster.active_count(), 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(slot_path.join("scratch.tmp").exists());
    assert_eq!(current_branch(&slot_path), "HEAD");
}

// 실제 장애 재현 케이스: authority store에는 Running lease가 남아 있지만 slot worktree는
// 이미 clean detached prerelease로 돌아와 있고 agent branch도 없다. 이 split-brain을
// live slot으로 계속 보존하면 dispatcher가 capacity를 잃어 병렬 실행이 멈춘다.
#[test]
fn reconcile_releases_clean_baseline_split_brain_running_lease() {
    let repo = TempGitRepo::new("reconcile-clean-baseline-split-brain");
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
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    run_git(
        &repo.repo_root,
        &["branch", "-D", lease.branch_name.as_str()],
    );

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projections should load");
    let detail = read_agent_session_detail_record(
        &test_parallel_runtime(),
        &repo.pool_root(),
        &lease.session_key(),
    )
    .expect("stale session detail should be recorded");

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.blocked_slots, 0);
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!runtime_projection.slot_leases.contains_key("slot-1"));
    assert!(
        runtime_projection
            .task_dispatch_blocks
            .iter()
            .any(|block| block.task_id == lease.task_id)
    );
    assert_eq!(detail.state_label, "failed");
    assert_eq!(detail.completion_state_label, "aborted");
    assert!(
        detail
            .latest_summary
            .contains("stale active lease reconciled")
    );
}

// 같은 split-brain이라도 baseline worktree에 변경이 남아 있으면 자동 회수하면 안 된다.
// 이 경우는 사용자나 아직 늦게 쓰는 worker가 남긴 산출물일 수 있으므로 blocked로 보존한다.
#[test]
fn reconcile_preserves_dirty_baseline_split_brain_running_lease() {
    let repo = TempGitRepo::new("reconcile-dirty-baseline-split-brain");
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
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    run_git(
        &repo.repo_root,
        &["branch", "-D", lease.branch_name.as_str()],
    );
    fs::write(slot_path.join("README.md"), "dirty\n").expect("dirty slot file should write");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projections should load");

    assert_eq!(pool.blocked_slots, 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(runtime_projection.slot_leases.contains_key("slot-1"));
    assert!(
        runtime_projection
            .task_dispatch_blocks
            .iter()
            .all(|block| block.task_id != lease.task_id)
    );
    assert_eq!(
        fs::read_to_string(slot_path.join("README.md")).expect("README should be readable"),
        "dirty\n"
    );
}

#[test]
fn parallel_entry_from_off_resets_stale_startup_slots_even_when_another_slot_is_running() {
    let repo = TempGitRepo::new("parallel-enable-reset-stale-with-running");
    let service = test_parallel_mode_service();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let running_lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request(
                "task-running",
                "Running Task",
                "agent-running",
                "running-task",
            ),
        )
        .expect("running slot lease should be acquired");
    let running_slot_path = PathBuf::from(running_lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&running_lease.worktree_path)
        .expect("running slot should transition to running");
    fs::write(running_slot_path.join("keep-running.tmp"), "keep me\n")
        .expect("running scratch file should write");

    let mut stale_lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-stale", "Stale Task", "agent-stale", "stale-task"),
        )
        .expect("stale slot lease should be acquired");
    let stale_slot_path = PathBuf::from(stale_lease.worktree_path.clone());
    stale_lease.leased_at = "2020-01-01T00:00:00Z".to_string();
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&repo.workspace_dir(), &stale_lease)
        .expect("stale lease should be persisted");
    record_assigned_session_detail(
        &adapter,
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &stale_lease,
    )
    .expect("stale assigned detail should be recorded");
    fs::write(stale_slot_path.join("scratch.tmp"), "discard me\n")
        .expect("stale scratch file should write");

    let report = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("parallel enable reset should preserve live slots and reset stale slots");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(report.live_blocker_count(), 1);
    assert!(
        report
            .succeeded_reset_slot_ids()
            .contains(&stale_lease.slot_id)
    );
    assert_eq!(snapshot.roster.active_count(), 1);
    assert!(!repo.slot_lease_path(2).exists());
    assert!(
        !repo
            .session_detail_path(&stale_lease.session_key())
            .exists()
    );
    assert!(!stale_slot_path.join("scratch.tmp").exists());
    assert!(repo.slot_lease_path(1).exists());
    assert!(running_slot_path.join("keep-running.tmp").exists());
}

// Leased startup residue is also disposable on off -> on entry. The reset keeps planning task
// authority but clears runtime/session mirrors before the next dispatch pass.
#[test]
fn parallel_entry_from_off_resets_stale_startup_leases_and_slot_worktrees() {
    let repo = TempGitRepo::new("parallel-enable-reset-stale-startup");
    let service = test_parallel_mode_service();
    let mut lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    lease.leased_at = "2020-01-01T00:00:00Z".to_string();
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&repo.workspace_dir(), &lease)
        .expect("stale lease should be persisted");
    record_assigned_session_detail(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("assigned detail should be recorded");
    fs::write(slot_path.join("scratch.tmp"), "discard me\n").expect("scratch file should write");

    let reset_count = service
        .reset_pool_on_parallel_enable(&repo.workspace_dir())
        .expect("parallel re-entry from off should reset stale startup lease");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(reset_count, DEFAULT_POOL_SIZE);
    assert_eq!(snapshot.roster.active_count(), 0);
    assert!(snapshot.detail.session.is_none());
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.session_detail_path(&lease.session_key()).exists());
    assert!(!slot_path.join("scratch.tmp").exists());
    assert_eq!(current_branch(&slot_path), "HEAD");
}

// `:parallel` 진입 reset의 범위는 disposable pool runtime으로 한정된다. 기존
// planning task authority와 queue projection은 사용자가 만든 작업 원장이므로 보존해야 한다.
#[test]
fn parallel_entry_reset_preserves_existing_planning_tasks() {
    let repo = TempGitRepo::new("parallel-reset-preserves-tasks");
    let service = test_parallel_mode_service();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let queue_task = queue_task(1, "task-1");
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![TaskDefinition {
            id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_relation_note: "covers the reset scope contract".to_string(),
            title: "Keep existing task".to_string(),
            description: "This task must survive parallel pool reset.".to_string(),
            status: TaskStatus::Ready,
            base_priority: 99,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: Default::default(),
            updated_at: queue_task.updated_at.clone(),
        }],
    };
    let queue_projection = PriorityQueueProjection {
        next_task: Some(queue_task.clone()),
        active_tasks: vec![queue_task],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    adapter
        .commit_task_authority_snapshot(
            &repo.workspace_dir(),
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("planning task authority should commit");

    service
        .reset_pool_on_parallel_enable(&repo.workspace_dir())
        .expect("parallel pool reset should succeed");

    let snapshot = adapter
        .load_task_authority_snapshot(&repo.workspace_dir())
        .expect("planning task authority should load after reset")
        .expect("planning task authority should remain present");
    assert_eq!(snapshot.task_authority, task_authority);
    assert_eq!(snapshot.queue_projection, queue_projection);
}

// reconcile은 비어 있는 pool root를 실제 capacity로 바꾸는 provisioning 단계다.
// 모든 slot worktree가 생성되고 missing count가 사라져야 dispatcher가 곧바로
// idle slot을 사용할 수 있다.
#[test]
fn reconcile_provisions_missing_slots_into_idle_baselines() {
    let repo = TempGitRepo::new("provision-slots");
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.missing_slots, 0);
    assert!(pool.reconcile_status.contains("provisioned 3"));
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        assert!(repo.pool_root().join(slot_id(slot_number)).exists());
    }
}

// git worktree inventory에서 사라진 slot path라도 lease가 없으면 pool이 소유한
// disposable residue다. reconcile은 남은 파일을 제거하고 같은 slot path를 clean
// detached baseline worktree로 다시 만들어야 한다.
#[test]
fn reconcile_recreates_missing_slot_over_filesystem_residue() {
    let repo = TempGitRepo::new("provision-over-residue");
    let residue_path = repo.pool_root().join(slot_id(1));
    fs::create_dir_all(&residue_path).expect("residue directory should be created");
    fs::write(residue_path.join("scratch.tmp"), "transient\n")
        .expect("residue file should be written");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.blocked_slots, 0);
    assert!(!residue_path.join("scratch.tmp").exists());
    assert_eq!(current_branch(&residue_path), "HEAD");
    assert_eq!(pool.slots[0].branch_name, "prerelease (detached)");
}

// worker launch가 중간에 사라져 Leased 상태만 오래 남으면 roster가 계속 active로
// 보인다. reconcile은 오래된 launch-pending lease와 clean worktree를 startup
// failure로 회수해 slot을 다시 idle pool로 돌려야 한다.
#[test]
fn reconcile_releases_stale_leased_startup_slot() {
    let repo = TempGitRepo::new("stale-leased-startup");
    let service = test_parallel_mode_service();
    let mut lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    lease.leased_at = "2020-01-01T00:00:00Z".to_string();
    SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(&repo.workspace_dir(), &lease)
        .expect("stale lease should be persisted");
    record_assigned_session_detail(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("assigned detail should be recorded");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.running_slots, 0);
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
    assert_eq!(current_branch(&PathBuf::from(&lease.worktree_path)), "HEAD");
    assert_eq!(
        read_agent_session_detail_record(
            &test_parallel_runtime(),
            &repo.pool_root(),
            &lease.session_key()
        )
        .expect("failed startup detail should be recorded")
        .state_label,
        "failed"
    );
}

// pool worktree는 repository 내부가 아니라 sibling `repo-akra-worktrees` 아래에 둔다.
// 이렇게 해야 원본 checkout의 status와 nested worktree 탐색이 agent slot 파일들로
// 오염되지 않는다.
#[test]
fn pool_root_lives_in_repo_sibling_akra_worktrees_root() {
    let repo = TempGitRepo::new("pool-root");
    let pool_root = repo.pool_root();
    let normalized = pool_root.to_string_lossy().replace('\\', "/");

    assert!(
        normalized.contains("/repo-akra-worktrees/"),
        "pool root should live under a repo sibling prerelease worktrees root: {normalized}"
    );
    assert!(
        normalized.ends_with("/akra-pool"),
        "pool root should end at the akra pool directory: {normalized}"
    );
}

// 사용자가 로컬 표준 branch를 지웠더라도 reconcile은 baseline ref를 먼저
// 복구한 뒤 slot을 provision해야 한다. slot 생성과 branch 복구가 같은 흐름에서
// 일어나야 이후 slot들이 모두 동일한 기준 commit을 바라본다.
#[test]
fn reconcile_creates_local_prerelease_branch_before_provisioning_slots() {
    let repo = TempGitRepo::new("create-akra");
    repo.delete_local_prerelease_branch();
    assert!(!repo.branch_exists(POOL_BASELINE_BRANCH));
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert!(repo.branch_exists(POOL_BASELINE_BRANCH));
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.reconcile_status
            .contains(&format!("created `{POOL_BASELINE_BRANCH}`"))
    );
}

// local baseline이 현재 작업 branch HEAD로 drift해도 reconcile은 현재 workspace가 아니라
// 표준 remote branch를 authoritative baseline으로 삼아야 한다. 이 테스트는 pool slot이
// 사용자의 feature HEAD에서 시작하는 회귀를 막는다.
#[test]
fn reconcile_resets_drifted_local_prerelease_baseline_to_origin_prerelease() {
    let repo = TempGitRepo::new("reset-akra");
    let origin_prerelease_head = run_command(
        "git",
        [
            "-C",
            repo.repo_root.to_str().expect("repo root should be utf-8"),
            "rev-parse",
            &remote_standard_tracking_ref(),
        ],
        None,
    )
    .expect("origin prerelease should resolve");
    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");
    let current_head = repo.head_sha();
    assert_ne!(origin_prerelease_head, current_head);
    run_git(
        &repo.repo_root,
        &["branch", "-f", POOL_BASELINE_BRANCH, "HEAD"],
    );
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .expect("prerelease should resolve"),
        origin_prerelease_head
    );
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
}

// fresh repository처럼 local/remote 표준 branch가 모두 없으면 reconcile이 현재 작업 branch HEAD를
// 표준 branch로 만들고 origin에 push해야 한다. 이 흐름이 `:parallel`의 첫 pool 생성 완충 장치다.
#[test]
fn reconcile_seeds_missing_standard_branch_from_current_head_and_pushes_origin() {
    let repo = TempGitRepo::new("seed-standard-branch");
    let origin_root = repo.create_bare_origin_remote();
    repo.delete_local_prerelease_branch();
    repo.delete_remote_standard_tracking_branch();
    let expected_head = repo.head_sha();
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let remote_head = run_command(
        "git",
        [
            "--git-dir",
            origin_root.to_str().expect("origin root should be utf-8"),
            "rev-parse",
            &local_standard_ref(),
        ],
        None,
    )
    .expect("pushed standard branch should resolve in origin");

    assert_eq!(remote_head, expected_head);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .expect("local standard branch should resolve"),
        expected_head
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                &remote_standard_tracking_ref(),
            ],
            None,
        )
        .expect("remote tracking standard branch should resolve"),
        expected_head
    );
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
}

// baseline ref가 이동하면 기존 clean detached slot들도 예전 commit에 떨어져 있을
// 수 있다. reconcile은 dirty하지 않은 slot을 새 baseline으로 reset해, board에
// "detached away" 경고가 남지 않도록 정렬한다.
#[test]
fn reconcile_resets_clean_detached_slots_after_empty_prerelease_baseline_moves() {
    let repo = TempGitRepo::new("reset-detached-slots");
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);

    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");
    let refreshed_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(refreshed_pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(refreshed_pool.blocked_slots, 0);
    assert!(refreshed_pool.slots.iter().all(|slot| {
        !slot.worktree_label.contains(&format!(
            "detached away from `{POOL_BASELINE_BRANCH}` baseline"
        ))
    }));
}

// agent slot worktree에서 reconcile을 호출해도 canonical 표준 branch는 agent
// branch HEAD로 갱신되면 안 된다. root detection이 slot workspace를 원본 repo로
// 되돌려 계산하는지 확인하는 회귀 테스트다.
#[test]
fn reconcile_does_not_refresh_prerelease_from_agent_slot_workspace() {
    let repo = TempGitRepo::new("agent-slot-does-not-reset-akra");
    let slot_path = repo.create_agent_slot(1, "task-one");
    let original_prerelease_head = run_command(
        "git",
        [
            "-C",
            repo.repo_root.to_str().expect("repo root should be utf-8"),
            "rev-parse",
            POOL_BASELINE_BRANCH,
        ],
        None,
    )
    .expect("prerelease should resolve");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    assert_ne!(
        original_prerelease_head,
        run_command(
            "git",
            [
                "-C",
                slot_path.to_str().expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("slot head should resolve")
    );
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        slot_path.to_str().expect("slot path should be utf-8"),
    );

    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .expect("prerelease should resolve"),
        original_prerelease_head
    );
    assert!(pool.blocked_slots > 0);
}

// merged agent slot은 cleanup pending 상태에서 reconcile이 완전히 회수할 수 있어야
// 한다. untracked scratch 파일, agent branch, lease mirror가 모두 제거되고 slot이
// detached 표준 branch idle 상태로 돌아오는 end-to-end cleanup 계약을 고정한다.
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
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let slot = &pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::Idle);
    assert!(slot.branch_name.starts_with(POOL_BASELINE_BRANCH));
    assert!(!slot_path.join("scratch.tmp").exists());
    assert!(!repo.branch_exists(&branch_name));
    assert!(!repo.slot_lease_path(1).exists());
    assert!(pool.reconcile_status.contains("cleaned 1"));
}
