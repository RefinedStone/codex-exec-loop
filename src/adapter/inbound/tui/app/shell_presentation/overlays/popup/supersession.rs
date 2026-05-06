use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::text::Line;

use crate::domain::parallel_mode::{
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeSupervisorSnapshot,
};

use super::super::super::super::{AkraTheme, NativeTuiApp};
use super::SupersessionOverlayView;

/* Supersession is the operator board for parallel mode. It intentionally keeps
 * readiness, pool capacity, active roster, selected detail, and distributor state
 * as separate line groups so the popup can answer "can work start?", "who is
 * running?", and "why is integration blocked?" without requiring navigation.
 */
pub(crate) fn build_supersession_overlay_view(app: &NativeTuiApp) -> SupersessionOverlayView {
    let mode_label = if app.parallel_mode_enabled() {
        "parallel"
    } else {
        "normal"
    };
    let readiness_snapshot = app.parallel_mode_readiness_snapshot();
    let supervisor_snapshot = app.parallel_mode_supervisor_snapshot();
    let activity_frame = supersession_activity_frame();
    /*
    The app state remains the source of truth for live readiness and supervisor
    snapshots. This adapter only chooses popup grouping and copy, so service-layer
    invariants such as queue ordering, pool reconciliation, and official completion
    refresh stay testable outside ratatui rendering.
    */
    let summary_lines = build_summary_lines(
        mode_label,
        readiness_snapshot,
        &supervisor_snapshot,
        activity_frame,
    );
    let capability_lines =
        build_capability_lines(readiness_snapshot, &supervisor_snapshot, activity_frame);
    let pool_lines = build_pool_lines(&supervisor_snapshot.pool, activity_frame);
    let roster_lines = build_roster_lines(&supervisor_snapshot, activity_frame);
    let detail_lines = build_detail_lines(&supervisor_snapshot);
    let distributor_lines = build_distributor_lines(&supervisor_snapshot.distributor);
    let mut key_lines = vec![AkraTheme::key_line("Ctrl+R: rerun readiness")];

    if app.parallel_mode_enabled() {
        key_lines.push(AkraTheme::key_line("Ctrl+P: parallel off"));
    } else if readiness_snapshot.is_some_and(|snapshot| snapshot.allows_parallel_mode()) {
        key_lines.push(AkraTheme::key_line("next action: type :parallel"));
    } else {
        key_lines.push(AkraTheme::key_line(
            "next action: fix readiness blockers, then type :parallel",
        ));
    }
    key_lines.push(AkraTheme::key_line("Ctrl+O or Esc/Ctrl+C: close"));

    SupersessionOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Supersession Control Tower", " / supervisor board"),
            Line::from(format!(
                "activity {activity_frame} / {}",
                if app.parallel_mode_prompt_input_locked() {
                    "prompt locked while parallel loading is active"
                } else {
                    "prompt available"
                }
            )),
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
    activity_frame: &'static str,
) -> Vec<Line<'static>> {
    /*
    Summary lines are the popup's triage header: readiness tells whether parallel
    mode can be enabled, pool and roster summaries show dispatch capacity, and the
    distributor compact summary shows whether completed work is stuck downstream.
    */
    let mut lines = vec![
        Line::from(format!(
            "activity: {}  |  prompt: locked  |  workers: {}",
            activity_frame,
            if supervisor_snapshot.roster.active_count() > 0 {
                "running"
            } else if is_pending_pool_board(&supervisor_snapshot.pool) {
                "loading"
            } else {
                "idle"
            }
        )),
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
            pool_summary_label(&supervisor_snapshot.pool)
        )),
        Line::from(format!(
            "agents: {}  |  queue: {}",
            roster_summary_label(supervisor_snapshot),
            distributor_summary_label(&supervisor_snapshot.distributor)
        )),
    ];
    if let Some(alert) = readiness_snapshot.and_then(|snapshot| snapshot.top_alert.as_deref()) {
        lines.push(Line::from(format!("alert: {alert}")));
    } else if let Some(notice) = supervisor_snapshot.top_notice.as_deref() {
        lines.push(Line::from(format!("notice: {notice}")));
    }

    lines
}

