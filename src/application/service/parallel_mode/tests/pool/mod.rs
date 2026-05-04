use super::*;
use std::collections::BTreeSet;
use std::sync::Barrier;

// 디스패치 계획 테스트는 실제 planner 전체를 띄우지 않고도 큐 우선순위와
// `next_task` 파생값이 같은 입력에서 만들어졌는지만 검증하면 충분하다.
// 이 helper는 활성 task 목록을 그대로 rank 순서의 ready snapshot으로 접어,
// pool 서비스가 planner snapshot을 소비하는 경계만 작게 고정한다.
fn planning_snapshot_with_active_tasks(task_ids: &[&str]) -> PlanningRuntimeSnapshot {
    let active_tasks = task_ids
        .iter()
        .enumerate()
        .map(|(index, task_id)| queue_task(index + 1, task_id))
        .collect::<Vec<_>>();
    let queue_projection = PriorityQueueProjection {
        next_task: active_tasks.first().cloned(),
        active_tasks,
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    PlanningRuntimeSnapshot::ready_with_queue_projection(
        "prompt".to_string(),
        queue_projection.queue_summary(),
        None,
        queue_projection.next_task.clone(),
        queue_projection,
    )
}

// synthetic queue task는 테스트마다 priority, timestamp, title을 손으로 반복하면
// 어떤 필드가 dispatch 정렬의 핵심인지 흐려진다. rank를 단일 입력으로 삼아
// 후보 순서와 우선순위가 함께 움직이도록 만들어 fixture의 의도를 드러낸다.
fn queue_task(rank: usize, task_id: &str) -> PriorityQueueTask {
    PriorityQueueTask {
        rank,
        task_id: task_id.to_string(),
        direction_id: "direction-1".to_string(),
        direction_title: "Direction".to_string(),
        task_title: format!("Task {rank}"),
        status: TaskStatus::Ready,
        combined_priority: 100 - rank as i32,
        updated_at: format!("2026-04-30T00:0{rank}:00Z"),
        rank_reasons: vec!["ready".to_string()],
    }
}

// readiness snapshot이 없거나 repository 상태를 읽을 수 없는 board는 모든 slot을
// unavailable로 표시해야 하지만, 이것이 "pool capacity를 모두 소진했다"는 뜻은
// 아니다. exhausted는 사용 가능한 pool 안에서 더 배정할 자리가 없을 때만 켜진다.
#[test]
fn unavailable_pool_board_does_not_report_exhausted() {
    let pool = build_pool_board(&SqlitePlanningAuthorityAdapter::new(), "/tmp/root", None);

    assert_eq!(pool.unavailable_slots, DEFAULT_POOL_SIZE);
    assert!(!pool.exhausted);
}

// dispatcher는 planner queue가 더 많은 task를 제안해도 idle slot 수만큼만 후보를
// 내보내야 한다. 이 테스트는 기본 pool 크기와 active task 순서가 candidate
// truncation의 기준으로 유지되는지 확인한다.
#[test]
fn build_dispatch_plan_fills_idle_slots_with_distinct_active_tasks() {
    let repo = TempGitRepo::new("dispatch-fill");
    let service = test_parallel_mode_service();
    let planning_snapshot =
        planning_snapshot_with_active_tasks(&["task-1", "task-2", "task-3", "task-4"]);
    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &planning_snapshot, usize::MAX)
        .expect("dispatch plan should build");

    assert_eq!(plan.idle_slot_count, DEFAULT_POOL_SIZE);
    assert_eq!(
        plan.candidates
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-1", "task-2", "task-3"]
    );
}

