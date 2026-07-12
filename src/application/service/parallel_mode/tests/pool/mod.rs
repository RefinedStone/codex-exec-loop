use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use std::collections::BTreeSet;
use std::sync::{Barrier, Mutex, mpsc};

// 디스패치 계획 테스트는 실제 planning worker 전체를 띄우지 않고도 큐 우선순위와
// `next_task` 파생값이 같은 입력에서 만들어졌는지만 검증하면 충분하다.
// 이 helper는 활성 task 목록을 그대로 rank 순서의 ready projection으로 접어,
// pool 서비스가 planning snapshot을 소비하는 경계만 작게 고정한다.
fn planning_projection_with_active_tasks(task_ids: &[&str]) -> PlanningRuntimeProjection {
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
    PlanningRuntimeProjection::ready_with_queue_projection(
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

fn install_runtime_insert_failure(repo: &TempGitRepo, trigger_name: &str, table_name: &str) {
    let authority = SqlitePlanningAuthorityAdapter::new();
    let location = authority
        .resolve_authority_location(&repo.workspace_dir())
        .expect("authority location should resolve before fault injection");
    let connection = rusqlite::Connection::open(&location.authority_store_path)
        .expect("authority store should open for fault injection");
    connection
        .execute_batch(&format!(
            "CREATE TRIGGER {trigger_name}
             BEFORE INSERT ON {table_name}
             BEGIN
                 SELECT RAISE(FAIL, 'forced {table_name} insert failure');
             END;"
        ))
        .expect("authority write failure trigger should install");
}

fn delete_authoritative_session_detail(repo: &TempGitRepo, session_key: &str) {
    let authority = SqlitePlanningAuthorityAdapter::new();
    let location = authority
        .resolve_authority_location(&repo.workspace_dir())
        .expect("authority location should resolve before deleting the session fixture");
    let connection = rusqlite::Connection::open(&location.authority_store_path)
        .expect("authority store should open for session fixture deletion");
    let deleted = connection
        .execute(
            "DELETE FROM runtime_session_details WHERE session_key = ?1",
            [session_key],
        )
        .expect("authority session fixture should be deleted");
    assert_eq!(deleted, 1);
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

#[test]
fn blocked_readiness_pool_board_reports_supervisor_gate_without_reconcile() {
    let repo = TempGitRepo::new("pool-readiness-blocked");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Blocked,
        vec![ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitWorktree,
            ParallelModeCapabilityState::Blocked,
            "worktree inventory failed",
            Some("repair worktree metadata".to_string()),
        )],
        Some("worktree inventory failed".to_string()),
    );

    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert_eq!(pool.unavailable_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.missing_slots, 0);
    assert!(pool.reconcile_status.contains("readiness: blocked"));
    assert!(
        pool.slots
            .iter()
            .all(|slot| slot.owner_label == "supervisor gate")
    );
    assert!(!repo.pool_root().exists());
}

// dispatcher는 planning queue가 더 많은 task를 제안해도 idle slot 수만큼만 후보를
// 내보내야 한다. 이 테스트는 기본 pool 크기와 active task 순서가 candidate
// truncation의 기준으로 유지되는지 확인한다.
#[test]
fn build_dispatch_plan_fills_idle_slots_with_distinct_active_tasks() {
    let repo = TempGitRepo::new("dispatch-fill");
    let service = test_parallel_mode_service();
    let planning_projection =
        planning_projection_with_active_tasks(&["task-1", "task-2", "task-3", "task-4"]);
    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &planning_projection, usize::MAX)
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

// 일부 worktree가 blocked 상태여도 남은 idle slot까지 dispatcher가 멈추면 안 된다.
// blocked slot은 capacity만 줄이고, ready queue의 다음 작업은 사용 가능한 slot 수만큼
// 계속 후보로 올라와야 한다.
#[test]
fn build_dispatch_plan_uses_remaining_idle_capacity_when_other_worktrees_are_blocked() {
    let repo = TempGitRepo::new("dispatch-with-blocked-slots");
    let service = test_parallel_mode_service();
    repo.create_detached_slot(1);
    let blocked_slot_two = repo.create_agent_slot(2, "blocked-task-two");
    let blocked_slot_three = repo.create_agent_slot(3, "blocked-task-three");
    repo.commit_file_in_slot(
        &blocked_slot_two,
        "slot-two.txt",
        "blocked slot two\n",
        "blocked slot two work",
    );
    repo.commit_file_in_slot(
        &blocked_slot_three,
        "slot-three.txt",
        "blocked slot three\n",
        "blocked slot three work",
    );
    let planning_projection =
        planning_projection_with_active_tasks(&["task-1", "task-2", "task-3", "task-4"]);

    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &planning_projection, usize::MAX)
        .expect("dispatch plan should build with blocked slots");

    assert_eq!(plan.idle_slot_count, 1);
    assert_eq!(
        plan.candidates
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-1"]
    );
}

