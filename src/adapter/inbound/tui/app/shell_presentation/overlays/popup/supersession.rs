#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use ratatui::text::{Line, Span};

use crate::adapter::inbound::tui::supersession_mud::{
    ParallelModeProgressSummary, SupersessionMudUiState, parallel_mode_progress_summary,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentSessionDetailSnapshot,
    ParallelModeDistributorQueueItem, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
    ParallelModeQueueItemState, ParallelModeSupervisorSnapshot,
};

use super::super::super::super::parallel_supervisor_events::parallel_supervisor_snapshot_stream_lines;
use super::super::super::super::parallel_terminal_delivery::ParallelLiveStreamModel;
use super::super::super::super::{AkraTheme, ShellOverlay};
use super::super::super::ConversationScreenModel;
use super::SupersessionOverlayView;

/* Supersession is the operator board for parallel mode. It intentionally keeps
 * readiness, pool capacity, active roster, selected detail, and distributor state
 * as separate line groups so the popup can answer "can work start?", "who is
 * running?", and "why is integration blocked?" without requiring navigation.
 */
pub(crate) fn build_supersession_overlay_view(
    screen_model: &ConversationScreenModel<'_>,
    mud_ui_state: &SupersessionMudUiState,
) -> SupersessionOverlayView {
    let readiness_snapshot = screen_model.parallel_mode_readiness.as_ref();
    let supervisor_snapshot = &screen_model.parallel_mode_supervisor;
    let planning_projection = &screen_model.planning_runtime_projection;
    let activity_frame = supersession_activity_frame(screen_model.animation_elapsed_millis);
    let progress = parallel_mode_progress_summary(
        supervisor_snapshot,
        planning_projection.queue_projection(),
        screen_model.parallel_mode_control_effect_in_flight,
    );
    let selected_slot_id = mud_ui_state.selected_lane_slot_id(supervisor_snapshot);
    let selected_lane =
        selected_slot_id.and_then(|slot_id| operations_lane(supervisor_snapshot, slot_id));
    let (accepted_queue_lines, accepted_queue_pressure) =
        build_accepted_queue_lines(screen_model, supervisor_snapshot);
    let overview_lines = build_operations_overview_lines(
        screen_model,
        readiness_snapshot,
        supervisor_snapshot,
        &progress,
        selected_lane.as_ref(),
        accepted_queue_pressure,
    );
    let (lane_lines, compact_lane_lines) =
        build_operations_lane_lines(supervisor_snapshot, selected_slot_id, activity_frame);
    let timeline_lines = build_selected_lane_timeline_lines(selected_lane.as_ref());
    let (selected_lane_lines, compact_selected_lane_lines) =
        build_selected_lane_detail_lines(screen_model, supervisor_snapshot, selected_lane.as_ref());
    let event_stream = ParallelLiveStreamModel::pending_viewport(
        &screen_model.parallel_event_stream_snapshot,
        fallback_parallel_event_stream_lines(screen_model),
    );
    let focused_full_viewport = screen_model.shell_overlay == ShellOverlay::Supersession;
    let key_lines = build_command_hint_lines(
        screen_model.parallel_mode_enabled,
        screen_model.parallel_mode_loading_prompt_indicator_visible,
        readiness_snapshot.is_some_and(|snapshot| snapshot.allows_parallel_mode()),
        focused_full_viewport,
    );

    SupersessionOverlayView {
        focused_full_viewport,
        header_lines: vec![
            AkraTheme::title_line("Parallel Operations", ""),
            Line::styled(
                format!(
                    "{}  ·  {}  ·  {}",
                    if is_pending_pool_board(&supervisor_snapshot.pool) {
                        format!("{activity_frame} PREPARING")
                    } else if screen_model.parallel_mode_enabled {
                        "ON".to_string()
                    } else {
                        "OFF".to_string()
                    },
                    operations_board_state_label(
                        screen_model.parallel_mode_enabled,
                        screen_model.parallel_mode_control_effect_in_flight,
                        supervisor_snapshot,
                    ),
                    if focused_full_viewport {
                        "focused inspection"
                    } else {
                        "composer available"
                    }
                ),
                AkraTheme::subtle(),
            ),
        ],
        overview_lines,
        accepted_queue_lines,
        timeline_lines,
        lane_lines,
        compact_lane_lines,
        selected_lane_lines,
        compact_selected_lane_lines,
        event_stream,
        key_lines,
    }
}

fn build_command_hint_lines(
    parallel_mode_enabled: bool,
    prompt_input_locked: bool,
    readiness_allows_parallel_mode: bool,
    focused_full_viewport: bool,
) -> Vec<Line<'static>> {
    /*
     * Inline command hints can be clipped to a single body row on compact terminals.
     * Keep the board-level actions on the first row so the visible row never degrades
     * to only "Ctrl+R" while off/close/peek remain hidden below it.
     */
    let primary = if parallel_mode_enabled && focused_full_viewport {
        "Enter inspect  ·  V agent view  ·  Ctrl+R refresh  ·  Ctrl+P off  ·  Esc close"
    } else if parallel_mode_enabled {
        "Ctrl+O board  ·  :parallel refresh  ·  :peek agents  ·  :parallel off"
    } else if readiness_allows_parallel_mode {
        "Ctrl+R refresh  ·  :parallel enable  ·  Ctrl+O/Esc/Ctrl+C close"
    } else {
        "Ctrl+R refresh  ·  fix readiness then :parallel  ·  Ctrl+O/Esc/Ctrl+C close"
    };
    let secondary = if focused_full_viewport && prompt_input_locked {
        "Tab section  ·  ↑↓ select  ·  Enter/Space inspect"
    } else if focused_full_viewport {
        "Tab section  ·  ↑↓ select  ·  Enter/Space inspect  ·  V opens agent picker"
    } else if prompt_input_locked {
        "Parallel setup in progress  ·  draft preserved  ·  Ctrl+O opens operations"
    } else {
        "Enter sends prompt  ·  Ctrl+O opens operations"
    };

    vec![AkraTheme::key_line(primary), AkraTheme::key_line(secondary)]
}

#[derive(Clone, Copy)]
struct OperationsLane<'a> {
    slot: &'a ParallelModePoolSlotSnapshot,
    roster: Option<&'a ParallelModeAgentRosterEntry>,
    detail: Option<&'a ParallelModeAgentSessionDetailSnapshot>,
    queue_item: Option<&'a ParallelModeDistributorQueueItem>,
    discrepancy: Option<&'static str>,
}

fn operations_lane<'a>(
    snapshot: &'a ParallelModeSupervisorSnapshot,
    slot_id: &str,
) -> Option<OperationsLane<'a>> {
    let slot = snapshot
        .pool
        .slots
        .iter()
        .find(|slot| slot.slot_id == slot_id)?;
    let roster = snapshot
        .roster
        .entries
        .iter()
        .find(|entry| entry.slot_id == slot.slot_id);
    let detail = snapshot
        .detail
        .session_for_lane(&slot.slot_id, roster.map(|entry| entry.agent_id.as_str()));
    let queue_item = queue_item_for_lane(&snapshot.distributor, slot, roster, detail);
    Some(OperationsLane {
        slot,
        roster,
        detail,
        queue_item,
        discrepancy: lane_projection_discrepancy(slot, roster),
    })
}

