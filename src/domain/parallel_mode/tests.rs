use std::collections::BTreeMap;

use super::orchestrator::ParallelModePostTurnQueueDecision;
use super::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAutomationTrigger,
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeDispatchBlockReason, ParallelModeDispatchCommandState, ParallelModeDispatchOutcome,
    ParallelModeDispatchTaskCandidate, ParallelModeLiveSessionDetailDefaults,
    ParallelModeOrchestratorState, ParallelModeOrchestratorStateMachine,
    ParallelModePoolResetPolicy, ParallelModePoolResetScope, ParallelModePoolSlotCleanupDecision,
    ParallelModePoolSlotState, ParallelModePostTurnQueueSignal, ParallelModeReadinessSnapshot,
    ParallelModeReadinessState, ParallelModeRuntimeEvent, ParallelModeRuntimeEventEntry,
    ParallelModeRuntimeEventsSnapshot, ParallelModeSlotLeaseRequest, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, ParallelModeSupervisorState,
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

// Operator-facing labels are shared by TUI badges, summaries, and diagnostics.
// Keep every public readiness/capability enum variant covered so new rows do
// not silently diverge from the compact vocabulary.
#[test]
fn readiness_and_capability_labels_cover_parallel_diagnostics_vocabulary() {
    assert_eq!(ParallelModeReadinessState::Repairing.label(), "repairing");
    assert!(!ParallelModeReadinessState::Repairing.allows_parallel_mode());

    assert_eq!(
        ParallelModeCapabilityKey::GitWorktree.label(),
        "git worktree"
    );
    assert_eq!(ParallelModeCapabilityKey::GhAuth.label(), "gh auth");
    assert_eq!(ParallelModeCapabilityKey::Planning.label(), "planning");
    assert_eq!(
        ParallelModeCapabilityKey::AuthorityStore.label(),
        "authority store"
    );

    assert_eq!(ParallelModeCapabilityState::Repairing.label(), "repairing");
}

#[test]
fn pool_slot_state_labels_cover_board_vocabulary() {
    assert_eq!(ParallelModePoolSlotState::Leased.label(), "leased");
    assert_eq!(
        ParallelModePoolSlotState::AwaitingCleanup.label(),
        "awaiting_cleanup"
    );
    assert_eq!(ParallelModePoolSlotState::Missing.label(), "missing");
}

#[test]
fn slot_lease_request_from_task_identity_normalizes_parallel_agent_slug() {
    let request = ParallelModeSlotLeaseRequest::from_task_identity(
        " task-supersession-runtime ",
        "Wire runtime into slot lease lifecycle",
    );

    assert_eq!(request.task_id, "task-supersession-runtime");
    assert_eq!(request.task_title, "Wire runtime into slot lease lifecycle");
    assert_eq!(request.agent_id, "agent-task-supersession-runtime");
    assert_eq!(request.task_slug, "task-supersession-runtime");
}

#[test]
fn slot_lease_request_from_task_identity_falls_back_to_task_title_slug() {
    let request =
        ParallelModeSlotLeaseRequest::from_task_identity(" !!! ", "Wire Runtime Into Slot Lease");

    assert_eq!(request.task_id, "!!!");
    assert_eq!(request.task_title, "Wire Runtime Into Slot Lease");
    assert_eq!(request.agent_id, "agent-wire-runtime-into-slot-lease");
    assert_eq!(request.task_slug, "wire-runtime-into-slot-lease");
}

#[test]
fn slot_lease_request_uses_default_slug_when_task_identity_has_no_words() {
    let request = ParallelModeSlotLeaseRequest::from_task_identity(" !!! ", " ??? ");

    assert_eq!(request.task_slug, "task");
    assert_eq!(request.agent_id, "agent-agent");
}

