use std::collections::BTreeMap;

use super::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeCapabilityKey,
    ParallelModeCapabilitySnapshot, ParallelModeCapabilityState, ParallelModeDispatchBlockReason,
    ParallelModeLiveSessionDetailDefaults, ParallelModeOrchestratorState,
    ParallelModeOrchestratorStateMachine, ParallelModePoolResetScope,
    ParallelModePoolSlotCleanupDecision, ParallelModePoolSlotState, ParallelModeReadinessSnapshot,
    ParallelModeReadinessState, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    ParallelModeSupervisorState,
};

// readiness 집계의 최우선 안전 규칙을 고정한다. 하나라도 Blocked가 있으면 다른
// capability가 Ready여도 병렬 실행을 막아 supervisor가 Recover 경로로 갈 수 있어야 한다.
#[test]
fn readiness_derivation_marks_blocked_when_any_blocker_exists() {
    let readiness = ParallelModeReadinessState::derive_from_capabilities(&[
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitRepository,
            ParallelModeCapabilityState::Ready,
            "ready",
            None,
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Blocked,
            "planning invalid",
            Some("repair planning".to_string()),
        ),
    ]);

    assert_eq!(readiness, ParallelModeReadinessState::Blocked);
}

// PushRemote는 병렬 모드의 품질을 낮추지만 즉시 중단할 필수 blocker는 아니다.
// 이 테스트는 선택 capability 실패가 전체 상태를 Degraded로만 끌어내리는 계약을 지킨다.
#[test]
fn readiness_derivation_marks_degraded_when_only_optional_capabilities_fail() {
    let readiness = ParallelModeReadinessState::derive_from_capabilities(&[
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitRepository,
            ParallelModeCapabilityState::Ready,
            "ready",
            None,
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Degraded,
            "push unavailable",
            Some("restore auth".to_string()),
        ),
    ]);

    assert_eq!(readiness, ParallelModeReadinessState::Degraded);
}

// Repairing은 곧 Ready가 될 가능성이 있는 중간 상태다. Blocked와 달리 복구 대기
// 신호인 Degraded에 머물러 사용자가 진행 가능성과 위험을 구분하게 한다.
#[test]
fn readiness_derivation_marks_degraded_when_capability_is_repairing() {
    let readiness = ParallelModeReadinessState::derive_from_capabilities(&[
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitRepository,
            ParallelModeCapabilityState::Ready,
            "ready",
            None,
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Repairing,
            "repair in progress",
            Some("wait for repair".to_string()),
        ),
    ]);

    assert_eq!(readiness, ParallelModeReadinessState::Degraded);
}

// 모든 필수 capability가 Ready이면 병렬 모드 gate가 열린다. 이 baseline이 있어
// 새 capability를 추가할 때 Ready 경로를 불필요하게 막는 회귀를 바로 잡는다.
#[test]
fn readiness_derivation_marks_ready_when_all_capabilities_are_ready() {
    let readiness = ParallelModeReadinessState::derive_from_capabilities(&[
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitRepository,
            ParallelModeCapabilityState::Ready,
            "ready",
            None,
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Ready,
            "ready",
            None,
        ),
    ]);

    assert_eq!(readiness, ParallelModeReadinessState::Ready);
}

// supervisor 상태는 mode toggle과 readiness gate를 함께 본다. 같은 Blocked
// snapshot이어도 사용자가 병렬 모드를 꺼두면 Recover가 아니라 Prepare에 남아야 한다.
#[test]
fn supervisor_state_recovers_when_enabled_readiness_blocks_parallel_mode() {
    let readiness = ParallelModeReadinessSnapshot::new(
        "/repo",
        ParallelModeReadinessState::Blocked,
        Vec::new(),
        None,
    );

    assert_eq!(
        ParallelModeSupervisorState::derive(true, Some(&readiness)),
        ParallelModeSupervisorState::Recover
    );
    assert_eq!(
        ParallelModeSupervisorState::derive(false, Some(&readiness)),
        ParallelModeSupervisorState::Prepare
    );
}

// parallel entry state machine은 `:parallel`을 off -> on으로 진입할 때만 pool reset을
// 요청한다. reset scope는 pool-only로 고정되어 planning task authority를 건드리지 않는다.
#[test]
fn orchestrator_entry_plan_resets_pool_only_on_off_to_on_entry() {
    let first_entry = ParallelModeOrchestratorStateMachine::plan_parallel_entry(false, true);
    let refresh_entry = ParallelModeOrchestratorStateMachine::plan_parallel_entry(true, true);
    let blocked_entry = ParallelModeOrchestratorStateMachine::plan_parallel_entry(false, false);

    assert_eq!(
        first_entry.state,
        ParallelModeOrchestratorState::PoolResetting
    );
    assert_eq!(
        first_entry.reset_scope,
        Some(ParallelModePoolResetScope::PoolOnly)
    );
    assert_eq!(
        refresh_entry.state,
        ParallelModeOrchestratorState::Dispatching
    );
    assert_eq!(refresh_entry.reset_scope, None);
    assert_eq!(
        blocked_entry.state,
        ParallelModeOrchestratorState::ReadinessBlocked
    );
    assert_eq!(blocked_entry.reset_scope, None);
}