fn queue_item_for_lane<'a>(
    distributor: &'a ParallelModeDistributorSnapshot,
    slot: &ParallelModePoolSlotSnapshot,
    roster: Option<&ParallelModeAgentRosterEntry>,
    detail: Option<&ParallelModeAgentSessionDetailSnapshot>,
) -> Option<&'a ParallelModeDistributorQueueItem> {
    distributor.queue_items.iter().find(|item| {
        if let Some(identity) = item.identity.as_deref() {
            return detail.is_some_and(|detail| identity.session_key == detail.session_key)
                || slot.owner_identity.as_ref().is_some_and(|owner| {
                    identity.session_key == owner.session_key
                        && identity.slot_id == slot.slot_id
                        && identity.task_id == owner.task_id
                });
        }
        roster.is_some_and(|entry| {
            item.source_agent == entry.agent_id && item.branch_name == entry.branch_name
        })
    })
}

fn lane_projection_discrepancy(
    slot: &ParallelModePoolSlotSnapshot,
    roster: Option<&ParallelModeAgentRosterEntry>,
) -> Option<&'static str> {
    let slot_requires_roster = matches!(
        slot.state,
        ParallelModePoolSlotState::Leased
            | ParallelModePoolSlotState::Running
            | ParallelModePoolSlotState::AwaitingCleanup
    );
    let Some(roster) = roster else {
        return slot_requires_roster.then_some("lease has no roster row");
    };
    if slot.state == ParallelModePoolSlotState::Idle {
        return Some("idle slot still has a roster row");
    }
    match (slot.owner_identity.as_ref(), roster.lease_identity.as_ref()) {
        (Some(owner), Some(lease)) => (owner.agent_id != roster.agent_id
            || owner.task_id != lease.task_id
            || owner.session_key != lease.session_key)
            .then_some("pool and roster identities disagree"),
        (Some(_), None) => Some("roster lease identity is unknown"),
        (None, Some(_)) => Some("pool owner identity is unknown"),
        (None, None) => None,
    }
}

pub(super) fn operations_board_state_label(
    mode_enabled: bool,
    refreshing: bool,
    snapshot: &ParallelModeSupervisorSnapshot,
) -> &'static str {
    if !mode_enabled {
        return "OFF";
    }
    if is_pending_pool_board(&snapshot.pool) {
        return "ENABLING";
    }
    if refreshing {
        return "REFRESHING · showing last snapshot";
    }
    if snapshot.pool.blocked_slots + snapshot.pool.missing_slots + snapshot.pool.unavailable_slots
        > 0
    {
        return "ATTENTION";
    }
    if snapshot.pool.awaiting_cleanup_slots > 0 {
        return "CLEANUP";
    }
    if snapshot.pool.leased_slots + snapshot.pool.running_slots > 0 {
        return "RUNNING";
    }
    "READY"
}

fn build_operations_overview_lines(
    screen_model: &ConversationScreenModel<'_>,
    readiness_snapshot: Option<&crate::domain::parallel_mode::ParallelModeReadinessSnapshot>,
    snapshot: &ParallelModeSupervisorSnapshot,
    progress: &ParallelModeProgressSummary,
    selected_lane: Option<&OperationsLane<'_>>,
    accepted_queue_rows: usize,
) -> Vec<Line<'static>> {
    let active = snapshot.pool.leased_slots + snapshot.pool.running_slots;
    let mut lines = vec![Line::from(vec![
        Span::styled(
            operations_board_state_label(
                screen_model.parallel_mode_enabled,
                screen_model.parallel_mode_control_effect_in_flight,
                snapshot,
            ),
            if progress.attention > 0 {
                AkraTheme::warning()
            } else {
                AkraTheme::brand()
            },
        ),
        Span::raw(format!(
            "  ·  slots {active}/{}  ·  accepted queue {accepted_queue_rows}  ·  delivery {}",
            snapshot.pool.configured_size,
            snapshot.distributor.queue_depth()
        )),
    ])];
    if let Some(blocker) = operations_blocker(screen_model, snapshot, selected_lane) {
        lines.push(Line::from(vec![
            Span::styled("BLOCKER  ", AkraTheme::danger()),
            Span::styled(truncate_timeline_text(&blocker, 112), AkraTheme::warning()),
        ]));
    } else {
        lines.push(Line::styled(
            "No active blocker · unknown facts remain unknown",
            AkraTheme::subtle(),
        ));
    }
    lines.push(Line::styled(
        format!(
            "{}  ·  {}",
            readiness_snapshot
                .map(|readiness| readiness.readiness_label())
                .unwrap_or("readiness unknown"),
            truncate_timeline_text(&snapshot.workspace_path, 104)
        ),
        AkraTheme::muted(),
    ));
    lines
}

fn operations_blocker(
    screen_model: &ConversationScreenModel<'_>,
    snapshot: &ParallelModeSupervisorSnapshot,
    selected_lane: Option<&OperationsLane<'_>>,
) -> Option<String> {
    global_operations_blocker(screen_model, snapshot).or_else(|| {
        selected_lane.and_then(|lane| {
            (lane.discrepancy.is_some()
                || matches!(
                    lane.slot.state,
                    ParallelModePoolSlotState::Blocked
                        | ParallelModePoolSlotState::Missing
                        | ParallelModePoolSlotState::Unavailable
                ))
            .then(|| {
                lane.discrepancy.map_or_else(
                    || lane.slot.worktree_label.clone(),
                    |discrepancy| {
                        format!(
                            "{discrepancy} · {}",
                            truncate_timeline_text(&lane.slot.worktree_label, 88)
                        )
                    },
                )
            })
        })
    })
}

pub(super) fn global_operations_blocker(
    screen_model: &ConversationScreenModel<'_>,
    snapshot: &ParallelModeSupervisorSnapshot,
) -> Option<String> {
    screen_model
        .last_parallel_mode_dispatch_withheld_reason
        .clone()
        .or_else(|| {
            screen_model
                .parallel_mode_readiness
                .as_ref()
                .and_then(|readiness| readiness.top_alert.clone())
        })
        .or_else(|| {
            snapshot
                .distributor
                .orchestrator_status
                .blocked_reason
                .clone()
        })
        .or_else(|| snapshot.distributor.head_blocked_detail.clone())
}

