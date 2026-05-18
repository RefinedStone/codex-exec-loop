use super::{
    build_detail_lines, build_distributor_lines, build_parallel_event_stream_lines,
    build_roster_lines,
};
use crate::adapter::inbound::tui::app::parallel_supervisor_events::parallel_supervisor_event_line;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeCompletionFeedEntry, ParallelModeDistributorQueueItem,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus, ParallelModePoolBoardSnapshot,
    ParallelModePoolSlotSnapshot, ParallelModePoolSlotState, ParallelModeQueueItemState,
    ParallelModeRuntimeEventFeedEntry, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};

/*
These tests pin the supersession control tower's copy contract, not ratatui layout.
The domain snapshots deliberately carry machine-oriented state names and orchestration
diagnostics; this popup layer must translate them into operator-facing rows that explain
why parallel work is blocked, waiting, or ready to merge.
*/
#[test]
fn distributor_lines_render_blocked_head_and_rebase_provenance() {
    /*
    Rebase provenance is the only clue that a blocked distributor head was rewritten
    onto the integration baseline before push failed. If this line disappears, the
    operator cannot distinguish "needs manual push recovery" from a generic queue block.
    */
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
    // The snapshot exposes both a generic note and structured blocked-head detail.
    // When they are identical, the popup should show the stronger label once.
    /*
    This protects the narrow popup from repeating a long recovery message twice.
    The stronger "blocked head" label carries operator intent, while "note" is
    only useful when it adds different queue context.
    */
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
    /*
    Distributor queue rows are scanned under pressure during integration. The first row
    must be marked as the active head, later rows as waiting work, and enum labels must
    be converted away from Rust-style identifiers before they reach the UI.
    */
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![
            /*
            The first item is intentionally pr_pending rather than blocked. That
            proves the "current" marker belongs to queue order, not only to error
            states, and keeps GitHub wait states visible as active head work.
            */
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
    /*
    Orchestrator status is the bridge between distributor queue state and the integration
    worktree. These assertions keep conflict files, held queue count, and slot-return
    reasons visible so the operator knows why capacity is intentionally withheld.
    */
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
fn distributor_lines_render_runtime_event_feed_with_sequence_and_revision() {
    /*
    runtime_events는 current projection row가 덮어쓴 과거 전이를 보여 주는 감사 feed다.
    Supersession popup은 raw snake_case event kind보다 sequence, observed revision, summary를
    한 줄에 보여줘야 operator가 최신 store write를 바로 읽을 수 있다.
    */
    let snapshot = ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queued")
        .with_runtime_event_feed(vec![ParallelModeRuntimeEventFeedEntry::new(
            42,
            "session_detail_upsert",
            "session_detail",
            "slot-1:task-1",
            7,
            "runtime session detail stored / session: slot-1:task-1 / state: commit_ready",
            "2026-04-17T00:03:00Z",
        )]);
    let rendered = build_distributor_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("runtime events:"));
    assert!(rendered.contains(
        "event #42 @ 00:03 / session detail:slot-1:task-1 / session detail upsert / rev 7 / runtime session detail stored / session: slot-1:task-1 / state: commit_ready"
    ));
    assert!(!rendered.contains("session_detail_upsert"));
}

#[test]
fn parallel_event_stream_does_not_replay_raw_runtime_feed() {
    /*
    The DB projection loads a compact recent runtime feed that can include stale rows from
    before the operator opened :parallel. The live event stream must use the append-only
    supervisor event log instead, where the runtime feed has already been baselined.
    */
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/workspace",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "empty"),
        ParallelModeSupervisorDetailSnapshot::new(None, "empty"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queued")
            .with_runtime_event_feed(vec![
                runtime_event_feed_entry(3, "third runtime write"),
                runtime_event_feed_entry(1, "first runtime write"),
                runtime_event_feed_entry(2, "second runtime write"),
            ]),
        None,
    );
    let rendered = build_parallel_event_stream_lines(
        &snapshot,
        vec![parallel_supervisor_event_line(
            "11:45:04",
            "You",
            "안녕하세요",
        )],
    )
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>()
    .join("\n");

    assert!(rendered.contains("You: 안녕하세요"));
    assert!(!rendered.contains("first runtime write"));
    assert!(!rendered.contains("second runtime write"));
    assert!(!rendered.contains("third runtime write"));
}

fn runtime_event_feed_entry(
    sequence: i64,
    summary: impl Into<String>,
) -> ParallelModeRuntimeEventFeedEntry {
    ParallelModeRuntimeEventFeedEntry::new(
        sequence,
        "parallel_runtime_reset",
        "parallel_runtime",
        "pool",
        60,
        summary,
        format!("2026-05-13T11:45:{sequence:02}+00:00"),
    )
}

#[test]
fn roster_and_detail_lines_render_distinct_reported_and_official_labels() {
    /*
    Agent-reported completion and official ledger refresh are different phases. The
    roster should keep the quick reported signal, while detail/history should show the
    official completion lifecycle without leaking raw `reported_complete`/`commit_ready`
    state names into the popup.
    */
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/workspace",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        /*
        Roster and detail deliberately disagree on the lifecycle phase: the roster
        row uses the agent-facing reported state, while the selected detail has
        already advanced to commit_ready. The popup must preserve both perspectives.
        */
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
                    /*
                    History rows keep the chronological bridge from agent report
                    to official ledger acceptance. The assertions below pin both
                    state-label humanization and ordering-sensitive copy.
                    */
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
    let roster_rendered = build_roster_lines(&snapshot, "|")
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
    assert!(detail_rendered.contains("timeline: slot-1 / slot-1:task-1"));
    assert!(detail_rendered.contains("events: 00:01 reported -> 00:02 official"));
    assert!(detail_rendered.contains(
        "last event: 00:02 official / official ledger refresh accepted the completion report"
    ));
    assert!(
        detail_rendered.contains("2026-04-17T00:01:00Z / reported / agent reported completion")
    );
    assert!(detail_rendered.contains(
        "2026-04-17T00:02:00Z / official / official ledger refresh accepted the completion report"
    ));
    assert!(!detail_rendered.contains("delivery:"));
    assert!(!detail_rendered.contains("commit_ready"));
}

#[test]
fn detail_lines_keep_delivery_boundary_visible_after_timeline_condenses() {
    /*
    Long successful deliveries quickly exceed the compact timeline budget. The
    dedicated delivery boundary keeps source push, PR automation, and merge
    integration visible even after older lifecycle events are collapsed.
    */
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/workspace",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "empty"),
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
                "cleaned",
                "cleaned",
                "slot cleaned and returned to the idle pool",
                "tests passed",
                "official ledger refresh succeeded",
                Some("branch merged into prerelease and the slot returned to idle".to_string()),
                vec![
                    ParallelModeAgentSessionHistoryEntry::new(
                        "assigned",
                        "2026-04-17T00:00:00Z",
                        "slot lease acquired and branch reserved for launch",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "starting",
                        "2026-04-17T00:01:00Z",
                        "thread prepared for agent launch",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "running",
                        "2026-04-17T00:02:00Z",
                        "agent session entered the running state",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "reported_complete",
                        "2026-04-17T00:03:00Z",
                        "agent reported completion",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "ledger_refreshing",
                        "2026-04-17T00:04:00Z",
                        "official refresh worker started",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "commit_ready",
                        "2026-04-17T00:05:00Z",
                        "official ledger refresh accepted the completion report",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "merge_queued",
                        "2026-04-17T00:06:00Z",
                        "distributor accepted the result and queued it for GitHub delivery",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "pushing",
                        "2026-04-17T00:07:00Z",
                        "distributor is pushing the source branch to origin",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "pr_pending",
                        "2026-04-17T00:08:00Z",
                        "source branch pushed and pull request ensure is in progress",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "merge_pending",
                        "2026-04-17T00:09:00Z",
                        "pull request is open and merge readiness is being checked",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "integrating",
                        "2026-04-17T00:10:00Z",
                        "pull request is ready and distributor is integrating the queued branch",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "merged",
                        "2026-04-17T00:11:00Z",
                        "agent branch is merged into prerelease and awaiting slot cleanup",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "cleanup_pending",
                        "2026-04-17T00:12:00Z",
                        "agent branch is merged into prerelease and awaiting slot cleanup",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "cleaned",
                        "2026-04-17T00:13:00Z",
                        "slot cleaned and returned to the idle pool",
                    ),
                ],
                "2026-04-17T00:13:00Z",
            )),
            "empty",
        ),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queued"),
        None,
    );
    let detail_rendered = build_detail_lines(&snapshot)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(detail_rendered.contains("events: 00:08 ... pr pending"));
    assert!(detail_rendered.contains("delivery: push 00:07 -> PR 00:08 -> merge 00:10"));
    assert!(detail_rendered.contains("last event: 00:13 cleaned"));
}

#[test]
fn roster_and_detail_lines_surface_missing_slot_worktree_health() {
    /*
    Slot health comes from the pool board, while agent rows come from the live roster.
    The popup merges them so a running agent can still show that its leased worktree is
    missing or blocked, which is the condition an operator must repair before cleanup.
    */
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/workspace",
        /*
        The pool board is the only source of worktree health. The agent session
        below still looks running, so this fixture proves the popup joins pool
        reconciliation data back onto roster and detail rows.
        */
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
    let roster_rendered = build_roster_lines(&snapshot, "|")
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