// dispatch eligibility도 같은 state machine이 판단한다. runtime이 이미 소유한 task는
// 중복 실행하지 않고, startup 실패 후 task가 갱신되기 전까지는 같은 실패를 반복하지 않는다.
#[test]
fn orchestrator_dispatch_eligibility_blocks_runtime_and_stale_failed_start_tasks() {
    let runtime_owned =
        ParallelModeOrchestratorStateMachine::dispatch_eligibility(true, None, Some(10));
    let stale_failed_start =
        ParallelModeOrchestratorStateMachine::dispatch_eligibility(false, Some(20), Some(10));
    let changed_after_failure =
        ParallelModeOrchestratorStateMachine::dispatch_eligibility(false, Some(10), Some(20));

    assert_eq!(
        runtime_owned.block_reason,
        Some(ParallelModeDispatchBlockReason::RuntimeAlreadyOwnsTask)
    );
    assert_eq!(
        stale_failed_start.block_reason,
        Some(ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges)
    );
    assert!(changed_after_failure.is_dispatchable());
}

// roster projection은 lease 생명주기와 runtime detail을 합쳐 TUI 목록을 만든다.
// Running lease의 후속 pipeline label은 runtime detail이 우선하고, Leased/Cleanup은
// 도메인 기본 label과 duration fallback을 유지하는 정렬 규칙을 검증한다.
#[test]
fn roster_projection_sorts_active_leases_and_applies_runtime_detail_overrides() {
    let running = lease(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        ParallelModeSlotLeaseState::Running,
        "2026-01-01T00:00:00Z",
        Some("2026-01-01T00:05:00Z"),
    );
    let leased = lease(
        "slot-2",
        "task-2",
        "Task Two",
        "agent-2",
        ParallelModeSlotLeaseState::Leased,
        "2026-01-01T00:10:00Z",
        None,
    );
    let cleanup = lease(
        "slot-3",
        "task-3",
        "Task Three",
        "agent-3",
        ParallelModeSlotLeaseState::CleanupPending,
        "2026-01-01T00:20:00Z",
        Some("2026-01-01T00:25:00Z"),
    );
    let detail = session_detail(
        &running,
        "commit_ready",
        "official ledger refresh accepted the completion report",
    );
    let duration_labels = BTreeMap::from([(running.session_key(), "7m".to_string())]);

    // 입력 순서를 cleanup, leased, running으로 흔들어 selection_priority가 실제
    // 화면 순서를 결정한다는 점을 드러낸다.
    let roster = super::ParallelModeAgentRosterSnapshot::project_from_leases(
        vec![cleanup, leased, running],
        &[detail],
        true,
        &duration_labels,
    );

    assert_eq!(roster.active_count(), 3);
    assert_eq!(
        roster.empty_state,
        "no agent sessions launched in this slice"
    );
    assert_eq!(roster.entries[0].slot_id, "slot-1");
    assert_eq!(roster.entries[0].state_label, "commit_ready");
    assert_eq!(roster.entries[0].duration_label, "official");
    assert_eq!(
        roster.entries[0].latest_summary,
        "official ledger refresh accepted the completion report"
    );
    assert_eq!(roster.entries[1].slot_id, "slot-2");
    assert_eq!(roster.entries[1].state_label, "starting");
    assert_eq!(roster.entries[1].duration_label, "launch pending");
    assert_eq!(roster.entries[2].slot_id, "slot-3");
    assert_eq!(roster.entries[2].state_label, "cleanup_pending");
    assert_eq!(roster.entries[2].duration_label, "complete");
}