// slot lease와 distributor queue는 이미 작업이 진행 중인 task를 나타내는 두
// 소스다. dispatch plan은 둘을 함께 제외해야 같은 task가 새 slot과 integration
// queue 양쪽에서 중복 처리되지 않는다.
#[test]
fn build_dispatch_plan_excludes_leased_and_queued_tasks() {
    let repo = TempGitRepo::new("dispatch-excludes");
    let service = test_parallel_mode_service();
    let planning_snapshot =
        planning_snapshot_with_active_tasks(&["task-1", "task-2", "task-3", "task-4"]);
    let _leased = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task 1", "agent-task-1", "task-1"),
        )
        .expect("task-1 lease should be acquired");
    let queued_record = PlanningAuthorityDistributorQueueRecord {
        queue_item_id: "queued-task-2".to_string(),
        queue_order_key: 1,
        session_key: "slot-2@queued".to_string(),
        root_turn_id: Some("turn-task-2".to_string()),
        slot_id: "slot-2".to_string(),
        agent_id: "agent-task-2".to_string(),
        task_id: "task-2".to_string(),
        task_title: "Task 2".to_string(),
        source_branch: "akra-agent/slot-2/task-2".to_string(),
        source_commit_sha: repo.head_sha(),
        branch_name: "akra-agent/slot-2/task-2".to_string(),
        worktree_path: repo.workspace_dir(),
        commit_sha: repo.head_sha(),
        original_commit_sha: None,
        planning_refresh_state: "done".to_string(),
        integration_state: "queued".to_string(),
        conflict_files: Vec::new(),
        recovery_note: None,
        validation_summary: "queued".to_string(),
        authority_refresh_outcome: "official".to_string(),
        github_capabilities: None,
        pull_request_number: None,
        pull_request_url: None,
        queue_state: ParallelModeQueueItemState::Queued,
        integration_note: "queued for distributor".to_string(),
        enqueued_at: "2026-04-30T00:00:00Z".to_string(),
        updated_at: "2026-04-30T00:00:00Z".to_string(),
    };
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &queued_record,
    )
    .expect("queued distributor record should be stored");
    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &planning_snapshot, usize::MAX)
        .expect("dispatch plan should build");

    assert_eq!(plan.idle_slot_count, DEFAULT_POOL_SIZE - 1);
    assert_eq!(plan.excluded_task_ids, vec!["task-1", "task-2"]);
    assert_eq!(
        plan.candidates
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-3", "task-4"]
    );
}

// lease 획득은 여러 agent가 동시에 idle slot을 잡으려는 첫 관문이다. barrier로
// 경쟁을 한 번에 시작시킨 뒤 성공 수가 pool 크기와 같고, 초과 요청은 같은
// exhaustion 오류로 떨어지는지 확인해 allocation lock의 직렬화 계약을 고정한다.
#[test]
fn concurrent_slot_lease_requests_are_serialized_across_idle_slots() {
    let repo = TempGitRepo::new("concurrent-lease");
    let service = Arc::new(test_parallel_mode_service());
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);
    let worker_count = DEFAULT_POOL_SIZE + 3;
    let barrier = Arc::new(Barrier::new(worker_count));
    let mut handles = Vec::new();
    for worker_index in 1..=worker_count {
        let service = service.clone();
        let barrier = barrier.clone();
        let workspace_dir = repo.workspace_dir();
        handles.push(thread::spawn(move || {
            barrier.wait();
            service.acquire_slot_lease(
                &workspace_dir,
                sample_lease_request(
                    &format!("task-{worker_index}"),
                    &format!("Task {worker_index}"),
                    &format!("agent-{worker_index}"),
                    &format!("task-{worker_index}"),
                ),
            )
        }));
    }
    let mut leases = Vec::new();
    let mut errors = Vec::new();
    for handle in handles {
        match handle
            .join()
            .expect("concurrent lease worker should not panic")
        {
            Ok(lease) => leases.push(lease),
            Err(error) => errors.push(error),
        }
    }
    let leased_slot_ids = leases
        .iter()
        .map(|lease| lease.slot_id.as_str())
        .collect::<BTreeSet<_>>();
    let leased_task_ids = leases
        .iter()
        .map(|lease| lease.task_id.as_str())
        .collect::<BTreeSet<_>>();
    let leased_agent_ids = leases
        .iter()
        .map(|lease| lease.agent_id.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(leases.len(), DEFAULT_POOL_SIZE);
    assert_eq!(leased_slot_ids.len(), DEFAULT_POOL_SIZE);
    assert_eq!(leased_task_ids.len(), DEFAULT_POOL_SIZE);
    assert_eq!(leased_agent_ids.len(), DEFAULT_POOL_SIZE);
    assert_eq!(errors.len(), worker_count - DEFAULT_POOL_SIZE);
    assert!(
        errors
            .iter()
            .all(|error| error == "no idle slot is available for lease"),
        "unexpected concurrent lease errors: {errors:?}"
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            vec![],
            None,
        )),
    );
    assert_eq!(pool.leased_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.blocked_slots, 0);
    assert_eq!(pool.idle_slots, 0);
}

