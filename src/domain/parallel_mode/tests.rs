use std::collections::BTreeMap;

use super::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeCapabilityKey,
    ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeLiveSessionDetailDefaults, ParallelModePoolSlotCleanupDecision,
    ParallelModePoolSlotState, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState, ParallelModeSupervisorState,
};

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

    let lease_selected = ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &[leased, running],
        &history,
        None,
        live_defaults(),
    )
    .expect("active lease should produce live detail");
    assert_eq!(lease_selected.slot_id, "slot-1");
    assert_eq!(lease_selected.state_label, "running");

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

fn live_defaults() -> ParallelModeLiveSessionDetailDefaults<'static> {
    ParallelModeLiveSessionDetailDefaults {
        validation_summary: "validation unavailable",
        authority_refresh_outcome: "authority unavailable",
    }
}