// slot lease와 distributor queue는 이미 작업이 진행 중인 task를 나타내는 두
// 소스다. dispatch plan은 둘을 함께 제외해야 같은 task가 새 slot과 integration
// queue 양쪽에서 중복 처리되지 않는다.
#[test]
fn build_dispatch_plan_excludes_leased_and_queued_tasks() {
    let repo = TempGitRepo::new("dispatch-excludes");
    let service = test_parallel_mode_service();
    let planning_projection =
        planning_projection_with_active_tasks(&["task-1", "task-2", "task-3", "task-4"]);
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
        slot_id: "slot-2".to_string(),
        agent_id: "agent-task-2".to_string(),
        task_id: "task-2".to_string(),
        task_title: "Task 2".to_string(),
        delivery_target: None,
        source_branch: "akra-agent/slot-2/task-2".to_string(),
        source_base_commit_sha: "base".to_string(),
        source_commit_sha: repo.head_sha(),
        branch_name: "akra-agent/slot-2/task-2".to_string(),
        worktree_path: repo.workspace_dir(),
        commit_sha: repo.head_sha(),
        original_commit_sha: None,
        planning_refresh_state: "done".to_string(),
        integration_state: "queued".to_string(),
        integration_base_commit_sha: None,
        integration_commit_sha: None,
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
        retry_attempts: 0,
        retry_not_before: None,
    };
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &queued_record,
    )
    .expect("queued distributor record should be stored");
    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &planning_projection, usize::MAX)
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

// startup 실패는 lease가 제거되어 pool board만 보면 idle slot처럼 보인다. 하지만 같은
// planning task가 그대로 ready이면 dispatcher가 즉시 같은 실패를 반복할 수 있으므로,
// task가 다시 갱신되기 전까지는 dispatch 후보에서 제외한다.
#[test]
fn build_dispatch_plan_excludes_failed_start_tasks_until_task_changes() {
    let repo = TempGitRepo::new("dispatch-excludes-failed-start");
    let service = test_parallel_mode_service();
    let planning_projection =
        planning_projection_with_active_tasks(&["task-1", "task-2", "task-3"]);
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task 1", "agent-task-1", "task-1"),
        )
        .expect("task-1 lease should be acquired");

    service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect("failed start should release slot")
        .expect("lease should be released");
    let runtime_projection_before_reset =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projection should load after failed start");
    assert_eq!(runtime_projection_before_reset.session_details.len(), 1);
    assert_eq!(
        runtime_projection_before_reset.task_dispatch_blocks.len(),
        1
    );
    service
        .reset_pool_on_parallel_enable(&repo.workspace_dir())
        .expect("pool reset should preserve failed-start dispatch block");
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projection should load after pool reset");
    assert_eq!(runtime_projection.task_dispatch_blocks.len(), 1);

    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &planning_projection, usize::MAX)
        .expect("dispatch plan should build");

    assert_eq!(plan.idle_slot_count, DEFAULT_POOL_SIZE);
    assert_eq!(plan.excluded_task_ids, vec!["task-1"]);
    assert_eq!(
        plan.candidates
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-2", "task-3"]
    );
}