#[test]
fn slot_lease_request_slug_trims_separator_runs_from_task_id() {
    let request =
        ParallelModeSlotLeaseRequest::from_task_identity(" Task -- Needs Cleanup!!! ", "Fallback");

    assert_eq!(request.task_slug, "task-needs-cleanup");
    assert_eq!(request.agent_id, "agent-task-needs-cleanup");
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

#[test]
fn control_plane_entry_decision_selects_initial_reset_policy_in_domain() {
    let initial_entry =
        ParallelModeOrchestratorStateMachine::decide_parallel_entry(false, true, true);
    let guarded_reentry =
        ParallelModeOrchestratorStateMachine::decide_parallel_entry(false, true, false);
    let already_enabled =
        ParallelModeOrchestratorStateMachine::decide_parallel_entry(true, true, false);
    let readiness_blocked =
        ParallelModeOrchestratorStateMachine::decide_parallel_entry(false, false, true);

    assert_eq!(
        initial_entry.plan.state,
        ParallelModeOrchestratorState::PoolResetting
    );
    assert_eq!(
        initial_entry.plan.reset_scope,
        Some(ParallelModePoolResetScope::PoolOnly)
    );
    assert_eq!(
        initial_entry.reset_policy,
        Some(ParallelModePoolResetPolicy::ForceDisposable)
    );
    assert_eq!(
        guarded_reentry.reset_policy,
        Some(ParallelModePoolResetPolicy::ProtectLive)
    );
    assert_eq!(
        already_enabled.plan.state,
        ParallelModeOrchestratorState::Supervising
    );
    assert!(already_enabled.reset_policy.is_none());
    assert_eq!(
        readiness_blocked.plan.state,
        ParallelModeOrchestratorState::ReadinessBlocked
    );
    assert!(readiness_blocked.reset_policy.is_none());
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
        ParallelModeOrchestratorState::Supervising
    );
    assert_eq!(refresh_entry.reset_scope, None);
    assert_eq!(
        blocked_entry.state,
        ParallelModeOrchestratorState::ReadinessBlocked
    );
    assert_eq!(blocked_entry.reset_scope, None);
}

#[test]
fn dispatch_outcome_carries_trigger_and_structured_status_inputs() {
    let mut outcome = ParallelModeDispatchOutcome::new(
        ParallelModeAutomationTrigger::MainTurnPostEvaluation,
        "/repo",
        7,
    );
    outcome.idle_slot_count = 2;
    outcome.candidate_task_ids = vec!["task-1".to_string(), "task-2".to_string()];
    outcome.launched_task_ids = vec!["task-1".to_string()];

    assert_eq!(outcome.trigger.label(), "main_turn_post_evaluation");
    assert_eq!(outcome.workspace_directory, "/repo");
    assert_eq!(outcome.epoch_id, 7);
    assert_eq!(outcome.status_detail(), "auto dispatched 1 worker(s)");

    outcome.blocked_reason = Some("no idle slot is available for auto dispatch".to_string());
    outcome.status_copy_input.clear();
    assert_eq!(
        outcome.status_detail(),
        "auto dispatch blocked / no idle slot is available for auto dispatch"
    );
}

#[test]
fn post_turn_queue_continuation_dispatches_from_all_parallel_queue_signals() {
    let auto_follow = ParallelModeOrchestratorStateMachine::post_turn_queue_continuation(
        true,
        Some(ParallelModePostTurnQueueSignal::AutoFollowQueued),
        false,
    );
    let parallel_completion = ParallelModeOrchestratorStateMachine::post_turn_queue_continuation(
        true,
        Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized),
        true,
    );

    assert_eq!(
        auto_follow,
        ParallelModePostTurnQueueDecision::Dispatch {
            trigger: ParallelModeAutomationTrigger::MainTurnPostEvaluation,
            consume_auto_follow_prompt: true
        }
    );
    assert_eq!(
        parallel_completion.dispatch_trigger(),
        Some(ParallelModeAutomationTrigger::ParallelOfficialCompletion)
    );
    assert!(!parallel_completion.should_consume_auto_follow_prompt());
}

#[test]
fn post_turn_queue_continuation_ignores_parallel_completion_without_ready_head() {
    let decision = ParallelModeOrchestratorStateMachine::post_turn_queue_continuation(
        true,
        Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized),
        false,
    );
    let disabled = ParallelModeOrchestratorStateMachine::post_turn_queue_continuation(
        false,
        Some(ParallelModePostTurnQueueSignal::AutoFollowQueued),
        true,
    );

    assert_eq!(decision, ParallelModePostTurnQueueDecision::NoDispatch);
    assert_eq!(disabled, ParallelModePostTurnQueueDecision::NoDispatch);
}

#[test]
fn runtime_dispatch_commands_are_emitted_from_central_parallel_events() {
    let commands = ParallelModeOrchestratorStateMachine::runtime_dispatch_commands(
        true,
        ParallelModeRuntimeEvent::ParallelCompletionFinalized,
        true,
        Some("queue-head-42".to_string()),
        Some(7),
        "2026-05-08T00:00:00+00:00",
    );

    assert_eq!(commands.len(), 1);
    let command = &commands[0];
    assert_eq!(command.command_id, "dispatch-ready-queue-queue-head-42");
    assert_eq!(
        command.trigger,
        ParallelModeAutomationTrigger::ParallelOfficialCompletion
    );
    assert_eq!(command.state, ParallelModeDispatchCommandState::Pending);
    assert_eq!(command.epoch_id, Some(7));
}