pub(super) fn parallel_projection_has_desync(snapshot: &ParallelModeSupervisorSnapshot) -> bool {
    if is_pending_pool_board(&snapshot.pool) {
        return false;
    }
    let slot_has_disagreement = snapshot.pool.slots.iter().any(|slot| {
        let roster = snapshot
            .roster
            .entries
            .iter()
            .find(|entry| entry.slot_id == slot.slot_id);
        lane_projection_discrepancy(slot, roster).is_some()
    });
    slot_has_disagreement
        || snapshot.roster.entries.iter().any(|entry| {
            !snapshot
                .pool
                .slots
                .iter()
                .any(|slot| slot.slot_id == entry.slot_id)
        })
}

fn build_accepted_queue_lines(
    screen_model: &ConversationScreenModel<'_>,
    snapshot: &ParallelModeSupervisorSnapshot,
) -> (Vec<Line<'static>>, usize) {
    let active_task_ids = snapshot
        .roster
        .entries
        .iter()
        .filter_map(|entry| entry.lease_identity.as_ref())
        .map(|identity| identity.task_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut lines = screen_model
        .planning_runtime_projection
        .queue_projection()
        .into_iter()
        .flat_map(|queue| queue.active_tasks.iter())
        .filter(|task| !active_task_ids.contains(task.task_id.as_str()))
        .map(|task| {
            Line::from(vec![
                Span::styled(format!("#{} DISPATCH  ", task.rank), AkraTheme::accent()),
                Span::raw(truncate_timeline_text(&task.task_title, 70)),
            ])
        })
        .collect::<Vec<_>>();
    lines.extend(
        snapshot
            .distributor
            .queue_items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let style = if matches!(
                    item.queue_state,
                    ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed
                ) {
                    AkraTheme::warning()
                } else {
                    AkraTheme::tool()
                };
                Line::from(vec![
                    Span::styled(format!("D{} DELIVERY  ", index + 1), style),
                    Span::raw(format!(
                        "{}  ·  {}",
                        item.queue_state.label(),
                        truncate_timeline_text(&item.task_title, 62)
                    )),
                ])
            }),
    );
    let pressure = lines.len();
    if pressure == 0 {
        lines.push(Line::styled(
            "No accepted work is waiting; active leases remain in the lane board.",
            AkraTheme::subtle(),
        ));
    }
    (lines, pressure)
}

fn build_operations_lane_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    selected_slot_id: Option<&str>,
    activity_frame: &'static str,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    if snapshot.pool.slots.is_empty() {
        let loading = vec![Line::styled(
            format!("{activity_frame} Waiting for the three-slot pool projection"),
            AkraTheme::muted(),
        )];
        return (loading.clone(), loading);
    }
    let mut lines = Vec::new();
    let mut compact = Vec::new();
    for slot in &snapshot.pool.slots {
        let lane = operations_lane(snapshot, &slot.slot_id)
            .expect("pool slot used to build an operations lane must remain present");
        let selected = selected_slot_id == Some(slot.slot_id.as_str());
        let state_label = lane_state_label(&lane);
        let state_style = lane_state_style(&lane);
        let marker = if selected { "▌" } else { " " };
        let pulse = if lane_is_active(&lane) {
            activity_frame
        } else {
            " "
        };
        let role = lane
            .roster
            .and_then(|entry| entry.role_label.as_deref())
            .unwrap_or("unknown role");
        let agent = lane
            .roster
            .map(|entry| entry.agent_id.as_str())
            .or_else(|| {
                lane.slot
                    .owner_identity
                    .as_ref()
                    .map(|owner| owner.agent_id.as_str())
            })
            .unwrap_or("unknown agent");
        let task = lane
            .roster
            .map(|entry| entry.task_title.as_str())
            .unwrap_or("no leased task");
        let elapsed = lane
            .roster
            .map(|entry| entry.duration_label.as_str())
            .unwrap_or("—");
        let next_gate = lane_next_gate(&lane);
        lines.extend([
            Line::from(vec![
                Span::styled(format!("{marker} {pulse} {}  ", slot.slot_id), state_style),
                Span::styled(state_label, state_style),
                Span::styled(format!("  ·  {elapsed}"), AkraTheme::muted()),
            ]),
            Line::from(format!(
                "    role {role}  ·  agent {}",
                truncate_timeline_text(agent, 38)
            )),
            Line::from(format!("    task {}", truncate_timeline_text(task, 72))),
            Line::styled(
                format!(
                    "    branch {}  ·  worktree {}",
                    truncate_timeline_text(&slot.branch_name, 40),
                    truncate_timeline_text(&slot.worktree_label, 38)
                ),
                AkraTheme::muted(),
            ),
            Line::from(vec![
                Span::styled("    next ", AkraTheme::subtle()),
                Span::styled(next_gate, state_style),
                Span::styled(
                    lane.discrepancy
                        .map(|discrepancy| format!("  ·  {discrepancy}"))
                        .unwrap_or_default(),
                    AkraTheme::warning(),
                ),
            ]),
        ]);
        compact.push(Line::from(vec![
            Span::styled(
                format!(
                    "{}{} {} ",
                    if selected { ">" } else { " " },
                    slot.slot_id,
                    state_label
                ),
                state_style,
            ),
            Span::raw(format!(
                "{} · {} · →{}",
                truncate_timeline_text(role, 16),
                truncate_timeline_text(task, 22),
                next_gate
            )),
        ]));
    }
    for roster in snapshot.roster.entries.iter().filter(|entry| {
        !snapshot
            .pool
            .slots
            .iter()
            .any(|slot| slot.slot_id == entry.slot_id)
    }) {
        let row = format!(
            "! {} UNMAPPED  ·  {}  ·  {}",
            roster.slot_id,
            truncate_timeline_text(&roster.task_title, 42),
            truncate_timeline_text(&roster.branch_name, 44)
        );
        lines.push(Line::styled(row.clone(), AkraTheme::danger()));
        compact.push(Line::styled(row, AkraTheme::danger()));
    }
    (lines, compact)
}

fn lane_is_active(lane: &OperationsLane<'_>) -> bool {
    matches!(
        lane.slot.state,
        ParallelModePoolSlotState::Leased
            | ParallelModePoolSlotState::Running
            | ParallelModePoolSlotState::AwaitingCleanup
    ) || lane
        .queue_item
        .is_some_and(|item| item.queue_state.is_active())
}