// 사용자가 로컬 표준 branch를 삭제한 linked-worktree 상태에서도 pool baseline은 표준 remote
// branch를 기준으로 계산되어야 한다. remote fallback이
// 없으면 정상 slot들이 blocked로 오인되어 parallel mode가 불필요하게 멈춘다.
#[test]
fn build_pool_board_uses_remote_prerelease_when_local_branch_is_missing() {
    let repo = TempGitRepo::new("remote-akra");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let head_sha = repo.head_sha();
    repo.delete_local_prerelease_branch();
    repo.set_remote_tracking_branch(&remote_standard_branch_name(), &head_sha);
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

// TUI와 app-server는 하위 디렉터리에서 호출될 수 있으므로 pool store의 기준점은
// 현재 working directory가 아니라 git common dir에서 역산한 canonical repo root다.
// nested workspace 입력이 같은 repository root로 수렴하는지 확인한다.
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

// linked worktree의 planning 파일이 canonical repository의 authority shadow store와
// 달라질 수 있다. readiness 검사는 파일 시스템의 우연한 worktree 복사본보다
// canonical root에 저장된 authority record를 신뢰해야 한다.
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

// 표준 remote branch가 아직 없더라도 일반 workspace HEAD가 있으면 readiness는 통과시킨다.
// 실제 표준 branch 생성과 push는 mutating reconcile 단계에서 한 번에 수행된다.
#[test]
fn inspect_readiness_allows_missing_origin_standard_branch_when_head_can_seed() {
    let repo = TempGitRepo::new("missing-origin-prerelease");
    repo.create_bare_origin_remote();
    repo.delete_remote_standard_tracking_branch();
    let service = test_parallel_mode_service();
    let snapshot = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeSnapshot::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    let capability = snapshot
        .capability(ParallelModeCapabilityKey::AkraBranch)
        .expect("akra branch capability should exist");

    assert_eq!(capability.state, ParallelModeCapabilityState::Ready);
    assert!(
        capability
            .summary()
            .contains(&remote_standard_branch_name())
    );
    assert!(capability.summary().contains("current HEAD"));
}

// 표준 remote branch를 새로 seed해야 하는 상태에서는 push remote가 필수다. remote-tracking ref도
// push remote도 없는데 readiness가 degraded로 통과하면 `:parallel`이 곧바로 reconcile 실패로
// 이어진다.
#[test]
fn inspect_readiness_blocks_missing_standard_branch_when_push_remote_is_absent() {
    let repo = TempGitRepo::new("missing-standard-no-push-remote");
    repo.delete_remote_standard_tracking_branch();
    let service = test_parallel_mode_service();
    let snapshot = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeSnapshot::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    let capability = snapshot
        .capability(ParallelModeCapabilityKey::AkraBranch)
        .expect("akra branch capability should exist");

    assert_eq!(capability.state, ParallelModeCapabilityState::Blocked);
    assert!(capability.summary().contains("cannot be seeded"));
    assert!(!snapshot.allows_parallel_mode());
}

// pool board의 reconciliation 세부 규칙은 missing/blocked/leased 상태 조합이 많아
// 별도 모듈로 분리한다. 이 파일은 dispatch와 root detection의 큰 흐름만 맡는다.
mod reconciliation;

// lease lifecycle은 acquisition, release, stale cleanup처럼 상태 전이가 길게 이어져
// 전용 모듈에서 slot store 계약을 더 촘촘히 검증한다.
mod lease;