#[test]
fn runtime_dispatch_commands_require_mode_and_actionable_head() {
    let disabled = ParallelModeOrchestratorStateMachine::runtime_dispatch_commands(
        false,
        ParallelModeRuntimeEvent::TaskIntakeCommitted,
        true,
        Some("queue-head-42".to_string()),
        None,
        "2026-05-08T00:00:00+00:00",
    );
    let missing_head = ParallelModeOrchestratorStateMachine::runtime_dispatch_commands(
        true,
        ParallelModeRuntimeEvent::ParallelCompletionFinalized,
        false,
        None,
        None,
        "2026-05-08T00:00:00+00:00",
    );

    assert!(disabled.is_empty());
    assert!(missing_head.is_empty());
}

#[test]
fn runtime_events_snapshot_reports_latest_visible_event() {
    let snapshot = ParallelModeRuntimeEventsSnapshot::new(
        vec![ParallelModeRuntimeEventEntry::new(
            7,
            "slot_lease_upsert",
            "slot_lease",
            "slot-1",
            3,
            "runtime slot lease stored / slot: slot-1 / state: running",
            "2026-05-04T10:00:00+00:00",
        )],
        4,
        "no runtime events captured yet",
    );

    assert_eq!(snapshot.visible_count(), 1);
    assert_eq!(
        snapshot.compact_summary(),
        "events 1/4 / latest #7 slot_lease_upsert slot_lease:slot-1"
    );
}

#[test]
fn runtime_events_snapshot_empty_summary_uses_empty_state_copy() {
    let snapshot = ParallelModeRuntimeEventsSnapshot::empty("no runtime events captured yet");

    assert_eq!(snapshot.visible_count(), 0);
    assert_eq!(snapshot.total_event_count, 0);
    assert_eq!(snapshot.latest(), None);
    assert_eq!(
        snapshot.compact_summary(),
        "no runtime events captured yet".to_string()
    );
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

// dispatch 후보 선정은 capacity, 이미 처리 중인 task, startup 실패 차단을 함께 판단한다.
// capacity는 exclusion 뒤에 적용되어야 앞쪽 task가 제외되어도 뒤쪽 ready task가 idle slot을 채운다.
#[test]
fn orchestrator_dispatch_selection_applies_capacity_after_exclusion() {
    let selection = ParallelModeOrchestratorStateMachine::select_dispatch_candidates(
        2,
        usize::MAX,
        vec!["task-1".to_string()],
        &BTreeMap::from([("task-2".to_string(), 20)]),
        vec![
            ParallelModeDispatchTaskCandidate::new("task-1", Some(10)),
            ParallelModeDispatchTaskCandidate::new("task-2", Some(10)),
            ParallelModeDispatchTaskCandidate::new("task-3", Some(10)),
            ParallelModeDispatchTaskCandidate::new("task-4", Some(10)),
        ],
    );

    assert_eq!(selection.dispatch_capacity, 2);
    assert_eq!(selection.excluded_task_ids, vec!["task-1", "task-2"]);
    assert_eq!(selection.selected_task_ids, vec!["task-3", "task-4"]);
}

#[test]
fn orchestrator_dispatch_selection_reopens_failed_start_task_after_task_update() {
    let selection = ParallelModeOrchestratorStateMachine::select_dispatch_candidates(
        1,
        usize::MAX,
        Vec::new(),
        &BTreeMap::from([("task-1".to_string(), 10)]),
        vec![ParallelModeDispatchTaskCandidate::new("task-1", Some(20))],
    );

    assert!(selection.excluded_task_ids.is_empty());
    assert_eq!(selection.selected_task_ids, vec!["task-1"]);
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
    assert_eq!(
        cleanup.runtime_state_override(&session_detail(&cleanup, "failed", "cleanup failed")),
        Some("failed")
    );
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

#[test]
fn runtime_detail_selection_keeps_active_queue_history_and_starting_detail_paths() {
    let leased = lease(
        "slot-2",
        "task-2",
        "Task Two",
        "agent-2",
        ParallelModeSlotLeaseState::Leased,
        "2026-01-01T00:10:00Z",
        None,
    );
    let history_detail = session_detail(
        &leased,
        "assigned",
        "agent thread was prepared for execution",
    );

    let live_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        std::slice::from_ref(&leased),
        std::slice::from_ref(&history_detail),
        Some(leased.session_key().as_str()),
        live_defaults(),
    )
    .expect("active queue live lease should be selected");
    assert_eq!(live_selected.state_label, "starting");
    assert_eq!(
        live_selected.latest_summary,
        "agent thread was prepared for execution"
    );

    let history_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &[],
        std::slice::from_ref(&history_detail),
        Some(leased.session_key().as_str()),
        live_defaults(),
    )
    .expect("active queue history detail should be selected without a live lease");
    assert_eq!(history_selected.session_key, leased.session_key());
}