// failed-start 차단은 영구 blacklist가 아니다. planning task가 실패 기록보다 새로
// 갱신되면 사용자가 큐를 수정하거나 재준비한 것으로 보고 다시 dispatch할 수 있어야 한다.
#[test]
fn build_dispatch_plan_allows_failed_start_task_after_task_changes() {
    let repo = TempGitRepo::new("dispatch-retries-updated-task");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task 1", "agent-task-1", "task-1"),
        )
        .expect("task-1 lease should be acquired");

    service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect("failed start should release slot")
        .expect("lease should be released");

    let mut task = queue_task(1, "task-1");
    task.updated_at = "2999-01-01T00:00:00Z".to_string();
    let queue_projection = PriorityQueueProjection {
        next_task: Some(task.clone()),
        active_tasks: vec![task],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    let planning_projection = PlanningRuntimeProjection::ready_with_queue_projection(
        "prompt".to_string(),
        queue_projection.queue_summary(),
        None,
        queue_projection.next_task.clone(),
        queue_projection,
    );

    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &planning_projection, usize::MAX)
        .expect("dispatch plan should build");

    assert_eq!(plan.idle_slot_count, DEFAULT_POOL_SIZE);
    assert!(plan.excluded_task_ids.is_empty());
    assert_eq!(
        plan.candidates
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-1"]
    );
}

#[test]
fn operator_update_during_failed_start_cleanup_is_not_reblocked_by_later_detail_write() {
    let repo = TempGitRepo::new("failed-start-update-during-cleanup");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task 1", "agent-task-1", "task-1"),
        )
        .expect("task lease should be acquired");
    let operator_updated_at = Mutex::new(None::<String>);

    service
        .release_workspace_slot_lease_after_failed_start_with_test_hook(
            &lease.worktree_path,
            || {
                let runtime_projection =
                    SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
                        .expect("pre-cleanup dispatch block should already be durable");
                let blocked_at = chrono::DateTime::parse_from_rfc3339(
                    &runtime_projection.task_dispatch_blocks[0].blocked_at,
                )
                .expect("dispatch block timestamp should be RFC3339");
                let updated_at = (blocked_at + chrono::TimeDelta::milliseconds(1)).to_rfc3339();
                *operator_updated_at
                    .lock()
                    .expect("operator update timestamp mutex should not be poisoned") =
                    Some(updated_at);
                std::thread::sleep(std::time::Duration::from_millis(20));
            },
        )
        .expect("failed start cleanup should complete")
        .expect("leased workspace should be released");

    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projection should include failure records");
    let block = &runtime_projection.task_dispatch_blocks[0];
    let detail = runtime_projection
        .session_details
        .iter()
        .find(|detail| detail.session_key == lease.session_key())
        .expect("failed-start detail should be persisted");
    assert_eq!(detail.updated_at, block.blocked_at);

    let mut updated_task = queue_task(1, "task-1");
    updated_task.updated_at = operator_updated_at
        .into_inner()
        .expect("operator update timestamp mutex should not be poisoned")
        .expect("operator update should occur after pre-block");
    let updated_queue = PriorityQueueProjection {
        next_task: Some(updated_task.clone()),
        active_tasks: vec![updated_task],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    let updated_projection = PlanningRuntimeProjection::ready_with_queue_projection(
        "prompt".to_string(),
        updated_queue.queue_summary(),
        None,
        updated_queue.next_task.clone(),
        updated_queue,
    );
    let plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &updated_projection, usize::MAX)
        .expect("operator-updated task dispatch plan should build");
    assert!(plan.excluded_task_ids.is_empty());
    assert_eq!(plan.candidates[0].task_id, "task-1");
}

