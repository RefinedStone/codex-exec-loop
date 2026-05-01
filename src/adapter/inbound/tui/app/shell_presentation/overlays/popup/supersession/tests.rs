use super::{build_detail_lines, build_distributor_lines, build_roster_lines};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeCompletionFeedEntry, ParallelModeDistributorQueueItem,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus, ParallelModePoolBoardSnapshot,
    ParallelModePoolSlotSnapshot, ParallelModePoolSlotState, ParallelModeQueueItemState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};

#[test]
fn distributor_lines_render_blocked_head_and_rebase_provenance() {
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![ParallelModeDistributorQueueItem::new(
            "agent-1",
            "Task One",
            ParallelModeQueueItemState::Blocked,
            "akra-agent/slot-1/task-one",
            "def4567",
            "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected",
        )],
        vec![ParallelModeCompletionFeedEntry::new(
            "merge queued",
            "distributor queue head is blocked",
        )],
        "blocked",
        "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected",
    )
    .with_head_blocked_detail(Some(
        "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected"
            .to_string(),
    ))
    .with_head_rebase_provenance(Some("rebased abc1234 -> def4567 onto `akra`".to_string()));

    let rendered = build_distributor_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("blocked head: rebased branch"));
    assert!(rendered.contains("provenance: rebased abc1234 -> def4567 onto `akra`"));
}

#[test]
fn distributor_lines_omit_duplicate_note_when_blocked_detail_matches() {
    let detail = "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected";
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![ParallelModeDistributorQueueItem::new(
            "agent-1",
            "Task One",
            ParallelModeQueueItemState::Blocked,
            "akra-agent/slot-1/task-one",
            "def4567",
            detail,
        )],
        vec![],
        "blocked",
        detail,
    )
    .with_head_blocked_detail(Some(detail.to_string()));

    let rendered = build_distributor_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!rendered.contains(&format!("note: {detail}")));
    assert!(rendered.contains(&format!("blocked head: {detail}")));
}

#[test]
fn distributor_lines_render_head_marker_and_humanized_queue_state_labels() {
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![
            ParallelModeDistributorQueueItem::new(
                "agent-1",
                "Task One",
                ParallelModeQueueItemState::PrPending,
                "akra-agent/slot-1/task-one",
                "abc1234",
                "pull request #42 is open and waiting for merge",
            ),
            ParallelModeDistributorQueueItem::new(
                "agent-2",
                "Task Two",
                ParallelModeQueueItemState::Queued,
                "akra-agent/slot-2/task-two",
                "def5678",
                "commit-ready result accepted into distributor queue",
            ),
        ],
        vec![],
        ParallelModeQueueItemState::PrPending.label(),
        "pull request #42 is open and waiting for merge",
    );

    let rendered = build_distributor_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("head: pr pending"));
    assert!(rendered.contains(
        "current: agent-1 / Task One / pr pending / akra-agent/slot-1/task-one / abc1234 / pull request #42 is open and waiting for merge"
    ));
    assert!(rendered.contains(
        "next: agent-2 / Task Two / queued / akra-agent/slot-2/task-two / def5678 / commit-ready result accepted into distributor queue"
    ));
    assert!(!rendered.contains("pr_pending"));
}

#[test]
fn distributor_lines_render_orchestrator_status_details() {
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![ParallelModeDistributorQueueItem::new(
            "agent-1",
            "Task One",
            ParallelModeQueueItemState::Blocked,
            "akra-agent/slot-1/task-one",
            "abc1234",
            "could not cherry-pick into `akra` cleanly",
        )],
        vec![],
        "blocked",
        "could not cherry-pick into `akra` cleanly",
    )
    .with_orchestrator_status(ParallelModeOrchestratorStatus {
        queue_head: "agent-1 / task-1 / blocked".to_string(),
        barrier_state: "blocked".to_string(),
        blocked_reason: Some("could not cherry-pick into `akra` cleanly".to_string()),
        conflict_files: vec!["conflict.txt".to_string()],
        held_queue_count: 2,
        integration_worktree_readiness: "blocked: dirty worktree".to_string(),
        slot_return_wait_reason: Some(
            "slot `slot-1` stays running until the queue head is integrated".to_string(),
        ),
    });

    let rendered = build_distributor_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("orchestrator head: agent-1 / task-1 / blocked"));
    assert!(rendered.contains("orchestrator barrier: blocked"));
    assert!(rendered.contains("orchestrator held queue: 2"));
    assert!(rendered.contains("integration worktree: blocked: dirty worktree"));
    assert!(rendered.contains("blocked reason: could not cherry-pick into `akra` cleanly"));
    assert!(rendered.contains("conflict files: conflict.txt"));
    assert!(
        rendered.contains(
            "slot return: slot `slot-1` stays running until the queue head is integrated"
        )
    );
}

