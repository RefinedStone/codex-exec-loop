use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::domain::parallel_mode::{
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModeSupervisorSnapshot,
};

use super::super::super::NativeTuiApp;
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
        Line::from(format!(
            "summary: idle {} / leased {} / blocked {} / unavailable {}",
            pool.idle_slots, pool.leased_slots, pool.blocked_slots, pool.unavailable_slots
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
            entry.state_label,
            entry.duration_label,
            entry.latest_summary
        ))
    }));
    lines
}

fn build_detail_lines(supervisor_snapshot: &ParallelModeSupervisorSnapshot) -> Vec<Line<'static>> {
    let first_slot = supervisor_snapshot.pool.slots.first();
    let slot_template = first_slot
        .map(|slot| {
            format!(
                "{} / {} / {}",
                slot.slot_id,
                slot.state.label(),
                slot.branch_name
            )
        })
        .unwrap_or_else(|| "no slot template available".to_string());

    vec![
        Line::from("selection: none"),
        Line::from(format!(
            "board state: {}",
            supervisor_snapshot.state_label()
        )),
        Line::from(format!("slot template: {slot_template}")),
        Line::from("detail fields: task, thread, worktree, validation, distributor outcome"),
        Line::from("placeholder until agent lifecycle tracking lands"),
    ]
}

fn build_distributor_lines(distributor: &ParallelModeDistributorSnapshot) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("head: {}", distributor.head_summary)),
        Line::from(format!("queue depth: {}", distributor.queue_depth())),
        Line::from(format!("note: {}", distributor.note)),
    ];
    if distributor.queue_items.is_empty() {
        lines.push(Line::from(
            "queue: no items are waiting for distributor work",
        ));
    } else {
        lines.extend(distributor.queue_items.iter().map(|item| {
            Line::from(format!(
                "{}: {} / {} / {} / {} / {}",
                item.source_agent,
                item.task_title,
                item.queue_state.label(),
                item.branch_name,
                item.commit_short_sha,
                item.integration_note
            ))
        }));
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