#[test]
fn failed_start_block_survives_session_detail_authority_failure_and_allows_updated_task() {
    let repo = TempGitRepo::new("failed-start-detail-authority-failure");
    let service = test_parallel_mode_service();
    let unchanged_projection = planning_projection_with_active_tasks(&["task-1"]);
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task 1", "agent-task-1", "task-1"),
        )
        .expect("task lease should be acquired");
    install_runtime_insert_failure(
        &repo,
        "fail_failed_start_session_detail",
        "runtime_session_details",
    );

    let error = service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect_err("session detail failure should remain observable after cleanup");
    assert!(error.contains("failed to store agent session detail"));
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));

    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projection should retain the failed-start fence");
    assert_eq!(runtime_projection.task_dispatch_blocks.len(), 1);
    assert_eq!(runtime_projection.task_dispatch_blocks[0].task_id, "task-1");
    let unchanged_plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &unchanged_projection, usize::MAX)
        .expect("unchanged task dispatch plan should build");
    assert_eq!(unchanged_plan.excluded_task_ids, vec!["task-1"]);
    assert!(unchanged_plan.candidates.is_empty());

    let mut updated_task = queue_task(1, "task-1");
    updated_task.updated_at = "2999-01-01T00:00:00Z".to_string();
    let updated_queue = PriorityQueueProjection {
        next_task: Some(updated_task.clone()),
        active_tasks: vec![updated_task],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    let updated_projection = PlanningRuntimeProjection::ready_with_queue_projection(
        "prompt".to_string(),
        updated_queue.queue_summary(),
        None,
        updated_queue.next_task.clone(),
        updated_queue,
    );
    let updated_plan = service
        .build_dispatch_plan(&repo.workspace_dir(), &updated_projection, usize::MAX)
        .expect("operator-updated task dispatch plan should build");
    assert!(updated_plan.excluded_task_ids.is_empty());
    assert_eq!(updated_plan.candidates[0].task_id, "task-1");
}

#[test]
fn failed_start_cleanup_treats_session_detail_mirror_as_output_only() {
    let repo = TempGitRepo::new("failed-start-detail-mirror-failure");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task 1", "agent-task-1", "task-1"),
        )
        .expect("task lease should be acquired");
    let detail_path = repo.session_detail_path(&lease.session_key());
    fs::remove_file(&detail_path).expect("assigned detail mirror should be removable");
    fs::create_dir(&detail_path).expect("directory collision should block mirror rename");

    let released = service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect("authority-backed failed-start cleanup should ignore mirror projection failure")
        .expect("failed-start cleanup should release the slot");
    assert!(released.same_generation_as(&lease));
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));

    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projection should retain the failed-start fence");
    assert_eq!(runtime_projection.task_dispatch_blocks.len(), 1);
    assert_eq!(runtime_projection.task_dispatch_blocks[0].task_id, "task-1");
    assert_eq!(runtime_projection.session_details.len(), 1);
    assert_eq!(runtime_projection.session_details[0].state_label, "failed");
}

