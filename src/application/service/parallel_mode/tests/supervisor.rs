use super::*;

// parallel mode가 꺼져 있으면 supervisor는 pool을 실제로 운용하지 않고 준비 화면의
// 기본 숫자만 보여준다. 이 상태에서 distributor head가 inactive인 것도 TUI가
// 사용자에게 "아직 시작 전"으로 표시하기 위한 계약이다.
#[test]
fn build_supervisor_snapshot_marks_prepare_when_mode_is_off() {
    let service = test_parallel_mode_service();
    let snapshot = service.build_supervisor_snapshot("/tmp/root", false, None);

    assert_eq!(snapshot.state, ParallelModeSupervisorState::Prepare);
    assert_eq!(snapshot.pool.configured_size, DEFAULT_POOL_SIZE);
    assert_eq!(snapshot.roster.active_count(), 0);
    assert_eq!(snapshot.distributor.head_summary, "inactive");
}

// mode flag는 켜졌지만 readiness가 blocked이면 supervisor는 work를 배정하지 않고
// recovery 화면으로 전환한다. unavailable pool과 paused distributor를 함께
// 검증해 준비 실패가 dispatch로 새지 않도록 막는다.
#[test]
fn build_supervisor_snapshot_uses_recover_when_mode_enabled_but_blocked() {
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        "/tmp/root",
        ParallelModeReadinessState::Blocked,
        vec![],
        Some("planning: blocked".to_string()),
    );
    let snapshot = service.build_supervisor_snapshot("/tmp/root", true, Some(&readiness));

    assert_eq!(snapshot.state, ParallelModeSupervisorState::Recover);
    assert_eq!(snapshot.pool.unavailable_slots, DEFAULT_POOL_SIZE);
    assert_eq!(snapshot.distributor.head_summary, "paused");
}

// 새 lease 직후의 roster는 agent가 아직 실행되기 전인 starting 상태를 나타낸다.
// branch reservation과 launch pending 문구가 함께 보존되어야 TUI가 할당은 되었지만
// session이 아직 붙지 않은 slot을 구분할 수 있다.
#[test]
fn build_supervisor_snapshot_populates_roster_from_live_slot_leases() {
    let repo = TempGitRepo::new("supervisor-roster-starting");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let entry = snapshot
        .roster
        .entries
        .first()
        .expect("roster should contain the leased agent");

    assert_eq!(snapshot.roster.active_count(), 1);
    assert_eq!(entry.agent_id, "agent-1");
    assert_eq!(entry.task_title, "Task One");
    assert_eq!(entry.slot_id, "slot-1");
    assert_eq!(entry.branch_name, lease.branch_name);
    assert_eq!(entry.state_label, "starting");
    assert_eq!(entry.duration_label, "launch pending");
    assert_eq!(
        entry.latest_summary,
        "slot lease acquired and branch reserved for launch"
    );
}

// mirror JSON 파일은 캐시일 뿐이고 authoritative state는 sqlite store에 남아야 한다.
// lease mirror와 session detail mirror를 지운 뒤 새 service가 같은 running slot과
// detail selection을 복구하는지 확인한다.
#[test]
fn build_supervisor_snapshot_reads_store_backed_runtime_projections_after_mirror_loss() {
    let repo = TempGitRepo::new("supervisor-store-recovery");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should transition to running");
    let session_key = lease_session_key(&lease);
    fs::remove_file(repo.slot_lease_path(1)).expect("slot lease mirror should be removed");
    fs::remove_file(repo.session_detail_path(&session_key))
        .expect("session detail mirror should be removed");
    let recovered = test_parallel_mode_service();
    let snapshot =
        recovered.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert_eq!(snapshot.pool.running_slots, 1);
    assert_eq!(snapshot.roster.active_count(), 1);
    assert_eq!(snapshot.roster.entries[0].state_label, "running");
    assert_eq!(
        snapshot
            .detail
            .session
            .as_ref()
            .expect("session detail should be recovered from the authority store")
            .session_key,
        session_key
    );
}

// roster projection은 같은 lease가 running에서 cleanup_pending으로 이동할 때
// duration과 summary 의미를 바꿔야 한다. 이 테스트는 agent 실행 중 표시와 merge
// 완료 후 cleanup 대기 표시가 서로 섞이지 않도록 상태별 텍스트를 고정한다.
#[test]
fn build_supervisor_snapshot_projects_running_and_cleanup_pending_roster_states() {
    let repo = TempGitRepo::new("supervisor-roster-lifecycle");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());

    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    let running_snapshot =
        service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let running_entry = running_snapshot
        .roster
        .entries
        .first()
        .expect("running roster entry should exist");
    assert_eq!(running_entry.state_label, "running");
    assert_ne!(running_entry.duration_label, "launch pending");
    assert_eq!(
        running_entry.latest_summary,
        "agent session entered the running state"
    );

    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    let cleanup_snapshot =
        service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let cleanup_entry = cleanup_snapshot
        .roster
        .entries
        .first()
        .expect("cleanup-pending roster entry should exist");
    assert_eq!(cleanup_entry.state_label, "cleanup_pending");
    assert_eq!(cleanup_entry.duration_label, "complete");
    assert_eq!(
        cleanup_entry.latest_summary,
        "agent branch is merged into prerelease and awaiting slot cleanup"
    );
}

