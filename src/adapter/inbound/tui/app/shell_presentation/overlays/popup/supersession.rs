use std::collections::BTreeMap;

use ratatui::text::Line;

use crate::domain::parallel_mode::{
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeSupervisorSnapshot,
};

use super::super::super::super::{AkraTheme, NativeTuiApp};
use super::SupersessionOverlayView;

pub(crate) fn build_supersession_overlay_view(app: &NativeTuiApp) -> SupersessionOverlayView {
    let mode_label = if app.parallel_mode_enabled() {
        "parallel"
    } else {
        "normal"
    };
    let readiness_snapshot = app.parallel_mode_readiness_snapshot();
    let supervisor_snapshot = app.parallel_mode_supervisor_snapshot();
    let summary_lines = build_summary_lines(mode_label, readiness_snapshot, &supervisor_snapshot);
    let capability_lines = readiness_snapshot
        .map(|snapshot| {
            snapshot
                .capabilities
                .iter()
                .map(|capability| Line::from(capability.summary()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![Line::from("parallel readiness has not been inspected yet")]);
    let pool_lines = build_pool_lines(&supervisor_snapshot.pool);
    let roster_lines = build_roster_lines(&supervisor_snapshot);
    let detail_lines = build_detail_lines(&supervisor_snapshot);
    let distributor_lines = build_distributor_lines(&supervisor_snapshot.distributor);
    let mut key_lines = vec![AkraTheme::key_line("Ctrl+R: rerun readiness")];
    if app.parallel_mode_enabled() {
        key_lines.push(AkraTheme::key_line("Ctrl+P: parallel off"));
    } else if readiness_snapshot.is_some_and(|snapshot| snapshot.allows_parallel_mode()) {
        key_lines.push(AkraTheme::key_line("next action: type :parallel on"));
    } else {
        key_lines.push(AkraTheme::key_line(
            "next action: fix readiness blockers, then type :parallel on",
        ));
    }
    key_lines.push(AkraTheme::key_line("Ctrl+O or Esc/Ctrl+C: close"));

    SupersessionOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Supersession Control Tower", " / supervisor board"),
            Line::from("Track readiness, pool capacity, agent roster, and distributor state."),
        ],
        summary_lines,
        capability_lines,
        pool_lines,
        roster_lines,
        detail_lines,
        distributor_lines,
        key_lines,
    }
}

fn build_summary_lines(
    mode_label: &str,
    readiness_snapshot: Option<&crate::domain::parallel_mode::ParallelModeReadinessSnapshot>,
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("mode: {mode_label}")),
        Line::from(format!(
            "board state: {}",
            supervisor_snapshot.state_label()
        )),
        Line::from(format!(
            "readiness: {}",
            readiness_snapshot
                .map(|snapshot| snapshot.readiness_label().to_string())
                .unwrap_or_else(|| "not checked yet".to_string())
        )),
        Line::from(format!("workspace: {}", supervisor_snapshot.workspace_path)),
        Line::from(format!(
            "pool: {}",
            supervisor_snapshot.pool.compact_summary()
        )),
        Line::from(format!(
            "agents: {}  |  queue: {}",
            supervisor_snapshot.roster.compact_summary(),
            supervisor_snapshot.distributor.compact_summary()
        )),
    ];

    if let Some(alert) = readiness_snapshot.and_then(|snapshot| snapshot.top_alert.as_deref()) {
        lines.push(Line::from(format!("alert: {alert}")));
    } else if let Some(notice) = supervisor_snapshot.top_notice.as_deref() {
        lines.push(Line::from(format!("notice: {notice}")));
    }

    lines
}

fn build_pool_lines(pool: &ParallelModePoolBoardSnapshot) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("configured size: {}", pool.configured_size)),
        Line::from(format!("pool root: {}", pool.pool_root_label)),
        Line::from(format!(
            "summary: idle {} / leased {} / running {} / cleanup {} / blocked {} / missing {} / unavailable {}",
            pool.idle_slots,
            pool.leased_slots,
            pool.running_slots,
            pool.awaiting_cleanup_slots,
            pool.blocked_slots,
            pool.missing_slots,
            pool.unavailable_slots
        )),
        Line::from(format!("reconcile: {}", pool.reconcile_status)),
    ];
    if pool.exhausted {
        lines.push(Line::from("capacity: exhausted"));
    }
    lines.extend(pool.slots.iter().map(|slot| {
        Line::from(format!(
            "{}: {} / branch {} / worktree {} / owner {}",
            slot.slot_id,
            slot.state.label(),
            slot.branch_name,
            slot.worktree_label,
            slot.owner_label
        ))
    }));
    lines
}