#[test]
fn legacy_session_mirror_seeds_only_the_matching_lease_identity() {
    #[derive(Clone, Copy)]
    enum MirrorFixture {
        SanitizerCollision,
        SameKeyDifferentIdentity,
        ValidLegacy,
    }

    for (fixture_name, fixture) in [
        (
            "legacy-session-sanitizer-collision",
            MirrorFixture::SanitizerCollision,
        ),
        (
            "legacy-session-same-key-wrong-identity",
            MirrorFixture::SameKeyDifferentIdentity,
        ),
        ("legacy-session-valid-bootstrap", MirrorFixture::ValidLegacy),
    ] {
        let repo = TempGitRepo::new(fixture_name);
        let service = test_parallel_mode_service();
        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            )
            .expect("slot lease should be acquired");
        service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should transition to running");

        let session_key = lease.session_key();
        let mirror_path = repo.session_detail_path(&session_key);
        let mut legacy = read_agent_session_detail_record(
            &test_parallel_runtime(),
            &repo.pool_root(),
            &session_key,
        )
        .expect("running session mirror should exist before authority deletion");
        delete_authoritative_session_detail(&repo, &session_key);

        match fixture {
            MirrorFixture::SanitizerCollision => {
                legacy.session_key = session_key.replacen('@', "/", 1);
                legacy.slot_id = "foreign-slot".to_string();
                legacy.agent_id = "foreign-agent".to_string();
                legacy.task_id = "foreign-task".to_string();
                legacy.task_title = "Foreign Task".to_string();
                legacy.branch_name = "foreign-branch".to_string();
                legacy.worktree_path = "/tmp/foreign-worktree".to_string();
                legacy.lease_started_at = "1999-01-01T00:00:00Z".to_string();
                legacy.validation_summary = "foreign validation must not persist".to_string();
                assert_eq!(
                    agent_session_detail_record_path(&repo.pool_root(), &legacy.session_key),
                    mirror_path,
                    "the foreign session key should collide only after filename sanitization"
                );
            }
            MirrorFixture::SameKeyDifferentIdentity => {
                legacy.agent_id = "foreign-agent".to_string();
                legacy.validation_summary = "foreign validation must not persist".to_string();
            }
            MirrorFixture::ValidLegacy => {
                legacy.validation_summary = "valid legacy validation is preserved".to_string();
                let mut marker = legacy
                    .history
                    .last()
                    .cloned()
                    .expect("running legacy detail should have history");
                marker.state_label = "legacy_marker".to_string();
                marker.summary = "valid legacy history is preserved".to_string();
                legacy.history.push(marker);
            }
        }
        fs::write(
            &mirror_path,
            serde_json::to_string_pretty(&legacy).expect("legacy mirror should serialize"),
        )
        .expect("legacy mirror fixture should be installed");

        service
            .mark_workspace_commit_ready(
                &lease.worktree_path,
                "authority accepted the identity-checked completion",
            )
            .expect("identity-checked commit-ready should persist")
            .expect("running lease should transition to commit-ready");

        let projection =
            SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
                .expect("authority projection should load after commit-ready");
        assert_eq!(projection.session_details.len(), 1);
        let detail = &projection.session_details[0];
        assert_eq!(detail.session_key, session_key);
        assert_eq!(detail.slot_id, lease.slot_id);
        assert_eq!(detail.agent_id, lease.agent_id);
        assert_eq!(detail.task_id, lease.task_id);
        assert_eq!(detail.task_title, lease.task_title);
        assert_eq!(detail.branch_name, lease.branch_name);
        assert_eq!(detail.worktree_path, lease.worktree_path);
        assert_eq!(detail.lease_started_at, lease.leased_at);
        assert_eq!(detail.state_label, "commit_ready");
        assert_eq!(
            detail.authority_refresh_outcome,
            "authority accepted the identity-checked completion"
        );
        match fixture {
            MirrorFixture::ValidLegacy => {
                assert_eq!(
                    detail.validation_summary,
                    "valid legacy validation is preserved"
                );
                assert!(detail.history.iter().any(|entry| {
                    entry.state_label == "legacy_marker"
                        && entry.summary == "valid legacy history is preserved"
                }));
            }
            MirrorFixture::SanitizerCollision | MirrorFixture::SameKeyDifferentIdentity => {
                assert_ne!(
                    detail.validation_summary,
                    "foreign validation must not persist"
                );
                assert!(
                    detail
                        .history
                        .iter()
                        .all(|entry| !entry.summary.contains("foreign"))
                );
            }
        }
    }
}