fn lane_state_label(lane: &OperationsLane<'_>) -> &'static str {
    if lane.discrepancy.is_some() {
        return "DESYNC";
    }
    if let Some(item) = lane.queue_item {
        return match item.queue_state {
            ParallelModeQueueItemState::Idle => "IDLE",
            ParallelModeQueueItemState::Queued => "DELIVERY",
            ParallelModeQueueItemState::Pushing => "PUSHING",
            ParallelModeQueueItemState::PrPending => "PR",
            ParallelModeQueueItemState::MergePending => "REVIEW",
            ParallelModeQueueItemState::Integrating => "INTEGRATING",
            ParallelModeQueueItemState::Cleaning => "CLEANUP",
            ParallelModeQueueItemState::Done => "DONE",
            ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed => "BLOCKED",
        };
    }
    match lane.slot.state {
        ParallelModePoolSlotState::Idle => "IDLE",
        ParallelModePoolSlotState::Leased => "STARTING",
        ParallelModePoolSlotState::Running => {
            match lane.roster.map(|entry| entry.state_label.as_str()) {
                Some("reported_complete" | "ledger_refreshing") => "VERIFYING",
                Some("commit_ready" | "merge_queued") => "DELIVERY",
                Some("failed" | "official_refresh_recovery_needed") => "BLOCKED",
                _ => "RUNNING",
            }
        }
        ParallelModePoolSlotState::AwaitingCleanup => "CLEANUP",
        ParallelModePoolSlotState::Blocked => "BLOCKED",
        ParallelModePoolSlotState::Missing => "MISSING",
        ParallelModePoolSlotState::Unavailable => "UNAVAILABLE",
    }
}

fn lane_state_style(lane: &OperationsLane<'_>) -> ratatui::style::Style {
    if lane.discrepancy.is_some()
        || matches!(
            lane.slot.state,
            ParallelModePoolSlotState::Blocked
                | ParallelModePoolSlotState::Missing
                | ParallelModePoolSlotState::Unavailable
        )
        || lane.queue_item.is_some_and(|item| {
            matches!(
                item.queue_state,
                ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed
            )
        })
    {
        AkraTheme::warning()
    } else if lane_is_active(lane) {
        AkraTheme::brand()
    } else {
        AkraTheme::subtle()
    }
}

fn build_selected_lane_timeline_lines(lane: Option<&OperationsLane<'_>>) -> Vec<Line<'static>> {
    let Some(lane) = lane else {
        return vec![Line::styled(
            "No lane selected · lifecycle unknown",
            AkraTheme::subtle(),
        )];
    };
    let Some(detail) = lane.detail else {
        return vec![
            Line::styled(
                format!("● now  {}", lane_state_label(lane)),
                lane_state_style(lane),
            ),
            Line::styled(
                "  No exact session history is projected for this lane.",
                AkraTheme::subtle(),
            ),
        ];
    };
    let current_state =
        truncate_timeline_text(&display_supersession_state_label(&detail.state_label), 14);
    let mut lines = vec![Line::from(vec![
        Span::styled("● now ", AkraTheme::brand()),
        Span::styled(current_state.clone(), lane_state_style(lane)),
    ])];
    lines.extend(
        detail
            .history
            .iter()
            .rev()
            .filter(|entry| {
                !(entry.state_label == detail.state_label && entry.timestamp == detail.updated_at)
            })
            .take(12)
            .map(|entry| {
                let state = truncate_timeline_text(
                    &display_supersession_state_label(&entry.state_label),
                    14,
                );
                Line::from(vec![
                    Span::styled(
                        format!("{} ", compact_timestamp_label(&entry.timestamp)),
                        AkraTheme::muted(),
                    ),
                    Span::raw(state),
                ])
            }),
    );
    lines
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeliveryGateStatus {
    Done,
    Active,
    Pending,
    Unknown,
}

struct DeliveryGate {
    label: &'static str,
    status: DeliveryGateStatus,
}

fn build_selected_lane_detail_lines(
    _screen_model: &ConversationScreenModel<'_>,
    _snapshot: &ParallelModeSupervisorSnapshot,
    lane: Option<&OperationsLane<'_>>,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let Some(lane) = lane else {
        let empty = vec![Line::styled(
            "No lane selected · use ↑/↓ in the lane board",
            AkraTheme::subtle(),
        )];
        return (empty.clone(), empty);
    };
    let role = lane
        .roster
        .and_then(|entry| entry.role_label.as_deref())
        .unwrap_or("unknown");
    let agent = lane
        .roster
        .map(|entry| entry.agent_id.as_str())
        .or_else(|| {
            lane.slot
                .owner_identity
                .as_ref()
                .map(|owner| owner.agent_id.as_str())
        })
        .unwrap_or("unknown");
    let task = lane
        .roster
        .map(|entry| entry.task_title.as_str())
        .unwrap_or("unknown");
    let elapsed = lane
        .roster
        .map(|entry| entry.duration_label.as_str())
        .unwrap_or("unknown");
    let gates = delivery_gates(lane);
    let next_gate = next_delivery_gate(&gates);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{}  ", lane.slot.slot_id), lane_state_style(lane)),
            Span::styled(lane_state_label(lane), lane_state_style(lane)),
        ]),
        Line::from(format!("role      {role}")),
        Line::from(format!("task      {}", truncate_timeline_text(task, 62))),
        Line::from(format!("agent     {}", truncate_timeline_text(agent, 62))),
        Line::from(format!(
            "branch    {}",
            truncate_timeline_text(&lane.slot.branch_name, 62)
        )),
        Line::from(format!(
            "worktree  {}",
            truncate_timeline_text(&lane.slot.worktree_label, 62)
        )),
        Line::from(format!("activity  {elapsed}  ·  next {next_gate}")),
        Line::styled("DELIVERY GATES", AkraTheme::accent()),
    ];
    lines.extend(gates.iter().map(delivery_gate_line));
    let compact = vec![
        Line::from(vec![
            Span::styled(
                format!("{} {} ", lane.slot.slot_id, lane_state_label(lane)),
                lane_state_style(lane),
            ),
            Span::raw(format!("{role} · →{next_gate}")),
        ]),
        Line::from(format!(
            "{} · {} · {}",
            truncate_timeline_text(task, 20),
            truncate_timeline_text(agent, 12),
            elapsed.split_whitespace().collect::<String>()
        )),
        Line::styled(
            format!(
                "branch {}  ·  wt {}",
                truncate_timeline_text(&lane.slot.branch_name, 13),
                truncate_timeline_text(&lane.slot.worktree_label, 9)
            ),
            AkraTheme::muted(),
        ),
        compact_delivery_gate_line(&gates[..4]),
        compact_delivery_gate_line(&gates[4..]),
    ];
    (lines, compact)
}

