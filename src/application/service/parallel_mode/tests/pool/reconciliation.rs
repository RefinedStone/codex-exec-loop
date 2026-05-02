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

// 한 slot이 running인 동안에도 다른 idle baseline들은 최신 prerelease로 정리될 수
// 있어야 한다. 이 테스트는 실행 중인 lease를 보존하면서 reusable slot만 reset하고,
// canonical prerelease ref도 현재 head로 갱신되는지 함께 확인한다.
#[test]
fn reconcile_resets_reusable_detached_slots_while_another_slot_is_running() {
    let repo = TempGitRepo::new("dirty-reusable-slot-with-running-lease");
    let service = test_parallel_mode_service();
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
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
    let reusable_slot_path = repo.pool_root().join(slot_id(2));
    fs::write(reusable_slot_path.join("README.md"), "dirty\n")
        .expect("idle slot should become dirty");
    let refreshed_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
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
                "prerelease",
            ],
            None,
        )
        .expect("prerelease should resolve"),
        current_head
    );
}

// reconcile은 비어 있는 pool root를 실제 capacity로 바꾸는 provisioning 단계다.
// 모든 slot worktree가 생성되고 missing count가 사라져야 dispatcher가 곧바로
// idle slot을 사용할 수 있다.
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

// 사용자가 로컬 `prerelease` branch를 지웠더라도 reconcile은 baseline ref를 먼저
// 복구한 뒤 slot을 provision해야 한다. slot 생성과 branch 복구가 같은 흐름에서
// 일어나야 이후 slot들이 모두 동일한 기준 commit을 바라본다.
#[test]
fn reconcile_creates_local_prerelease_branch_before_provisioning_slots() {
    let repo = TempGitRepo::new("create-akra");
    repo.delete_local_prerelease_branch();
    assert!(!repo.branch_exists("prerelease"));
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert!(repo.branch_exists("prerelease"));
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert!(pool.reconcile_status.contains("created `prerelease`"));
}

// empty baseline은 과거 `prerelease` head에 머물 수 있으므로 reconcile이 현재
// workspace head를 새 baseline으로 채택해야 한다. 이 테스트는 branch ref 자체가
// 이동했는지 확인해 단순 slot count 성공에 가려지는 stale baseline을 잡는다.
#[test]
fn reconcile_resets_empty_prerelease_baseline_to_current_head() {
    let repo = TempGitRepo::new("reset-akra");
    let old_prerelease_head = repo.head_sha();
    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");
    let current_head = repo.head_sha();
    assert_ne!(old_prerelease_head, current_head);
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
                "prerelease",
            ],
            None,
        )
        .expect("prerelease should resolve"),
        current_head
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
            .contains("detached away from `prerelease` baseline")
    }));
}

// agent slot worktree에서 reconcile을 호출해도 canonical `prerelease`는 agent
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
            "prerelease",
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
        slot_path.to_str().expect("slot path should be utf-8"),
    );

    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                "prerelease",
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
// detached prerelease idle 상태로 돌아오는 end-to-end cleanup 계약을 고정한다.
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
    assert!(slot.branch_name.starts_with("prerelease"));
    assert!(!slot_path.join("scratch.tmp").exists());
    assert!(!repo.branch_exists(&branch_name));
    assert!(!repo.slot_lease_path(1).exists());
    assert!(pool.reconcile_status.contains("cleaned 1"));
}
