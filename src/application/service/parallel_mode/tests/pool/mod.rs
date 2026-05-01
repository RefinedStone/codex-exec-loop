use super::*;
use std::collections::BTreeSet;
use std::sync::Barrier;

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

#[test]
fn unavailable_pool_board_does_not_report_exhausted() {
    let pool = build_pool_board(&SqlitePlanningAuthorityAdapter::new(), "/tmp/root", None);

    assert_eq!(pool.unavailable_slots, DEFAULT_POOL_SIZE);
    assert!(!pool.exhausted);
}

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
    repo.set_remote_tracking_branch("origin/prerelease", &head_sha);

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

mod reconciliation;

mod lease;