fn delivery_gates(lane: &OperationsLane<'_>) -> Vec<DeliveryGate> {
    let mut states = BTreeSet::new();
    if let Some(detail) = lane.detail {
        states.extend(
            detail
                .history
                .iter()
                .map(|entry| entry.state_label.as_str()),
        );
        states.insert(detail.state_label.as_str());
        states.insert(detail.completion_state_label.as_str());
    }
    let queue_state = lane.queue_item.map(|item| item.queue_state);
    let known_pipeline = lane.roster.is_some() || lane.detail.is_some() || queue_state.is_some();
    let commit_done = state_seen(
        &states,
        &[
            "commit_ready",
            "merge_queued",
            "pushing",
            "pr_pending",
            "merge_pending",
            "integrating",
            "merged",
            "cleanup_pending",
            "cleaned",
        ],
    ) || queue_state.is_some();
    let validation_active = state_seen(&states, &["reported_complete", "ledger_refreshing"]);
    let commit_active = state_seen(&states, &["ledger_refreshing"]) && !commit_done;
    let pr_active = queue_state == Some(ParallelModeQueueItemState::PrPending)
        || state_seen(&states, &["pr_pending"]);
    let pr_done = matches!(
        queue_state,
        Some(
            ParallelModeQueueItemState::MergePending
                | ParallelModeQueueItemState::Integrating
                | ParallelModeQueueItemState::Cleaning
                | ParallelModeQueueItemState::Done
        )
    ) || state_seen(
        &states,
        &[
            "merge_pending",
            "integrating",
            "merged",
            "cleanup_pending",
            "cleaned",
        ],
    );
    let review_active = queue_state == Some(ParallelModeQueueItemState::MergePending)
        || state_seen(&states, &["merge_pending"]);
    let review_done = matches!(
        queue_state,
        Some(
            ParallelModeQueueItemState::Integrating
                | ParallelModeQueueItemState::Cleaning
                | ParallelModeQueueItemState::Done
        )
    ) || state_seen(
        &states,
        &["integrating", "merged", "cleanup_pending", "cleaned"],
    );
    let integration_active = queue_state == Some(ParallelModeQueueItemState::Integrating)
        || state_seen(&states, &["integrating"]);
    let integration_done = matches!(
        queue_state,
        Some(ParallelModeQueueItemState::Cleaning | ParallelModeQueueItemState::Done)
    ) || state_seen(&states, &["merged", "cleanup_pending", "cleaned"]);
    let cleanup_active = queue_state == Some(ParallelModeQueueItemState::Cleaning)
        || state_seen(&states, &["merged", "cleanup_pending"]);
    let cleanup_done =
        queue_state == Some(ParallelModeQueueItemState::Done) || state_seen(&states, &["cleaned"]);
    let status = |done: bool, active: bool| {
        if done {
            DeliveryGateStatus::Done
        } else if active {
            DeliveryGateStatus::Active
        } else if known_pipeline {
            DeliveryGateStatus::Pending
        } else {
            DeliveryGateStatus::Unknown
        }
    };
    vec![
        DeliveryGate {
            label: "Commit",
            status: status(commit_done, commit_active),
        },
        DeliveryGate {
            label: "Validation",
            status: status(commit_done, validation_active),
        },
        DeliveryGate {
            label: "PR",
            status: status(pr_done, pr_active),
        },
        DeliveryGate {
            label: "Review",
            status: status(review_done, review_active),
        },
        DeliveryGate {
            label: "Integration",
            status: status(integration_done, integration_active),
        },
        DeliveryGate {
            label: "Remote verify",
            status: status(integration_done, integration_active),
        },
        DeliveryGate {
            label: "Cleanup",
            status: status(cleanup_done, cleanup_active),
        },
    ]
}

fn state_seen(states: &BTreeSet<&str>, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| states.contains(candidate))
}

fn next_delivery_gate(gates: &[DeliveryGate]) -> &'static str {
    gates
        .iter()
        .find(|gate| gate.status == DeliveryGateStatus::Active)
        .or_else(|| {
            gates
                .iter()
                .find(|gate| gate.status == DeliveryGateStatus::Pending)
        })
        .map(|gate| gate.label)
        .unwrap_or_else(|| {
            if gates
                .iter()
                .all(|gate| gate.status == DeliveryGateStatus::Done)
            {
                "complete"
            } else {
                "unknown"
            }
        })
}

fn lane_next_gate(lane: &OperationsLane<'_>) -> &'static str {
    if lane.discrepancy.is_some()
        || matches!(
            lane.slot.state,
            ParallelModePoolSlotState::Blocked
                | ParallelModePoolSlotState::Missing
                | ParallelModePoolSlotState::Unavailable
        )
    {
        return "operator recovery";
    }
    if lane.slot.state == ParallelModePoolSlotState::Idle {
        return "accepted task";
    }
    next_delivery_gate(&delivery_gates(lane))
}

fn delivery_gate_line(gate: &DeliveryGate) -> Line<'static> {
    let (marker, label, style) = match gate.status {
        DeliveryGateStatus::Done => ("✓", "done", AkraTheme::success()),
        DeliveryGateStatus::Active => ("▶", "active", AkraTheme::brand()),
        DeliveryGateStatus::Pending => ("·", "pending", AkraTheme::muted()),
        DeliveryGateStatus::Unknown => ("?", "unknown", AkraTheme::subtle()),
    };
    Line::from(vec![
        Span::styled(format!("{marker} {:<14}", gate.label), style),
        Span::styled(label, style),
    ])
}

fn compact_delivery_gate_line(gates: &[DeliveryGate]) -> Line<'static> {
    let spans = gates
        .iter()
        .enumerate()
        .flat_map(|(index, gate)| {
            let (marker, style) = match gate.status {
                DeliveryGateStatus::Done => ("✓", AkraTheme::success()),
                DeliveryGateStatus::Active => ("▶", AkraTheme::brand()),
                DeliveryGateStatus::Pending => ("·", AkraTheme::muted()),
                DeliveryGateStatus::Unknown => ("?", AkraTheme::subtle()),
            };
            [
                Span::raw(if index == 0 { "" } else { " " }),
                Span::styled(format!("{marker}{}", gate.label), style),
            ]
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

#[cfg(test)]
#[allow(dead_code)]
fn build_pool_lines(
    pool: &ParallelModePoolBoardSnapshot,
    activity_frame: &'static str,
) -> Vec<Line<'static>> {
    build_pool_lines_with_mud(pool, activity_frame, &[])
}

#[cfg(test)]
fn build_pool_lines_with_mud(
    pool: &ParallelModePoolBoardSnapshot,
    activity_frame: &'static str,
    mud_pool_lines: &[String],
) -> Vec<Line<'static>> {
    /*
    Pool state is rendered before roster state because a missing or blocked slot
    explains why a seemingly idle lane cannot accept work. The per-slot rows keep
    branch, worktree, and owner together for quick cleanup decisions.
    */
    if is_pending_pool_board(pool) {
        return vec![
            Line::from(format!("loading pool board {activity_frame}")),
            Line::from(format!("stage: {}", pool.reconcile_status)),
            Line::from(format!("focus: {}", pool.pool_root_label)),
            Line::from("slots: waiting for baseline, leases, and worktree scan"),
        ];
    }

    let mut lines = mud_pool_lines
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    lines.extend([
        Line::from(format!("configured size: {}", pool.configured_size)),
        Line::from(format!(
            "pool root: {}",
            truncate_timeline_text(&pool.pool_root_label, 96)
        )),
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
    ]);
    if pool.exhausted {
        lines.push(Line::from("capacity: exhausted"));
    }
    lines.extend(pool.slots.iter().map(|slot| {
        Line::from(format!(
            "{}: {} / branch {} / worktree {} / owner {}",
            slot.slot_id,
            slot.state.label(),
            truncate_timeline_text(&slot.branch_name, 40),
            truncate_timeline_text(&slot.worktree_label, 48),
            truncate_timeline_text(&slot.owner_label, 40)
        ))
    }));
    lines
}