fn build_capability_lines(
    readiness_snapshot: Option<&crate::domain::parallel_mode::ParallelModeReadinessSnapshot>,
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    activity_frame: &'static str,
) -> Vec<Line<'static>> {
    if let Some(snapshot) = readiness_snapshot {
        return snapshot
            .capabilities
            .iter()
            .map(|capability| Line::from(capability.summary()))
            .collect::<Vec<_>>();
    }

    vec![
        Line::from(format!("loading pipeline {activity_frame}")),
        Line::from(format!("readiness: running {activity_frame}")),
        Line::from("pool reconcile: next"),
        Line::from("board refresh: next"),
        Line::from(format!(
            "stage: {}",
            supervisor_snapshot
                .top_notice
                .as_deref()
                .unwrap_or("parallel preparation is starting")
        )),
    ]
}

fn pool_summary_label(pool: &ParallelModePoolBoardSnapshot) -> String {
    if is_pending_pool_board(pool) {
        return pool.reconcile_status.clone();
    }

    pool.compact_summary()
}

fn roster_summary_label(supervisor_snapshot: &ParallelModeSupervisorSnapshot) -> String {
    if is_pending_pool_board(&supervisor_snapshot.pool) {
        return "pending".to_string();
    }

    supervisor_snapshot.roster.compact_summary()
}

fn distributor_summary_label(distributor: &ParallelModeDistributorSnapshot) -> String {
    if is_pending_distributor(distributor) {
        return distributor.head_summary.clone();
    }

    distributor.compact_summary()
}

fn build_pool_lines(
    pool: &ParallelModePoolBoardSnapshot,
    activity_frame: &'static str,
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

fn build_roster_lines(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    activity_frame: &'static str,
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

    let mut lines = vec![
        Line::from(format!("active count: {}", roster.active_count())),
        Line::from(format!("state: {}", supervisor_snapshot.state_label())),
    ];
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
        /*
        Detail falls back to board-level state instead of inventing a selected
        session. That prevents stale agent data from lingering after the supervisor
        has no active or recently completed session to inspect.
        */
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

    // Detail focuses on the selected running or recently completed agent and keeps
    // the official ledger refresh outcome next to distributor handoff status.
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

    let mut lines = vec![
        Line::from(format!("head: {}", distributor.head_summary)),
        Line::from(format!("queue depth: {}", distributor.queue_depth())),
    ];

    // `note` and `blocked head` can share the same text; avoid duplicating it in the
    // narrow popup while still surfacing richer blocked-head detail when present.
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
    /*
    The completion feed is a short audit trail from the distributor snapshot. It is
    appended after the queue because it is supporting evidence, not the next action.
    */
    lines.extend(
        distributor
            .completion_feed
            .iter()
            .map(|entry| Line::from(format!("{}: {}", entry.stage_label, entry.summary))),
    );
    lines
}

fn is_pending_pool_board(pool: &ParallelModePoolBoardSnapshot) -> bool {
    pool.pool_root_label.starts_with("loading:")
}

fn is_pending_distributor(distributor: &ParallelModeDistributorSnapshot) -> bool {
    distributor.queue_items.is_empty()
        && distributor.completion_feed.is_empty()
        && (distributor.head_summary.starts_with("waiting ")
            || distributor.head_summary.contains("progress")
            || distributor.head_summary.contains("refreshing"))
}

fn supersession_activity_frame() -> &'static str {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    FRAMES[((millis / 250) as usize) % FRAMES.len()]
}

fn build_orchestrator_lines(distributor: &ParallelModeDistributorSnapshot) -> Vec<Line<'static>> {
    let status = &distributor.orchestrator_status;
    /*
    Orchestrator status is the distributor-to-worktree boundary. Holding conflict
    files, barrier state, and slot-return wait reasons together makes it clear when
    capacity is withheld intentionally until integration recovery finishes.
    */
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