// failed detail은 operator가 원인과 slot 상태를 볼 수 있도록 roster row에는 남긴다.
// 다만 더 이상 실행 중인 worker가 아니므로 header의 active count와 live pulse에서는 제외되어야 한다.
#[test]
fn roster_active_count_excludes_failed_runtime_detail_rows() {
    let failed_running = lease(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        ParallelModeSlotLeaseState::Running,
        "2026-01-01T00:00:00Z",
        Some("2026-01-01T00:05:00Z"),
    );
    let detail = session_detail(
        &failed_running,
        "failed",
        "official completion refresh failed",
    );

    let roster = super::ParallelModeAgentRosterSnapshot::project_from_leases(
        vec![failed_running],
        &[detail],
        true,
        &BTreeMap::new(),
    );

    assert_eq!(roster.entries.len(), 1);
    assert_eq!(roster.active_count(), 0);
    assert_eq!(roster.compact_summary(), "0 active");
    assert_eq!(roster.entries[0].state_label, "failed");
    assert_eq!(roster.entries[0].duration_label, "blocked");
    assert_eq!(
        roster.entries[0].latest_summary,
        "official completion refresh failed"
    );
}

// stale ledger refresh recovery는 실제 작업 실패가 아니지만, 더 이상 live worker도 아니다.
// row는 남기고 active count에서는 제외해 로딩이 다시 살아나지 않게 한다.
#[test]
fn roster_active_count_excludes_official_refresh_recovery_rows() {
    let stale_running = lease(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        ParallelModeSlotLeaseState::Running,
        "2026-01-01T00:00:00Z",
        Some("2026-01-01T00:05:00Z"),
    );
    let detail = session_detail(
        &stale_running,
        "official_refresh_recovery_needed",
        "official completion refresh needs recovery",
    );

    let roster = super::ParallelModeAgentRosterSnapshot::project_from_leases(
        vec![stale_running],
        &[detail],
        true,
        &BTreeMap::new(),
    );

    assert_eq!(roster.entries.len(), 1);
    assert_eq!(roster.active_count(), 0);
    assert_eq!(
        roster.entries[0].state_label,
        "official_refresh_recovery_needed"
    );
    assert_eq!(roster.entries[0].duration_label, "recovery needed");
}

// live detail은 저장된 agent history가 비어 있거나 일부 필드를 잃어도 lease에서
// 화면에 필요한 최소 runtime 정보를 복원한다. CleanupPending은 완료된 branch가
// slot cleanup만 기다린다는 distributor 설명까지 채워야 한다.
#[test]
fn live_detail_enrichment_fills_missing_runtime_fields_from_lease() {
    let cleanup = lease(
        "slot-3",
        "task-3",
        "Task Three",
        "agent-3",
        ParallelModeSlotLeaseState::CleanupPending,
        "2026-01-01T00:20:00Z",
        Some("2026-01-01T00:25:00Z"),
    );
    let mut detail = session_detail(&cleanup, "running", "");
    // 실제 ledger나 세션 기록이 부분적으로 비어 들어오는 상황을 만든다.
    // fallback이 lease와 defaults에서 채워지는지 확인하려는 의도다.
    detail.validation_summary.clear();
    detail.authority_refresh_outcome.clear();
    detail.updated_at.clear();

    let enriched = ParallelModeAgentSessionDetailSnapshot::live_for_lease(
        &cleanup,
        Some(detail),
        live_defaults(),
    );

    assert_eq!(enriched.session_key, cleanup.session_key());
    assert_eq!(enriched.state_label, "cleanup_pending");
    assert_eq!(enriched.completion_state_label, "merged");
    assert_eq!(
        enriched.latest_summary,
        "agent session reported completion and slot cleanup is pending"
    );
    assert_eq!(enriched.validation_summary, "validation unavailable");
    assert_eq!(enriched.authority_refresh_outcome, "authority unavailable");
    assert_eq!(
        enriched.distributor_outcome.as_deref(),
        Some("branch is merged into prerelease and the slot is awaiting cleanup")
    );
    assert_eq!(enriched.updated_at, "2026-01-01T00:25:00Z");
}

// runtime detail 선택은 현재 queue head가 가장 강한 신호다. queue head가 없으면
// active lease, 둘 다 없으면 history로 내려가 supervisor detail panel이 비지 않게 한다.
#[test]
fn runtime_detail_selection_prefers_active_queue_head_then_active_lease_then_history() {
    let running = lease(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        ParallelModeSlotLeaseState::Running,
        "2026-01-01T00:00:00Z",
        Some("2026-01-01T00:05:00Z"),
    );
    let leased = lease(
        "slot-2",
        "task-2",
        "Task Two",
        "agent-2",
        ParallelModeSlotLeaseState::Leased,
        "2026-01-01T00:10:00Z",
        None,
    );
    let history = vec![session_detail(
        &running,
        "running",
        "agent session entered the running state",
    )];

    let queue_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &[running.clone(), leased.clone()],
        &history,
        Some(leased.session_key().as_str()),
        live_defaults(),
    )
    .expect("active queue lease should produce live detail");
    assert_eq!(queue_selected.slot_id, "slot-2");
    assert_eq!(queue_selected.state_label, "assigned");

    // queue head가 없을 때는 roster 정렬에서 가장 우선인 active lease가 detail 주인이 된다.
    let lease_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &[leased, running],
        &history,
        None,
        live_defaults(),
    )
    .expect("active lease should produce live detail");
    assert_eq!(lease_selected.slot_id, "slot-1");
    assert_eq!(lease_selected.state_label, "running");

    // active lease가 전혀 없어도 마지막 history가 있으면 supervisor detail은
    // 최근 완료/실패 맥락을 계속 보여준다.
    let history_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &[],
        &history,
        None,
        live_defaults(),
    )
    .expect("history fallback should be selected");
    assert_eq!(history_selected.slot_id, "slot-1");
}