fn build_roster_lines(supervisor_snapshot: &ParallelModeSupervisorSnapshot) -> Vec<Line<'static>> {
    let roster = &supervisor_snapshot.roster;
    let mut lines = vec![
        Line::from(format!("active count: {}", roster.active_count())),
        Line::from(format!("state: {}", supervisor_snapshot.state_label())),
    ];
    if roster.entries.is_empty() {
        lines.push(Line::from(format!("placeholder: {}", roster.empty_state)));
        lines.push(Line::from(
            "expected row: agent / task / slot / branch / state / age / summary",
        ));
        return lines;
    }

    let slot_health_by_id = supervisor_snapshot
        .pool
        .slots
        .iter()
        .map(|slot| (slot.slot_id.as_str(), slot_health_summary_from_slot(slot)))
        .collect::<BTreeMap<_, _>>();

    lines.extend(roster.entries.iter().map(|entry| {
        let state_label = display_supersession_state_label(&entry.state_label);
        let duration_label =
            display_roster_duration_label(&entry.state_label, &entry.duration_label);
        let slot_health = slot_health_by_id
            .get(entry.slot_id.as_str())
            .map(String::as_str)
            .unwrap_or("slot not projected");
        Line::from(format!(
            "{}: {} / {} / {} / {} / {} / {} / {}",
            entry.agent_id,
            entry.task_title,
            entry.slot_id,
            entry.branch_name,
            state_label,
            duration_label,
            entry.latest_summary,
            slot_health
        ))
    }));
    lines
}

fn build_detail_lines(supervisor_snapshot: &ParallelModeSupervisorSnapshot) -> Vec<Line<'static>> {
    let Some(detail) = supervisor_snapshot.detail.session.as_ref() else {
        return vec![
            Line::from("selection: none"),
            Line::from(format!(
                "board state: {}",
                supervisor_snapshot.state_label()
            )),
            Line::from(format!(
                "detail state: {}",
                supervisor_snapshot.detail.empty_state
            )),
        ];
    };

    let mut lines = vec![
        Line::from(format!(
            "selection: {} / {} / {}",
            detail.agent_id,
            detail.slot_id,
            display_supersession_state_label(&detail.state_label)
        )),
        Line::from(format!("task: {} / {}", detail.task_id, detail.task_title)),
        Line::from(format!(
            "thread: {}",
            detail.thread_id.as_deref().unwrap_or("not captured yet")
        )),
        Line::from(format!(
            "slot health: {}",
            slot_health_summary(supervisor_snapshot, &detail.slot_id)
        )),
        Line::from(format!("worktree: {}", detail.worktree_path)),
        Line::from(format!("branch: {}", detail.branch_name)),
        Line::from(format!("lease start: {}", detail.lease_started_at)),
        Line::from(format!(
            "completion: {}",
            display_supersession_state_label(&detail.completion_state_label)
        )),
        Line::from(format!("latest: {}", detail.latest_summary)),
        Line::from(format!("validation: {}", detail.validation_summary)),
        Line::from(format!(
            "ledger refresh: {}",
            detail.authority_refresh_outcome
        )),
        Line::from(format!(
            "distributor: {}",
            detail
                .distributor_outcome
                .as_deref()
                .unwrap_or("no distributor outcome recorded")
        )),
    ];
    lines.push(Line::from("history:"));
    lines.extend(detail.history.iter().map(|entry| {
        Line::from(format!(
            "{} / {} / {}",
            entry.timestamp,
            display_supersession_state_label(&entry.state_label),
            entry.summary
        ))
    }));
    lines
}