#[test]
fn running_roster_entry_uses_runtime_summary_fallback_when_detail_is_absent() {
    let running = lease(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        ParallelModeSlotLeaseState::Running,
        "2026-01-01T00:00:00Z",
        Some("2026-01-01T00:05:00Z"),
    );

    let roster = super::ParallelModeAgentRosterSnapshot::project_from_leases(
        vec![running],
        &[],
        true,
        &BTreeMap::new(),
    );

    assert_eq!(roster.entries[0].state_label, "running");
    assert_eq!(
        roster.entries[0].latest_summary,
        "agent session is active in the leased slot"
    );
}

#[test]
fn agent_roster_duration_labels_cover_pipeline_state_vocabulary() {
    let states = [
        ("reported_complete", "reported"),
        ("ledger_refreshing", "refreshing"),
        ("pushing", "pushing"),
        ("pr_pending", "pr pending"),
        ("merge_pending", "merge pending"),
        ("integrating", "integrating"),
    ];

    for (state_label, expected_duration) in states {
        let running = lease(
            "slot-1",
            "task-1",
            "Task One",
            "agent-1",
            ParallelModeSlotLeaseState::Running,
            "2026-01-01T00:00:00Z",
            Some("2026-01-01T00:05:00Z"),
        );
        let detail = session_detail(&running, state_label, "pipeline state projected");

        let roster = super::ParallelModeAgentRosterSnapshot::project_from_leases(
            vec![running],
            std::slice::from_ref(&detail),
            true,
            &BTreeMap::new(),
        );

        assert_eq!(roster.entries[0].duration_label, expected_duration);
    }
}

#[test]
fn runtime_detail_selection_uses_slot_id_tie_breaker_for_equal_priority_leases() {
    let slot_one = lease(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        ParallelModeSlotLeaseState::Running,
        "2026-01-01T00:00:00Z",
        Some("2026-01-01T00:05:00Z"),
    );
    let slot_two = lease(
        "slot-2",
        "task-2",
        "Task Two",
        "agent-2",
        ParallelModeSlotLeaseState::Running,
        "2026-01-01T00:00:00Z",
        Some("2026-01-01T00:05:00Z"),
    );

    let selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &[slot_one, slot_two],
        &[],
        None,
        live_defaults(),
    )
    .expect("equal-priority leases should still select a stable detail");

    assert_eq!(selected.slot_id, "slot-1");
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
    assert!(!ParallelModePoolSlotCleanupDecision::new(None, true, false).is_cleanup_ready());
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

#[test]
fn pool_board_compact_summary_lists_all_non_idle_pressure_states() {
    let snapshot = super::ParallelModePoolBoardSnapshot::new(
        7,
        "/repo/.akra-worktrees",
        "ready",
        vec![
            super::ParallelModePoolSlotSnapshot::new(
                "slot-1",
                ParallelModePoolSlotState::Idle,
                "prerelease",
                "slot-1",
                "idle",
            ),
            super::ParallelModePoolSlotSnapshot::new(
                "slot-2",
                ParallelModePoolSlotState::Leased,
                "akra-agent/slot-2/task-2",
                "slot-2",
                "agent-2 / task-2",
            ),
            super::ParallelModePoolSlotSnapshot::new(
                "slot-3",
                ParallelModePoolSlotState::Running,
                "akra-agent/slot-3/task-3",
                "slot-3",
                "agent-3 / task-3",
            ),
            super::ParallelModePoolSlotSnapshot::new(
                "slot-4",
                ParallelModePoolSlotState::AwaitingCleanup,
                "akra-agent/slot-4/task-4",
                "slot-4",
                "agent-4 / task-4",
            ),
            super::ParallelModePoolSlotSnapshot::new(
                "slot-5",
                ParallelModePoolSlotState::Blocked,
                "akra-agent/slot-5/task-5",
                "slot-5",
                "blocked",
            ),
            super::ParallelModePoolSlotSnapshot::new(
                "slot-6",
                ParallelModePoolSlotState::Missing,
                "akra-agent/slot-6/task-6",
                "slot-6",
                "missing",
            ),
            super::ParallelModePoolSlotSnapshot::new(
                "slot-7",
                ParallelModePoolSlotState::Unavailable,
                "akra-agent/slot-7/task-7",
                "slot-7",
                "unavailable",
            ),
        ],
    );

    assert_eq!(
        snapshot.compact_summary(),
        "idle 1/7 / leased 1 / running 1 / cleanup 1 / blocked 1 / missing 1 / unavailable 1"
    );
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