#[test]
fn roster_and_detail_lines_render_distinct_reported_and_official_labels() {
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/workspace",
        ParallelModePoolBoardSnapshot::new(0, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-1",
                "Task One",
                "slot-1",
                "akra-agent/slot-1/task-one",
                "reported_complete",
                "reported",
                "agent reported completion",
            )],
            "empty",
        ),
        ParallelModeSupervisorDetailSnapshot::new(
            Some(ParallelModeAgentSessionDetailSnapshot::new(
                "slot-1:task-1",
                "agent-1",
                "task-1",
                "Task One",
                "slot-1",
                Some("thread-1".to_string()),
                "/tmp/worktree",
                "akra-agent/slot-1/task-one",
                "2026-04-17T00:00:00Z",
                "commit_ready",
                "commit_ready",
                "official ledger refresh accepted the completion report",
                "tests passed",
                "official ledger refresh succeeded",
                Some("commit-ready result accepted into distributor queue".to_string()),
                vec![
                    ParallelModeAgentSessionHistoryEntry::new(
                        "reported_complete",
                        "2026-04-17T00:01:00Z",
                        "agent reported completion",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "commit_ready",
                        "2026-04-17T00:02:00Z",
                        "official ledger refresh accepted the completion report",
                    ),
                ],
                "2026-04-17T00:02:00Z",
            )),
            "empty",
        ),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "official", "queued"),
        None,
    );

    let roster_rendered = build_roster_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let detail_rendered = build_detail_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(roster_rendered.contains("reported / reported / agent reported completion"));
    assert!(!roster_rendered.contains("reported_complete"));
    assert!(detail_rendered.contains("selection: agent-1 / slot-1 / official"));
    assert!(detail_rendered.contains("completion: official"));
    assert!(
        detail_rendered.contains("2026-04-17T00:01:00Z / reported / agent reported completion")
    );
    assert!(detail_rendered.contains(
        "2026-04-17T00:02:00Z / official / official ledger refresh accepted the completion report"
    ));
    assert!(!detail_rendered.contains("commit_ready"));
}

#[test]
fn roster_and_detail_lines_surface_missing_slot_worktree_health() {
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/workspace",
        ParallelModePoolBoardSnapshot::new(
            3,
            "/tmp/pool",
            "reconcile blocked",
            vec![ParallelModePoolSlotSnapshot::new(
                "slot-1",
                ParallelModePoolSlotState::Blocked,
                "akra-agent/slot-1/task-one",
                "akra-pool/slot-1 / lease exists but worktree is missing",
                "agent-1 / task-1",
            )],
        ),
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-1",
                "Task One",
                "slot-1",
                "akra-agent/slot-1/task-one",
                "running",
                "1m 5s",
                "agent session is active in the leased slot",
            )],
            "empty",
        ),
        ParallelModeSupervisorDetailSnapshot::new(
            Some(ParallelModeAgentSessionDetailSnapshot::new(
                "slot-1:task-1",
                "agent-1",
                "task-1",
                "Task One",
                "slot-1",
                Some("thread-1".to_string()),
                "/tmp/missing-worktree",
                "akra-agent/slot-1/task-one",
                "2026-04-17T00:00:00Z",
                "running",
                "in_progress",
                "agent session is active in the leased slot",
                "not checked yet",
                "not refreshed yet",
                None,
                Vec::new(),
                "2026-04-17T00:01:05Z",
            )),
            "empty",
        ),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queued"),
        None,
    );

    let roster_rendered = build_roster_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let detail_rendered = build_detail_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(roster_rendered.contains("running / working 1m 5s"));
    assert!(roster_rendered.contains("slot blocked: lease exists but worktree is missing"));
    assert!(
        detail_rendered.contains("slot health: slot blocked: lease exists but worktree is missing")
    );
}