fn build_distributor_lines(distributor: &ParallelModeDistributorSnapshot) -> Vec<Line<'static>> {
    let blocked_head_detail = distributor
        .head_blocked_detail
        .as_deref()
        .map(str::trim)
        .filter(|detail| !detail.is_empty());
    let mut lines = vec![
        Line::from(format!("head: {}", distributor.head_summary)),
        Line::from(format!("queue depth: {}", distributor.queue_depth())),
    ];
    if blocked_head_detail != Some(distributor.note.trim()) {
        lines.push(Line::from(format!("note: {}", distributor.note)));
    }
    if let Some(detail) = blocked_head_detail {
        lines.push(Line::from(format!("blocked head: {detail}")));
    }
    if let Some(provenance) = distributor.head_rebase_provenance.as_deref() {
        lines.push(Line::from(format!("provenance: {provenance}")));
    }
    lines.extend(build_orchestrator_lines(distributor));
    if distributor.queue_items.is_empty() {
        lines.push(Line::from(
            "queue: no items are waiting for distributor work",
        ));
    } else {
        lines.extend(
            distributor
                .queue_items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let row_label = if index == 0 { "current" } else { "next" };
                    Line::from(format!(
                        "{row_label}: {} / {} / {} / {} / {} / {}",
                        item.source_agent,
                        item.task_title,
                        item.queue_state.label(),
                        item.branch_name,
                        item.commit_short_sha,
                        item.integration_note
                    ))
                }),
        );
    }
    lines.push(Line::from("completion feed:"));
    lines.extend(
        distributor
            .completion_feed
            .iter()
            .map(|entry| Line::from(format!("{}: {}", entry.stage_label, entry.summary))),
    );
    lines
}

fn build_orchestrator_lines(distributor: &ParallelModeDistributorSnapshot) -> Vec<Line<'static>> {
    let status = &distributor.orchestrator_status;
    let mut lines = vec![
        Line::from(format!("orchestrator head: {}", status.queue_head)),
        Line::from(format!("orchestrator barrier: {}", status.barrier_state)),
        Line::from(format!(
            "orchestrator held queue: {}",
            status.held_queue_count
        )),
        Line::from(format!(
            "integration worktree: {}",
            status.integration_worktree_readiness
        )),
    ];
    if let Some(reason) = status.blocked_reason.as_deref() {
        lines.push(Line::from(format!("blocked reason: {reason}")));
    }
    if !status.conflict_files.is_empty() {
        lines.push(Line::from(format!(
            "conflict files: {}",
            status.conflict_files.join(", ")
        )));
    }
    if let Some(reason) = status.slot_return_wait_reason.as_deref() {
        lines.push(Line::from(format!("slot return: {reason}")));
    }
    lines
}

fn display_supersession_state_label(state_label: &str) -> String {
    match state_label {
        "reported_complete" => "reported".to_string(),
        "commit_ready" => "official".to_string(),
        other => other.replace('_', " "),
    }
}

fn display_roster_duration_label(state_label: &str, duration_label: &str) -> String {
    let trimmed_duration = duration_label.trim();
    if state_label == "running" && !trimmed_duration.is_empty() {
        return format!("working {trimmed_duration}");
    }

    trimmed_duration.to_string()
}

fn slot_health_summary(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    slot_id: &str,
) -> String {
    supervisor_snapshot
        .pool
        .slots
        .iter()
        .find(|slot| slot.slot_id == slot_id)
        .map(slot_health_summary_from_slot)
        .unwrap_or_else(|| "slot not projected".to_string())
}

fn slot_health_summary_from_slot(slot: &ParallelModePoolSlotSnapshot) -> String {
    match slot.state {
        ParallelModePoolSlotState::Leased
        | ParallelModePoolSlotState::Running
        | ParallelModePoolSlotState::AwaitingCleanup => "slot ok".to_string(),
        ParallelModePoolSlotState::Idle => "slot idle".to_string(),
        ParallelModePoolSlotState::Missing => format!(
            "slot missing: {}",
            worktree_health_detail(&slot.worktree_label)
        ),
        ParallelModePoolSlotState::Blocked => format!(
            "slot blocked: {}",
            worktree_health_detail(&slot.worktree_label)
        ),
        ParallelModePoolSlotState::Unavailable => format!(
            "slot unavailable: {}",
            worktree_health_detail(&slot.worktree_label)
        ),
    }
}

fn worktree_health_detail(worktree_label: &str) -> String {
    worktree_label
        .rsplit_once(" / ")
        .map(|(_, detail)| detail.trim())
        .filter(|detail| !detail.is_empty())
        .unwrap_or(worktree_label.trim())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{build_detail_lines, build_distributor_lines, build_roster_lines};
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
        ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
        ParallelModeCompletionFeedEntry, ParallelModeDistributorQueueItem,
        ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus,
        ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
        ParallelModeQueueItemState, ParallelModeSupervisorDetailSnapshot,
        ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
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
        let detail =
            "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected";
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
        assert!(rendered.contains(
            "slot return: slot `slot-1` stays running until the queue head is integrated"
        ));
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
            detail_rendered
                .contains("slot health: slot blocked: lease exists but worktree is missing")
        );
    }
}