#[cfg(test)]
fn build_roster_lines(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    activity_frame: &'static str,
) -> Vec<Line<'static>> {
    build_roster_lines_with_mud(supervisor_snapshot, activity_frame, &[])
}

#[cfg(test)]
fn build_roster_lines_with_mud(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    activity_frame: &'static str,
    mud_roster_lines: &[String],
) -> Vec<Line<'static>> {
    let roster = &supervisor_snapshot.roster;
    if is_pending_pool_board(&supervisor_snapshot.pool) {
        return vec![
            Line::from(format!("loading agent roster {activity_frame}")),
            Line::from(format!("state: {}", supervisor_snapshot.state_label())),
            Line::from(format!("stage: {}", roster.empty_state)),
            Line::from("row shape: agent / task / slot / branch / state / age / summary"),
        ];
    }

    let mut lines = mud_roster_lines
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    lines.extend([
        Line::from(format!("active count: {}", roster.active_count())),
        Line::from(format!("state: {}", supervisor_snapshot.state_label())),
    ]);
    if roster.entries.is_empty() {
        /*
        The empty roster still teaches the expected row shape. That keeps the popup
        useful immediately after enabling parallel mode, before any slot is leased.
        */
        lines.push(Line::from(format!("placeholder: {}", roster.empty_state)));
        lines.push(Line::from(
            "expected row: agent / task / slot / branch / state / age / summary",
        ));
        return lines;
    }

    // Roster entries come from live agent sessions; joining slot health here keeps
    // each row actionable when a worktree is missing, blocked, or unavailable.
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
            "{} {}: {} / {} / {} / {} / {} / {} / {}",
            activity_frame,
            entry.agent_id,
            truncate_timeline_text(&entry.task_title, 36),
            entry.slot_id,
            truncate_timeline_text(&entry.branch_name, 40),
            state_label,
            duration_label,
            truncate_timeline_text(&entry.latest_summary, 72),
            slot_health
        ))
    }));
    lines
}

#[cfg(test)]
fn build_detail_lines(supervisor_snapshot: &ParallelModeSupervisorSnapshot) -> Vec<Line<'static>> {
    build_detail_lines_with_mud(supervisor_snapshot, &[])
}

#[cfg(test)]
fn build_detail_lines_with_mud(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    mud_detail_lines: &[String],
) -> Vec<Line<'static>> {
    let Some(detail) = supervisor_snapshot.detail.session.as_ref() else {
        /*
        Detail falls back to board-level state instead of inventing a selected
        session. That prevents stale agent data from lingering after the supervisor
        has no active or recently completed session to inspect.
        */
        let mut lines = mud_detail_lines
            .iter()
            .map(|line| Line::from(line.clone()))
            .collect::<Vec<_>>();
        lines.extend([
            Line::from("selection: none"),
            Line::from(format!(
                "board state: {}",
                supervisor_snapshot.state_label()
            )),
            Line::from(format!(
                "detail state: {}",
                supervisor_snapshot.detail.empty_state
            )),
            Line::from("timeline: no selected session history"),
        ]);
        return lines;
    };

    // Detail focuses on the selected running or recently completed agent and keeps
    // the official ledger refresh outcome next to distributor handoff status.
    let mut lines = vec![Line::from(format!(
        "timeline: {} / {}",
        detail.slot_id, detail.session_key
    ))];
    lines.extend(build_timeline_lines(detail));
    lines.extend(mud_detail_lines.iter().map(|line| Line::from(line.clone())));
    lines.extend([
        Line::from(format!(
            "selection: {} / {} / {}",
            detail.agent_id,
            detail.slot_id,
            display_supersession_state_label(&detail.state_label)
        )),
        Line::from(format!(
            "task: {} / {}",
            detail.task_id,
            truncate_timeline_text(&detail.task_title, 56)
        )),
        Line::from(format!(
            "thread: {}",
            detail.thread_id.as_deref().unwrap_or("not captured yet")
        )),
        Line::from(format!(
            "slot health: {}",
            slot_health_summary(supervisor_snapshot, &detail.slot_id)
        )),
        Line::from(format!(
            "worktree: {}",
            truncate_timeline_text(&detail.worktree_path, 96)
        )),
        Line::from(format!(
            "branch: {}",
            truncate_timeline_text(&detail.branch_name, 72)
        )),
        Line::from(format!("lease start: {}", detail.lease_started_at)),
        Line::from(format!(
            "completion: {}",
            display_supersession_state_label(&detail.completion_state_label)
        )),
        Line::from(format!(
            "latest: {}",
            truncate_timeline_text(&detail.latest_summary, 96)
        )),
        Line::from(format!(
            "validation: {}",
            truncate_timeline_text(&detail.validation_summary, 96)
        )),
        Line::from(format!(
            "ledger refresh: {}",
            truncate_timeline_text(&detail.authority_refresh_outcome, 96)
        )),
        Line::from(format!(
            "distributor: {}",
            truncate_timeline_text(
                detail
                    .distributor_outcome
                    .as_deref()
                    .unwrap_or("no distributor outcome recorded"),
                96
            )
        )),
    ]);
    lines.push(Line::from("history:"));
    lines.extend(detail.history.iter().map(|entry| {
        Line::from(format!(
            "{} / {} / {}",
            entry.timestamp,
            display_supersession_state_label(&entry.state_label),
            truncate_timeline_text(&entry.summary, 96)
        ))
    }));
    lines
}

fn fallback_parallel_event_stream_lines(
    screen_model: &ConversationScreenModel<'_>,
) -> Vec<Line<'static>> {
    if !screen_model
        .parallel_event_stream_snapshot
        .events()
        .is_empty()
    {
        return Vec::new();
    }
    let mut lines = parallel_supervisor_snapshot_stream_lines(
        &screen_model.parallel_mode_supervisor,
        screen_model.tui_language,
    );
    if lines.is_empty() {
        lines.push(Line::from(screen_model.tui_language.no_parallel_events()));
    }
    lines
}

#[cfg(test)]
fn build_timeline_lines(
    detail: &crate::domain::parallel_mode::ParallelModeAgentSessionDetailSnapshot,
) -> Vec<Line<'static>> {
    /*
    The selected-detail panel has only a few visible rows in inline mode, so this
    compact timeline sits before path/ledger fields. Full history stays below it as
    audit evidence, but operators can scan lifecycle chronology without scrolling.
    */
    let events = compact_timeline_events(detail);
    if events.is_empty() {
        return vec![
            Line::from(format!(
                "events: {} {}",
                compact_timestamp_label(&detail.updated_at),
                display_supersession_state_label(&detail.state_label)
            )),
            Line::from(format!(
                "last event: {}",
                truncate_timeline_text(&detail.latest_summary, 96)
            )),
        ];
    }

    let event_flow = events
        .iter()
        .map(|event| format!("{} {}", event.timestamp, event.state_label))
        .collect::<Vec<_>>()
        .join(" -> ");
    let last_event = events
        .last()
        .map(|event| {
            format!(
                "{} {} / {}",
                event.timestamp,
                event.state_label,
                truncate_timeline_text(&event.summary, 96)
            )
        })
        .unwrap_or_else(|| "not captured yet".to_string());

    let mut lines = vec![Line::from(format!("events: {event_flow}"))];
    if let Some(delivery_boundary) = delivery_boundary_label(detail) {
        lines.push(Line::from(delivery_boundary));
    }
    lines.push(Line::from(format!("last event: {last_event}")));
    lines
}