// live session detail은 roster보다 더 깊은 timeline을 보여준다. thread prepared와
// running 전이를 거친 뒤 선택된 detail이 agent/task/thread identity와 history를
// 잃지 않는지 검증한다.
#[test]
fn build_supervisor_snapshot_populates_detail_with_live_session_history() {
    let repo = TempGitRepo::new("supervisor-detail-live");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-42")
        .expect("thread prepared should be recorded");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("detail should select the live agent session");

    assert_eq!(detail.agent_id, "agent-1");
    assert_eq!(detail.task_id, "task-1");
    assert_eq!(detail.thread_id.as_deref(), Some("thread-42"));
    assert_eq!(detail.state_label, "running");
    assert_eq!(detail.completion_state_label, "in_progress");
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec!["assigned", "starting", "running"]
    );
}

// slot cleanup 이후 roster에는 active entry가 없어도 detail pane은 마지막 session을
// 잃으면 안 된다. 사용자가 방금 반환된 slot의 결과를 확인할 수 있도록 cleaned
// state, distributor outcome, 전체 history가 남는지 고정한다.
#[test]
fn build_supervisor_snapshot_keeps_cleaned_session_detail_after_slot_return() {
    let repo = TempGitRepo::new("supervisor-detail-cleaned");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());

    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-77")
        .expect("thread prepared should be recorded");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    service
        .cleanup_workspace_slot_if_pending(&lease.worktree_path)
        .expect("cleanup should succeed")
        .expect("cleanup should return the cleaned lease");
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("detail should keep the last cleaned session");

    assert_eq!(snapshot.roster.active_count(), 0);
    assert_eq!(detail.thread_id.as_deref(), Some("thread-77"));
    assert_eq!(detail.state_label, "cleaned");
    assert_eq!(detail.completion_state_label, "cleaned");
    assert_eq!(
        detail.distributor_outcome.as_deref(),
        Some("branch merged into prerelease and the slot returned to idle")
    );
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "assigned",
            "starting",
            "running",
            "merged",
            "cleanup_pending",
            "cleaned"
        ]
    );
}

// official completion 흐름은 worker 보고, ledger refresh, commit-ready가 supervisor
// 한 화면에서 동시에 보이는 경로다. roster/detail state와 distributor feed 순서를
// 함께 확인해 TUI가 완료 보고를 queue head로 일관되게 보여주도록 한다.
#[test]
fn build_supervisor_snapshot_projects_official_completion_and_commit_ready_states() {
    let repo = TempGitRepo::new("supervisor-official-completion");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-88")
        .expect("thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    let completion_report = service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-1",
            None,
            Some("Implemented official completion lifecycle."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be recorded")
        .expect("official completion contract should be returned");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing state should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: follow-up queued",
        )
        .expect("commit-ready state should be recorded");
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let roster_entry = snapshot
        .roster
        .entries
        .first()
        .expect("roster entry should exist");
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("detail should exist");

    assert_eq!(roster_entry.state_label, "commit_ready");
    assert_eq!(detail.state_label, "commit_ready");
    assert_eq!(detail.completion_state_label, "commit_ready");
    assert_eq!(completion_report.completed_turn_id, "turn-1");
    assert_eq!(completion_report.refresh_order, 1);
    assert_eq!(completion_report.completion.task_id, "task-1");
    assert_eq!(completion_report.completion.agent_id, "agent-1");
    assert_eq!(snapshot.distributor.head_summary, "official");
    assert_eq!(
        snapshot.distributor.completion_feed[0].summary,
        "Implemented official completion lifecycle."
    );
    assert_eq!(
        snapshot.distributor.completion_feed[2].summary,
        "official ledger refresh accepted the completion report"
    );
    assert!(
        snapshot
            .distributor
            .runtime_event_feed
            .iter()
            .any(|entry| entry.event_kind == "session_detail_upsert"
                && entry.summary.contains("state: commit_ready"))
    );
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "assigned",
            "starting",
            "running",
            "reported_complete",
            "ledger_refreshing",
            "commit_ready"
        ]
    );
}
