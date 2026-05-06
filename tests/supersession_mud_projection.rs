use codex_exec_loop_native::adapter::inbound::tui::supersession_mud::build_supersession_mud_lines;
use codex_exec_loop_native::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeDistributorQueueItem, ParallelModeDistributorSnapshot,
    ParallelModeOrchestratorStatus, ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeQueueItemState, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};

#[test]
fn supersession_mud_projection_integrates_lanes_actor_timeline_and_corridor() {
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root/projects/codex-exec-loop",
        ParallelModePoolBoardSnapshot::new(
            3,
            "/tmp/root/projects/codex-exec-loop-akra-worktrees/pool",
            "idle",
            vec![
                ParallelModePoolSlotSnapshot::new(
                    "slot-1",
                    ParallelModePoolSlotState::Running,
                    "akra-agent/slot-1/parallel-mode-mud-ui-pack",
                    "akra-pool/slot-1",
                    "agent-1 / task-1",
                ),
                ParallelModePoolSlotSnapshot::new(
                    "slot-2",
                    ParallelModePoolSlotState::Idle,
                    "prerelease",
                    "akra-pool/slot-2",
                    "idle",
                ),
                ParallelModePoolSlotSnapshot::new(
                    "slot-3",
                    ParallelModePoolSlotState::Blocked,
                    "akra-agent/slot-3/blocked-rendering-recovery",
                    "akra-pool/slot-3 / dirty worktree",
                    "agent-3 / task-3",
                ),
            ],
        ),
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-1",
                "Parallel Mode MUD Timeline UI Pack",
                "slot-1",
                "akra-agent/slot-1/parallel-mode-mud-ui-pack",
                "running",
                "04m12s",
                "rendering the selected session timeline and distributor corridor",
            )],
            "no active agents",
        ),
        ParallelModeSupervisorDetailSnapshot::new(
            Some(ParallelModeAgentSessionDetailSnapshot::new(
                "slot-1:task-1",
                "agent-1",
                "task-1",
                "Parallel Mode MUD Timeline UI Pack",
                "slot-1",
                Some("thread-1".to_string()),
                "/tmp/root/projects/codex-exec-loop-akra-worktrees/pool/slot-1",
                "akra-agent/slot-1/parallel-mode-mud-ui-pack",
                "2026-05-06T12:00:00Z",
                "commit_ready",
                "commit_ready",
                "official ledger refresh accepted the completion report",
                "cargo test passed",
                "official ledger refresh succeeded",
                Some("commit-ready result accepted into distributor queue".to_string()),
                vec![
                    ParallelModeAgentSessionHistoryEntry::new(
                        "assigned",
                        "2026-05-06T12:00:00Z",
                        "slot lease acquired",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "running",
                        "2026-05-06T12:01:00Z",
                        "agent session is active",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "commit_ready",
                        "2026-05-06T12:08:00Z",
                        "official ledger refresh accepted the completion report",
                    ),
                ],
                "2026-05-06T12:08:00Z",
            )),
            "no detail",
        ),
        ParallelModeDistributorSnapshot::new(
            vec![
                ParallelModeDistributorQueueItem::new(
                    "agent-1",
                    "Parallel Mode MUD Timeline UI Pack",
                    ParallelModeQueueItemState::Queued,
                    "akra-agent/slot-1/parallel-mode-mud-ui-pack",
                    "abc1234",
                    "commit-ready result accepted into distributor queue",
                ),
                ParallelModeDistributorQueueItem::new(
                    "agent-2",
                    "Rendering Recovery",
                    ParallelModeQueueItemState::Queued,
                    "akra-agent/slot-2/rendering-recovery",
                    "def5678",
                    "held behind queue head",
                ),
            ],
            Vec::new(),
            "queued",
            "commit-ready result accepted into distributor queue",
        )
        .with_orchestrator_status(ParallelModeOrchestratorStatus {
            queue_head: "agent-1 / task-1 / queued".to_string(),
            barrier_state: "head queued holds later queue items".to_string(),
            blocked_reason: None,
            conflict_files: Vec::new(),
            held_queue_count: 1,
            integration_worktree_readiness: "ready: prerelease worktree clean".to_string(),
            slot_return_wait_reason: Some(
                "slot `slot-1` stays running until the queue head is integrated".to_string(),
            ),
        }),
        Some("parallel mode dispatch refreshed".to_string()),
    );

    let projection = build_supersession_mud_lines(&snapshot);
    let rendered = [
        projection.summary_lines,
        projection.pool_lines,
        projection.roster_lines,
        projection.detail_lines,
        projection.distributor_lines,
    ]
    .concat()
    .join("\n");

    assert!(rendered.contains("realm: supervise"));
    assert!(rendered.contains("lane map: [slot-1:RUN] [slot-2:IDLE] [slot-3:BLOCK]"));
    assert!(rendered.contains("actor agent-1 in slot-1"));
    assert!(rendered.contains("quest log: slot-1 / agent-1"));
    assert!(rendered.contains("trail: assigned -> running -> commit ready"));
    assert!(rendered.contains("exit corridor: head queued | depth 2"));
    assert!(rendered.contains("held behind head: 1 quest(s)"));
    assert!(
        rendered.lines().all(|line| line.chars().count() <= 112),
        "MUD projection should keep line width bounded for narrow TUI panels:\n{rendered}"
    );
}