#[cfg(test)]
struct SupersessionTimelineEvent {
    state_label: String,
    timestamp: String,
    summary: String,
}

#[cfg(test)]
fn compact_timeline_events(
    detail: &crate::domain::parallel_mode::ParallelModeAgentSessionDetailSnapshot,
) -> Vec<SupersessionTimelineEvent> {
    let mut events = detail
        .history
        .iter()
        .filter(|entry| !entry.state_label.trim().is_empty())
        .map(|entry| SupersessionTimelineEvent {
            state_label: display_supersession_state_label(&entry.state_label),
            timestamp: compact_timestamp_label(&entry.timestamp),
            summary: entry.summary.clone(),
        })
        .collect::<Vec<_>>();

    let current_state = display_supersession_state_label(&detail.state_label);
    let current_timestamp = compact_timestamp_label(&detail.updated_at);
    let current_summary = detail.latest_summary.clone();
    let current_already_recorded = events.last().is_some_and(|event| {
        event.state_label == current_state && event.timestamp == current_timestamp
    });
    if !current_already_recorded {
        events.push(SupersessionTimelineEvent {
            state_label: current_state,
            timestamp: current_timestamp,
            summary: current_summary,
        });
    }

    const MAX_TIMELINE_EVENTS: usize = 6;
    if events.len() > MAX_TIMELINE_EVENTS {
        let drain_count = events.len() - MAX_TIMELINE_EVENTS;
        events.drain(0..drain_count);
        if let Some(first) = events.first_mut() {
            first.state_label = format!("... {}", first.state_label);
        }
    }

    events
}

#[cfg(test)]
struct DeliveryBoundaryStage {
    label: &'static str,
    state_labels: &'static [&'static str],
}

#[cfg(test)]
struct DeliveryBoundaryEvent {
    stage_label: &'static str,
    timestamp: String,
}

#[cfg(test)]
fn delivery_boundary_label(
    detail: &crate::domain::parallel_mode::ParallelModeAgentSessionDetailSnapshot,
) -> Option<String> {
    let events = delivery_boundary_events(detail);
    if events.is_empty() {
        return None;
    }

    Some(format!(
        "delivery: {}",
        events
            .iter()
            .map(|event| format!("{} {}", event.stage_label, event.timestamp))
            .collect::<Vec<_>>()
            .join(" -> ")
    ))
}

#[cfg(test)]
fn delivery_boundary_events(
    detail: &crate::domain::parallel_mode::ParallelModeAgentSessionDetailSnapshot,
) -> Vec<DeliveryBoundaryEvent> {
    let mut source_events = detail
        .history
        .iter()
        .filter(|entry| !entry.state_label.trim().is_empty())
        .map(|entry| (entry.state_label.as_str(), entry.timestamp.as_str()))
        .collect::<Vec<_>>();
    let current_state = detail.state_label.trim();
    if !current_state.is_empty() {
        let current_already_recorded = source_events.last().is_some_and(|(state, timestamp)| {
            *state == current_state && *timestamp == detail.updated_at.as_str()
        });
        if !current_already_recorded {
            source_events.push((current_state, detail.updated_at.as_str()));
        }
    }
    let has_distributor_delivery = source_events.iter().any(|(state_label, _)| {
        ["pushing", "pr_pending", "merge_pending", "integrating"].contains(state_label)
    });
    if !has_distributor_delivery {
        return Vec::new();
    }

    delivery_boundary_stages()
        .iter()
        .filter_map(|stage| {
            source_events
                .iter()
                .find(|(state_label, _)| stage.state_labels.contains(state_label))
                .map(|(_, timestamp)| DeliveryBoundaryEvent {
                    stage_label: stage.label,
                    timestamp: compact_timestamp_label(timestamp),
                })
        })
        .collect()
}

#[cfg(test)]
fn delivery_boundary_stages() -> [DeliveryBoundaryStage; 3] {
    [
        DeliveryBoundaryStage {
            label: "push",
            state_labels: &["pushing"],
        },
        DeliveryBoundaryStage {
            label: "PR",
            state_labels: &["pr_pending", "merge_pending"],
        },
        DeliveryBoundaryStage {
            label: "merge",
            state_labels: &["integrating", "merged", "cleanup_pending", "cleaned"],
        },
    ]
}

#[cfg(test)]
fn build_distributor_lines(distributor: &ParallelModeDistributorSnapshot) -> Vec<Line<'static>> {
    build_distributor_lines_with_mud(distributor, &[])
}