#[test]
fn failed_start_dispatch_block_failure_preserves_lease_branch_and_worktree() {
    let repo = TempGitRepo::new("failed-start-block-write-failure");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task 1", "agent-task-1", "task-1"),
        )
        .expect("task lease should be acquired");
    install_runtime_insert_failure(
        &repo,
        "fail_failed_start_dispatch_block",
        "runtime_task_dispatch_blocks",
    );

    let error = service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect_err("failed dispatch fence must stop cleanup");
    assert!(error.contains("failed to store startup failure dispatch block"));
    assert!(repo.slot_lease_path(1).exists());
    assert!(repo.branch_exists(&lease.branch_name));
    assert_eq!(
        current_branch(&PathBuf::from(&lease.worktree_path)),
        lease.branch_name
    );

    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projection should retain the active lease");
    assert!(runtime_projection.slot_leases.contains_key("slot-1"));
    assert!(runtime_projection.task_dispatch_blocks.is_empty());
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
        &test_parallel_runtime(),
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
            .all(|error| error == "no remote-verified idle slot is available for lease"),
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
fn reconcile_waits_for_checkout_and_lease_persistence_under_one_pool_mutation_lock() {
    let repo = TempGitRepo::new("allocation-reconcile-barrier");
    let service = Arc::new(test_parallel_mode_service());
    let workspace_dir = repo.workspace_dir();
    let (checkout_reached_tx, checkout_reached_rx) = mpsc::channel();
    let (resume_allocation_tx, resume_allocation_rx) = mpsc::channel();
    let allocator_service = service.clone();
    let allocator_workspace = workspace_dir.clone();
    let allocator = thread::spawn(move || {
        allocator_service.acquire_slot_lease_with_test_hook(
            &allocator_workspace,
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            move || {
                checkout_reached_tx
                    .send(())
                    .expect("allocator barrier should signal checkout");
                resume_allocation_rx
                    .recv()
                    .expect("allocator barrier should resume lease persistence");
            },
        )
    });
    checkout_reached_rx
        .recv()
        .expect("allocator should pause after checkout");

    assert!(
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&workspace_dir)
            .expect("authority projection should load while allocation is paused")
            .slot_leases
            .is_empty(),
        "lease persistence must occur after the allocation barrier"
    );
    let (reconcile_started_tx, reconcile_started_rx) = mpsc::channel();
    let (reconcile_done_tx, reconcile_done_rx) = mpsc::channel();
    let reconcile_workspace = workspace_dir.clone();
    let reconciler = thread::spawn(move || {
        reconcile_started_tx
            .send(())
            .expect("reconcile start should signal");
        let board = reconcile_pool_board(
            &SqlitePlanningAuthorityAdapter::new(),
            &test_parallel_runtime(),
            &reconcile_workspace,
        );
        reconcile_done_tx
            .send(board)
            .expect("reconcile completion should signal");
    });
    reconcile_started_rx
        .recv()
        .expect("concurrent reconcile should start");
    assert!(
        reconcile_done_rx
            .recv_timeout(std::time::Duration::from_millis(150))
            .is_err(),
        "reconcile must wait while checkout has not yet persisted its lease"
    );

    resume_allocation_tx
        .send(())
        .expect("allocator should resume");
    let lease = allocator
        .join()
        .expect("allocator thread should not panic")
        .expect("allocator should persist the lease");
    let board = reconcile_done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("reconcile should finish after allocation releases the permit");
    reconciler
        .join()
        .expect("reconcile thread should not panic");

    let projection = SqlitePlanningAuthorityAdapter::load_runtime_projections(&workspace_dir)
        .expect("authority projection should retain allocated lease");
    assert_eq!(projection.slot_leases.get(&lease.slot_id), Some(&lease));
    assert!(repo.branch_exists(&lease.branch_name));
    assert_eq!(
        current_branch(&PathBuf::from(&lease.worktree_path)),
        lease.branch_name
    );
    assert!(repo.slot_lease_path(1).is_file());
    assert_eq!(board.leased_slots, 1);
    assert_eq!(board.idle_slots, DEFAULT_POOL_SIZE - 1);
}