// cleanup decision은 branch 통합 여부를 slot 반환의 마지막 gate로 삼는다.
// Running lease는 worktree가 clean이어도 건드리지 않고, lease가 사라진 잔여물은
// clean+integrated가 모두 참일 때만 자동 청소 후보가 된다.
#[test]
fn pool_slot_cleanup_decision_respects_lease_state_and_branch_integration() {
    assert!(
        ParallelModePoolSlotCleanupDecision::new(
            Some(ParallelModeSlotLeaseState::CleanupPending),
            false,
            true
        )
        .is_cleanup_ready()
    );
    assert!(
        !ParallelModePoolSlotCleanupDecision::new(
            Some(ParallelModeSlotLeaseState::Running),
            true,
            true
        )
        .is_cleanup_ready()
    );
    assert!(ParallelModePoolSlotCleanupDecision::new(None, true, true).is_cleanup_ready());
    assert!(!ParallelModePoolSlotCleanupDecision::new(None, false, true).is_cleanup_ready());
}

// board row projection은 lease 생명주기를 TUI slot 상태로 바꾸는 단일 경계다.
// CleanupPending lease는 AwaitingCleanup slot으로 보이고 owner label은 agent/task 조합이다.
#[test]
fn pool_slot_snapshot_projects_lease_state_to_pool_slot_state() {
    let lease = lease(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        ParallelModeSlotLeaseState::CleanupPending,
        "2026-01-01T00:00:00Z",
        Some("2026-01-01T00:05:00Z"),
    );

    let slot = super::ParallelModePoolSlotSnapshot::from_lease(
        "slot-1",
        lease.branch_name.as_str(),
        "slot-1 / clean",
        &lease,
    );

    assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
    assert_eq!(slot.owner_label, "agent-1 / task-1");
}

// 테스트 fixture lease는 실제 pool allocation이 만드는 branch/worktree naming을 축약한다.
// session_key, owner label, runtime fallback이 같은 데이터 모양을 전제로 동작한다.
fn lease(
    slot_id: &str,
    task_id: &str,
    task_title: &str,
    agent_id: &str,
    state: ParallelModeSlotLeaseState,
    leased_at: &str,
    running_started_at: Option<&str>,
) -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
        slot_id,
        task_id,
        task_title,
        agent_id,
        format!("akra-agent/{slot_id}/{task_id}"),
        format!("/repo/.akra-worktrees/{slot_id}"),
        state,
        leased_at,
        running_started_at.map(str::to_string),
    )
}

// agent session detail fixture는 lease에서 온 ownership 필드와 runtime pipeline 필드를
// 한 번에 채운다. 각 테스트는 필요한 필드만 덮어써 projection fallback을 분리해서 검증한다.
fn session_detail(
    lease: &ParallelModeSlotLeaseSnapshot,
    state_label: &str,
    latest_summary: &str,
) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::new(
        lease.session_key(),
        lease.agent_id.clone(),
        lease.task_id.clone(),
        lease.task_title.clone(),
        lease.slot_id.clone(),
        Some("thread-1".to_string()),
        lease.worktree_path.clone(),
        lease.branch_name.clone(),
        lease.leased_at.clone(),
        state_label,
        state_label,
        latest_summary,
        "cargo test passed",
        "authority refreshed",
        None,
        Vec::new(),
        "2026-01-01T00:30:00Z",
    )
}

// live default fixture는 runtime detail에 빈 문자열이 들어온 경우 domain fallback으로
// 들어갈 문구를 고정한다. 이 값이 바뀌면 TUI의 unavailable 표현도 같이 바뀐다.
fn live_defaults() -> ParallelModeLiveSessionDetailDefaults<'static> {
    ParallelModeLiveSessionDetailDefaults {
        validation_summary: "validation unavailable",
        authority_refresh_outcome: "authority unavailable",
    }
}