#[cfg(test)]
fn build_distributor_lines_with_mud(
    distributor: &ParallelModeDistributorSnapshot,
    mud_distributor_lines: &[String],
) -> Vec<Line<'static>> {
    /*
    Distributor rows sit after agent detail because they explain what happens once
    an agent has reported completion. The strongest operator signal is the queue
    head: note, blocked detail, rebase provenance, and orchestrator status all
    describe why that head can or cannot advance into the integration baseline.
    */
    let blocked_head_detail = distributor
        .head_blocked_detail
        .as_deref()
        .map(str::trim)
        .filter(|detail| !detail.is_empty());

    if is_pending_distributor(distributor) {
        return vec![
            Line::from("loading distributor board"),
            Line::from(format!("stage: {}", distributor.head_summary)),
            Line::from(format!("pipeline: {}", distributor.note)),
            Line::from("queue: will appear after dispatch and completion feed scan"),
        ];
    }

    let mut lines = mud_distributor_lines
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    lines.extend([
        Line::from(format!("head: {}", distributor.head_summary)),
        Line::from(format!("queue depth: {}", distributor.queue_depth())),
    ]);

    // `note` and `blocked head` can share the same text; avoid duplicating it in the
    // narrow popup while still surfacing richer blocked-head detail when present.
    if blocked_head_detail != Some(distributor.note.trim()) {
        lines.push(Line::from(format!(
            "note: {}",
            truncate_timeline_text(&distributor.note, 96)
        )));
    }
    if let Some(detail) = blocked_head_detail {
        lines.push(Line::from(format!(
            "blocked head: {}",
            truncate_timeline_text(detail, 96)
        )));
    }
    if let Some(provenance) = distributor.head_rebase_provenance.as_deref() {
        lines.push(Line::from(format!(
            "provenance: {}",
            truncate_timeline_text(provenance, 96)
        )));
    }
    lines.extend(build_orchestrator_lines(distributor));
    if distributor.queue_items.is_empty() {
        lines.push(Line::from(
            "queue: no items are waiting for distributor work",
        ));
    } else {
        /*
        The first queue item is the only one the distributor can act on right now.
        Later rows are deliberately collapsed to the same shape with a weaker label
        so the popup communicates ordering without adding another table widget.
        */
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
                        truncate_timeline_text(&item.task_title, 36),
                        item.queue_state.label(),
                        truncate_timeline_text(&item.branch_name, 40),
                        item.commit_short_sha,
                        truncate_timeline_text(&item.integration_note, 72)
                    ))
                }),
        );
    }
    lines.push(Line::from("completion feed:"));
    /*
    The completion feed is a short audit trail from the distributor snapshot. It is
    appended after the queue because it is supporting evidence, not the next action.
    */
    lines.extend(distributor.completion_feed.iter().map(|entry| {
        Line::from(format!(
            "{}: {}",
            entry.stage_label,
            truncate_timeline_text(&entry.summary, 96)
        ))
    }));
    lines.push(Line::from("runtime events:"));
    if distributor.runtime_event_feed.is_empty() {
        lines.push(Line::from("events: no runtime events captured yet"));
    } else {
        lines.extend(distributor.runtime_event_feed.iter().map(|entry| {
            Line::from(format!(
                "event #{} @ {} / {}:{} / {} / rev {} / {}",
                entry.sequence,
                compact_timestamp_label(&entry.recorded_at),
                display_runtime_event_label(&entry.projection_kind),
                entry.projection_key,
                display_runtime_event_label(&entry.event_kind),
                entry.observed_planning_revision,
                truncate_timeline_text(&entry.summary, 88)
            ))
        }));
    }
    lines
}

fn is_pending_pool_board(pool: &ParallelModePoolBoardSnapshot) -> bool {
    pool.pool_root_label.starts_with("loading:")
}

#[cfg(test)]
fn is_pending_distributor(distributor: &ParallelModeDistributorSnapshot) -> bool {
    distributor.queue_items.is_empty()
        && distributor.completion_feed.is_empty()
        && distributor.runtime_event_feed.is_empty()
        && (distributor.head_summary.starts_with("waiting ")
            || distributor.head_summary.contains("progress")
            || distributor.head_summary.contains("refreshing"))
}

fn supersession_activity_frame(animation_elapsed_millis: u128) -> &'static str {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    FRAMES[((animation_elapsed_millis / 250) as usize) % FRAMES.len()]
}

#[cfg(test)]
fn build_orchestrator_lines(distributor: &ParallelModeDistributorSnapshot) -> Vec<Line<'static>> {
    let status = &distributor.orchestrator_status;
    /*
    Orchestrator status is the distributor-to-worktree boundary. Holding conflict
    files, barrier state, and slot-return wait reasons together makes it clear when
    capacity is withheld intentionally until integration recovery finishes.
    */
    let mut lines = vec![
        Line::from(format!(
            "orchestrator head: {}",
            truncate_timeline_text(&status.queue_head, 96)
        )),
        Line::from(format!(
            "orchestrator barrier: {}",
            truncate_timeline_text(&status.barrier_state, 96)
        )),
        Line::from(format!(
            "orchestrator held queue: {}",
            status.held_queue_count
        )),
        Line::from(format!(
            "integration worktree: {}",
            truncate_timeline_text(&status.integration_worktree_readiness, 96)
        )),
    ];
    if let Some(reason) = status.blocked_reason.as_deref() {
        lines.push(Line::from(format!(
            "blocked reason: {}",
            truncate_timeline_text(reason, 96)
        )));
    }
    if !status.conflict_files.is_empty() {
        lines.push(Line::from(format!(
            "conflict files: {}",
            status.conflict_files.join(", ")
        )));
    }
    if let Some(reason) = status.slot_return_wait_reason.as_deref() {
        lines.push(Line::from(format!(
            "slot return: {}",
            truncate_timeline_text(reason, 96)
        )));
    }
    lines
}

fn display_supersession_state_label(state_label: &str) -> String {
    /*
    Domain labels are precise but too lifecycle-specific for the popup. The control
    tower keeps the distinction operators need: reported means agent-owned, official
    means accepted by the ledger/distributor flow.
    */
    match state_label {
        "reported_complete" => "reported".to_string(),
        "commit_ready" => "official".to_string(),
        other => other.replace('_', " "),
    }
}

#[cfg(test)]
fn display_runtime_event_label(label: &str) -> String {
    label.replace('_', " ")
}

fn compact_timestamp_label(timestamp: &str) -> String {
    let trimmed = timestamp.trim();
    if trimmed.is_empty() {
        return "time?".to_string();
    }

    let time_part = trimmed
        .split_once('T')
        .map(|(_, time)| time)
        .unwrap_or(trimmed)
        .trim_end_matches('Z');

    let mut parts = time_part.split(':');
    let Some(hour) = parts.next() else {
        return trimmed.to_string();
    };
    let Some(minute) = parts.next() else {
        return trimmed.to_string();
    };
    format!("{hour}:{minute}")
}

fn truncate_timeline_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let keep = max_chars.saturating_sub(3);
    let mut truncated = trimmed.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
fn display_roster_duration_label(state_label: &str, duration_label: &str) -> String {
    /*
    Duration only gets a verb for actively running rows. Completed or blocked rows
    already carry status-heavy labels, so preserving their raw duration avoids
    implying that work is still progressing.
    */
    let trimmed_duration = duration_label.trim();
    if state_label == "running" && !trimmed_duration.is_empty() {
        return format!("working {trimmed_duration}");
    }

    trimmed_duration.to_string()
}

#[cfg(test)]
fn slot_health_summary(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    slot_id: &str,
) -> String {
    /*
    Detail and roster rows both consult the pool board for slot health. Agent
    snapshots should not duplicate worktree reconciliation, and missing slot rows
    must still be visible even when the agent session itself looks healthy.
    */
    supervisor_snapshot
        .pool
        .slots
        .iter()
        .find(|slot| slot.slot_id == slot_id)
        .map(slot_health_summary_from_slot)
        .unwrap_or_else(|| "slot not projected".to_string())
}

#[cfg(test)]
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

#[cfg(test)]
fn worktree_health_detail(worktree_label: &str) -> String {
    /*
    Pool worktree labels often use "path / diagnosis". The popup keeps the
    diagnosis for unhealthy slots because the path already appears in the slot row
    and the repair hint is the higher-value signal.
    */
    worktree_label
        .rsplit_once(" / ")
        .map(|(_, detail)| detail.trim())
        .filter(|detail| !detail.is_empty())
        .unwrap_or(worktree_label.trim())
        .to_string()
}

#[cfg(test)]
#[path = "supersession/tests.rs"]
mod tests;