#[test]
fn late_stream_events_from_released_generation_cannot_mutate_reallocated_slot() {
    let repo = TempGitRepo::new("late-stream-generation");
    let service = test_parallel_mode_service();
    let first = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("first generation should acquire");
    service
        .release_workspace_slot_lease_after_failed_start_for_lease(&first)
        .expect("first generation should release")
        .expect("first generation should be present");
    let second = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("second generation should acquire the reusable slot");
    assert_eq!(first.slot_id, second.slot_id);
    assert_ne!(first.lease_generation, second.lease_generation);
    assert_ne!(first.session_key(), second.session_key());

    assert!(
        service
            .record_workspace_slot_thread_prepared_for_lease(&first, "late-thread")
            .expect("late thread event should be ignored")
            .is_none()
    );
    let running_error = service
        .mark_workspace_slot_running_for_lease(&first)
        .expect_err("late TurnStarted must reject the replacement generation");
    assert!(running_error.contains("lease generation changed"));
    assert!(
        service
            .release_workspace_slot_lease_after_failed_start_for_lease(&first)
            .expect("late terminal event should be ignored")
            .is_none()
    );

    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("replacement generation should remain");
    assert_eq!(projection.slot_leases.get(&second.slot_id), Some(&second));
    assert_eq!(
        current_branch(&PathBuf::from(&second.worktree_path)),
        second.branch_name
    );
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

#[test]
fn slot_git_status_detects_staged_changes_and_relative_git_dir() {
    let repo = TempGitRepo::new("slot-status-staged");
    fs::write(repo.repo_root.join("staged.txt"), "staged change\n")
        .expect("staged file should write");
    run_git(&repo.repo_root, &["add", "staged.txt"]);

    let status =
        inspect_slot_git_status(&repo.repo_root).expect("git status should inspect repo root");
    assert_eq!(status.detail_label(), "staged changes");
    assert!(!status.is_clean_baseline());
    assert!(!status.has_pending_operation);
}

#[test]
fn slot_git_status_separates_ignored_output_from_frozen_source_changes() {
    let repo = TempGitRepo::new("slot-status-ignored-output");
    fs::write(repo.repo_root.join("build-cache.tmp"), "ignored output\n")
        .expect("ignored output should write");

    let status =
        inspect_slot_git_status(&repo.repo_root).expect("git status should inspect repo root");

    assert_eq!(status.detail_label(), "ignored files");
    assert!(!status.is_clean_baseline());
    assert!(status.is_clean_for_frozen_delivery());
}

#[cfg(unix)]
#[test]
fn slot_git_status_blocks_executable_local_filter_without_running_it() {
    let repo = TempGitRepo::new("slot-status-hostile-filter");
    let marker = repo.root.join("status-filter-executed");
    fs::write(
        repo.repo_root.join(".gitattributes"),
        "README.md filter=hostile\n",
    )
    .expect("filter attributes should write");
    fs::write(repo.repo_root.join("README.md"), "changed through filter\n")
        .expect("filtered path should change");
    run_git(
        &repo.repo_root,
        &[
            "config",
            "filter.hostile.clean",
            &format!("sh -c 'printf executed > {}'", marker.display()),
        ],
    );

    let error = inspect_slot_git_status(&repo.repo_root)
        .expect_err("status must fail before an executable local filter can run");

    assert!(
        error.to_string().contains("filter.hostile.clean"),
        "error: {error}"
    );
    assert!(!marker.exists(), "status executed the hostile clean filter");
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
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
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

// 표준 remote branch가 없으면 현재 workspace HEAD가 있더라도 readiness는 막아야 한다.
// integration target 생성은 운영자의 명시적 원격 작업이며 Akra가 암묵적으로 seed하지 않는다.
#[test]
fn inspect_readiness_blocks_missing_remote_integration_branch_without_seeding() {
    let repo = TempGitRepo::new("missing-origin-prerelease");
    repo.create_bare_origin_remote();
    repo.delete_remote_standard_tracking_branch();
    let service = test_parallel_mode_service();
    let snapshot = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    let capability = snapshot
        .capability(ParallelModeCapabilityKey::AkraBranch)
        .expect("akra branch capability should exist");

    assert_eq!(capability.state, ParallelModeCapabilityState::Blocked);
    assert!(
        capability
            .summary()
            .contains(&remote_standard_branch_name())
    );
    assert!(capability.summary().contains("explicitly"));
    assert!(!snapshot.allows_parallel_mode());
}

// configured push remote 자체가 없으면 required integration branch도 확인할 수 없으므로
// readiness는 같은 fail-closed 상태를 유지한다.
#[test]
fn inspect_readiness_blocks_missing_standard_branch_when_push_remote_is_absent() {
    let repo = TempGitRepo::new("missing-standard-no-push-remote");
    repo.delete_remote_standard_tracking_branch();
    run_git(
        &repo.repo_root,
        &["remote", "remove", DEFAULT_PUSH_REMOTE_NAME],
    );
    let service = test_parallel_mode_service();
    let snapshot = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    let capability = snapshot
        .capability(ParallelModeCapabilityKey::AkraBranch)
        .expect("akra branch capability should exist");

    assert_eq!(capability.state, ParallelModeCapabilityState::Blocked);
    assert!(capability.summary().contains("unavailable"));
    assert!(!snapshot.allows_parallel_mode());
}

// pool board의 reconciliation 세부 규칙은 missing/blocked/leased 상태 조합이 많아
// 별도 모듈로 분리한다. 이 파일은 dispatch와 root detection의 큰 흐름만 맡는다.
mod reconciliation;

// lease lifecycle은 acquisition, release, stale cleanup처럼 상태 전이가 길게 이어져
// 전용 모듈에서 slot store 계약을 더 촘촘히 검증한다.
mod lease;
