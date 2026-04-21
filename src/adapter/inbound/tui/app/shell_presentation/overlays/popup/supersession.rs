use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::domain::parallel_mode::{
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModeSupervisorSnapshot,
};

use super::super::super::super::NativeTuiApp;
use super::super::SupersessionOverlayView;

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
    let key_lines = vec![
        Line::from("r: rerun readiness    Ctrl+P: parallel off"),
        Line::from("Ctrl+O or Esc/Ctrl+C: close"),
    ];

    SupersessionOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Supersession Control Tower",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / supervisor board"),
            ]),
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

    lines.extend(roster.entries.iter().map(|entry| {
        Line::from(format!(
            "{}: {} / {} / {} / {} / {} / {}",
            entry.agent_id,
            entry.task_title,
            entry.slot_id,
            entry.branch_name,
            display_supersession_state_label(&entry.state_label),
            entry.duration_label,
            entry.latest_summary
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
        Line::from(format!("worktree: {}", detail.worktree_path)),
        Line::from(format!("branch: {}", detail.branch_name)),
        Line::from(format!("lease start: {}", detail.lease_started_at)),
        Line::from(format!(
            "completion: {}",
            display_supersession_state_label(&detail.completion_state_label)
        )),
        Line::from(format!("latest: {}", detail.latest_summary)),
        Line::from(format!("validation: {}", detail.validation_summary)),
        Line::from(format!("ledger refresh: {}", detail.ledger_refresh_outcome)),
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

fn display_supersession_state_label(state_label: &str) -> String {
    match state_label {
        "reported_complete" => "reported".to_string(),
        "commit_ready" => "official".to_string(),
        other => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::{build_detail_lines, build_distributor_lines, build_roster_lines};
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
        ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
        ParallelModeCompletionFeedEntry, ParallelModeDistributorQueueItem,
        ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModeQueueItemState,
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
}
