use crate::domain::parallel_mode::{
    ParallelModeDistributorSnapshot, ParallelModePoolSlotState, ParallelModeSupervisorSnapshot,
};
use crate::domain::planning::PriorityQueueProjection;
use std::collections::BTreeSet;
use std::ops::Range;

const LINE_LIMIT: usize = 112;
const FIELD_LIMIT: usize = 34;
const SUMMARY_LIMIT: usize = 56;
const PANEL_LINE_LIMIT: usize = 9;
const PANEL_TITLE_LIMIT: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeProgressSummary {
    pub working: usize,
    pub starting: usize,
    pub delivering: usize,
    pub queued: usize,
    pub available: usize,
    pub attention: usize,
    pub syncing: bool,
    pub next_task_title: Option<String>,
}

impl ParallelModeProgressSummary {
    pub fn compact_line(&self) -> String {
        let mut stages = Vec::new();
        if self.working > 0 {
            stages.push(format!("● {} working", self.working));
        }
        if self.starting > 0 {
            stages.push(format!("◐ {} starting", self.starting));
        }
        if self.delivering > 0 {
            stages.push(format!("◆ {} delivery", self.delivering));
        }
        if self.queued > 0 {
            stages.push(format!("○ {} queued", self.queued));
        }
        if self.syncing && stages.is_empty() {
            stages.push("◐ syncing".to_string());
        }
        if self.available > 0 {
            stages.push(format!("{} available", self.available));
        }
        if self.attention > 0 {
            stages.push(format!("! {} attention", self.attention));
        }
        if stages.is_empty() {
            stages.push("ready".to_string());
        }
        stages.join("  ·  ")
    }
}

pub fn parallel_mode_progress_summary(
    snapshot: &ParallelModeSupervisorSnapshot,
    queue_projection: Option<&PriorityQueueProjection>,
    syncing: bool,
) -> ParallelModeProgressSummary {
    let active_task_ids = snapshot
        .roster
        .entries
        .iter()
        .filter_map(|entry| entry.lease_identity.as_ref())
        .map(|identity| identity.task_id.as_str())
        .collect::<BTreeSet<_>>();
    let waiting_tasks = queue_projection
        .into_iter()
        .flat_map(|projection| projection.active_tasks.iter())
        .filter(|task| !active_task_ids.contains(task.task_id.as_str()))
        .collect::<Vec<_>>();

    let (mut working, mut starting, mut delivering) = (0, 0, 0);
    for entry in snapshot
        .roster
        .entries
        .iter()
        .filter(|entry| entry.counts_as_active())
    {
        if snapshot
            .distributor
            .queue_items
            .iter()
            .any(|item| item.source_agent == entry.agent_id)
        {
            delivering += 1;
            continue;
        }
        match lifecycle_progress_label(&entry.state_label) {
            "running" => working += 1,
            "assigned" => starting += 1,
            "blocked" | "cleaned" => {}
            "reported" | "official" | "delivery" => delivering += 1,
            _ => {}
        }
    }
    if snapshot.roster.entries.is_empty() {
        working = snapshot.pool.running_slots;
        starting = snapshot.pool.leased_slots;
    }
    delivering = delivering.max(snapshot.distributor.queue_depth());

    ParallelModeProgressSummary {
        working,
        starting,
        delivering,
        queued: waiting_tasks.len(),
        available: snapshot.pool.idle_slots,
        attention: snapshot.pool.blocked_slots
            + snapshot.pool.missing_slots
            + snapshot.pool.unavailable_slots,
        syncing,
        next_task_title: waiting_tasks
            .first()
            .map(|task| task.task_title.trim().to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupersessionMudFocusZone {
    RealmMap,
    Actors,
    QuestLog,
    ExitCorridor,
}

impl SupersessionMudFocusZone {
    fn next(self) -> Self {
        match self {
            Self::RealmMap => Self::Actors,
            Self::Actors => Self::QuestLog,
            Self::QuestLog => Self::ExitCorridor,
            Self::ExitCorridor => Self::RealmMap,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::RealmMap => Self::ExitCorridor,
            Self::Actors => Self::RealmMap,
            Self::QuestLog => Self::Actors,
            Self::ExitCorridor => Self::QuestLog,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersessionMudUiState {
    focused_zone: SupersessionMudFocusZone,
    selected_room_index: usize,
    selected_actor_index: usize,
    selected_quest_index: usize,
}

impl Default for SupersessionMudUiState {
    fn default() -> Self {
        Self {
            focused_zone: SupersessionMudFocusZone::RealmMap,
            selected_room_index: 0,
            selected_actor_index: 0,
            selected_quest_index: 0,
        }
    }
}

impl SupersessionMudUiState {
    pub fn focused_zone(&self) -> SupersessionMudFocusZone {
        self.focused_zone
    }

    pub fn selected_room_index(&self) -> usize {
        self.selected_room_index
    }

    pub fn selected_actor_index(&self) -> usize {
        self.selected_actor_index
    }

    pub fn selected_quest_index(&self) -> usize {
        self.selected_quest_index
    }

    pub fn focus_next_zone(&mut self) {
        self.focused_zone = self.focused_zone.next();
    }

    pub fn focus_previous_zone(&mut self) {
        self.focused_zone = self.focused_zone.previous();
    }

    pub fn move_selection(&mut self, snapshot: &ParallelModeSupervisorSnapshot, delta: isize) {
        match self.focused_zone {
            SupersessionMudFocusZone::RealmMap => {
                self.selected_room_index =
                    moved_index(self.selected_room_index, snapshot.pool.slots.len(), delta);
            }
            SupersessionMudFocusZone::Actors | SupersessionMudFocusZone::QuestLog => {
                self.selected_actor_index = moved_index(
                    self.selected_actor_index,
                    snapshot.roster.entries.len(),
                    delta,
                );
            }
            SupersessionMudFocusZone::ExitCorridor => {
                self.selected_quest_index = moved_index(
                    self.selected_quest_index,
                    snapshot.distributor.queue_items.len(),
                    delta,
                );
            }
        }
        self.clamp_to_snapshot(snapshot);
    }

    pub fn inspect_focused(&mut self, snapshot: &ParallelModeSupervisorSnapshot) {
        match self.focused_zone {
            SupersessionMudFocusZone::RealmMap => {
                if let Some(slot) = snapshot.pool.slots.get(self.selected_room_index)
                    && let Some(actor_index) = snapshot
                        .roster
                        .entries
                        .iter()
                        .position(|entry| entry.slot_id == slot.slot_id)
                {
                    self.selected_actor_index = actor_index;
                    self.focused_zone = SupersessionMudFocusZone::QuestLog;
                }
            }
            SupersessionMudFocusZone::Actors => {
                self.focused_zone = SupersessionMudFocusZone::QuestLog;
            }
            SupersessionMudFocusZone::QuestLog => {
                self.focused_zone = SupersessionMudFocusZone::ExitCorridor;
            }
            SupersessionMudFocusZone::ExitCorridor => {
                self.focused_zone = SupersessionMudFocusZone::RealmMap;
            }
        }
        self.clamp_to_snapshot(snapshot);
    }

    pub fn clamp_to_snapshot(&mut self, snapshot: &ParallelModeSupervisorSnapshot) {
        self.selected_room_index = self
            .selected_room_index
            .min(snapshot.pool.slots.len().saturating_sub(1));
        self.selected_actor_index = self
            .selected_actor_index
            .min(snapshot.roster.entries.len().saturating_sub(1));
        self.selected_quest_index = self
            .selected_quest_index
            .min(snapshot.distributor.queue_items.len().saturating_sub(1));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersessionMudLines {
    pub summary_lines: Vec<String>,
    pub pool_lines: Vec<String>,
    pub roster_lines: Vec<String>,
    pub detail_lines: Vec<String>,
    pub distributor_lines: Vec<String>,
}

pub fn build_supersession_mud_lines(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
) -> SupersessionMudLines {
    build_supersession_mud_view(supervisor_snapshot, &SupersessionMudUiState::default())
}

pub fn build_supersession_mud_view(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> SupersessionMudLines {
    SupersessionMudLines {
        summary_lines: build_mud_summary_lines(supervisor_snapshot, ui_state),
        pool_lines: build_mud_pool_lines(supervisor_snapshot, ui_state),
        roster_lines: build_mud_roster_lines(supervisor_snapshot, ui_state),
        detail_lines: build_mud_detail_lines(supervisor_snapshot, ui_state),
        distributor_lines: build_mud_distributor_lines(&supervisor_snapshot.distributor, ui_state),
    }
}

fn build_mud_summary_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    let progress = parallel_mode_progress_summary(snapshot, None, false);
    let current = snapshot.roster.entries.get(ui_state.selected_actor_index);
    vec![
        fit_line(format!("Parallel  {}", progress.compact_line())),
        fit_line(match current {
            Some(entry) => format!(
                "Current  {}  ·  {}  ·  {}",
                truncate_text(&entry.task_title, FIELD_LIMIT),
                lifecycle_progress_label(&entry.state_label),
                truncate_text(&entry.duration_label, FIELD_LIMIT)
            ),
            None => format!("No active task  ·  {} slots available", progress.available),
        }),
        fit_line(format!(
            "{}  ·  Tab sections  ·  ↑↓ select  ·  Enter inspect",
            zone_label(ui_state.focused_zone)
        )),
    ]
}

fn build_mud_pool_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    let pool = &snapshot.pool;
    let slot_window = selected_centered_window(
        pool.slots.len(),
        ui_state.selected_room_index,
        PANEL_LINE_LIMIT.saturating_sub(1),
    );
    let progress = parallel_mode_progress_summary(snapshot, None, false);
    let mut lines = vec![fit_line(format!(
        "Capacity  {}{}",
        progress.compact_line(),
        bounded_window_suffix(&slot_window, pool.slots.len(), "slots")
    ))];
    if pool.slots.is_empty() {
        lines.push(fit_line(format!(
            "Preparing {} parallel slots",
            pool.configured_size
        )));
        return lines;
    }

    lines.extend(
        pool.slots
            .iter()
            .enumerate()
            .skip(slot_window.start)
            .take(slot_window.len())
            .map(|(index, slot)| {
                let task_title = snapshot
                    .roster
                    .entries
                    .iter()
                    .find(|entry| entry.slot_id == slot.slot_id)
                    .map(|entry| truncate_text(&entry.task_title, PANEL_TITLE_LIMIT));
                fit_line(format!(
                    "{}{}  ·  {}{}",
                    selection_prefix(is_selected_room(ui_state, index)),
                    slot.slot_id,
                    room_state_label(slot.state),
                    task_title
                        .map(|title| format!("  ·  {title}"))
                        .unwrap_or_default()
                ))
            }),
    );
    lines
}

fn build_mud_roster_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    if snapshot.roster.entries.is_empty() {
        return vec![fit_line(format!(
            "agents: none active | {}",
            truncate_text(&snapshot.roster.empty_state, SUMMARY_LIMIT)
        ))];
    }

    let actor_window = selected_centered_window(
        snapshot.roster.entries.len(),
        ui_state.selected_actor_index,
        PANEL_LINE_LIMIT,
    );
    snapshot
        .roster
        .entries
        .iter()
        .enumerate()
        .skip(actor_window.start)
        .take(actor_window.len())
        .map(|(index, entry)| {
            fit_line(format!(
                "{}{}  ·  {}  ·  {}",
                selection_prefix(is_selected_actor(ui_state, index)),
                truncate_text(&entry.task_title, PANEL_TITLE_LIMIT),
                lifecycle_progress_label(&entry.state_label),
                truncate_text(&entry.duration_label, FIELD_LIMIT)
            ))
        })
        .collect()
}

fn build_mud_detail_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    let selected_actor = snapshot.roster.entries.get(ui_state.selected_actor_index);
    let detail = snapshot.detail.session.as_ref().filter(|detail| {
        selected_actor.is_some_and(|actor| {
            actor.agent_id == detail.agent_id && actor.slot_id == detail.slot_id
        })
    });
    let Some(detail) = detail else {
        if let Some(actor) = selected_actor {
            return vec![
                fit_line(format!(
                    "{}Current  {}",
                    selection_prefix(ui_state.focused_zone == SupersessionMudFocusZone::QuestLog),
                    truncate_text(&actor.task_title, FIELD_LIMIT)
                )),
                fit_line(format!(
                    "Stage  {}  ·  {}",
                    lifecycle_progress_label(&actor.state_label),
                    truncate_text(&actor.duration_label, FIELD_LIMIT)
                )),
                fit_line(format!(
                    "Latest  {}",
                    truncate_text(&actor.latest_summary, SUMMARY_LIMIT)
                )),
            ];
        }
        return vec![
            "No selected task".to_string(),
            "Flow  queued → starting → working → delivery".to_string(),
        ];
    };
    let trail = detail
        .history
        .iter()
        .filter(|entry| !entry.state_label.trim().is_empty())
        .map(|entry| lifecycle_progress_label(&entry.state_label).to_string())
        .chain(std::iter::once(
            lifecycle_progress_label(&detail.state_label).to_string(),
        ))
        .fold(Vec::<String>::new(), |mut states, state| {
            if states.last() != Some(&state) {
                states.push(state);
            }
            states
        });
    let mut lines = vec![
        fit_line(format!(
            "{}Current  {}",
            selection_prefix(ui_state.focused_zone == SupersessionMudFocusZone::QuestLog),
            truncate_text(&detail.task_title, FIELD_LIMIT)
        )),
        fit_line(format!(
            "Progress  {}",
            truncate_text(&trail.join(" → "), LINE_LIMIT.saturating_sub(10))
        )),
        fit_line(format!(
            "Latest  {}",
            truncate_text(&detail.latest_summary, SUMMARY_LIMIT)
        )),
    ];
    if let Some(outcome) = detail.distributor_outcome.as_deref() {
        lines.push(fit_line(format!(
            "Delivery  {}",
            truncate_text(outcome, SUMMARY_LIMIT)
        )));
    }
    lines
}

fn build_mud_distributor_lines(
    distributor: &ParallelModeDistributorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    let held_line_count = usize::from(distributor.orchestrator_status.held_queue_count > 0);
    let queue_window = selected_centered_window(
        distributor.queue_items.len(),
        ui_state.selected_quest_index,
        PANEL_LINE_LIMIT.saturating_sub(2 + held_line_count),
    );
    let mut lines = vec![
        fit_line(format!(
            "{}Delivery  ·  {}  ·  {} items  ·  {} held{}",
            selection_prefix(ui_state.focused_zone == SupersessionMudFocusZone::ExitCorridor),
            distributor.head_summary,
            distributor.queue_depth(),
            distributor.orchestrator_status.held_queue_count,
            bounded_window_suffix(&queue_window, distributor.queue_items.len(), "items")
        )),
        fit_line(format!(
            "Baseline  {}",
            truncate_text(
                &distributor
                    .orchestrator_status
                    .integration_worktree_readiness,
                SUMMARY_LIMIT
            )
        )),
    ];
    if distributor.queue_items.is_empty() {
        lines.push("No work waiting for delivery".to_string());
    } else {
        lines.extend(
            distributor
                .queue_items
                .iter()
                .enumerate()
                .skip(queue_window.start)
                .take(queue_window.len())
                .map(|(index, item)| {
                    fit_line(format!(
                        "{}{}  ·  {}  ·  {}",
                        selection_prefix(is_selected_quest(ui_state, index)),
                        truncate_text(&item.task_title, PANEL_TITLE_LIMIT),
                        item.queue_state.label(),
                        if index == 0 { "current" } else { "next" }
                    ))
                }),
        );
    }
    if distributor.orchestrator_status.held_queue_count > 0 {
        lines.push(fit_line(format!(
            "{} task(s) waiting behind the current delivery",
            distributor.orchestrator_status.held_queue_count
        )));
    }
    lines
}

fn selected_centered_window(len: usize, selected_index: usize, max_visible: usize) -> Range<usize> {
    if len == 0 || max_visible == 0 {
        return 0..0;
    }
    let visible = len.min(max_visible);
    let selected_index = selected_index.min(len - 1);
    let start = selected_index
        .saturating_sub(visible / 2)
        .min(len - visible);
    start..start + visible
}

fn bounded_window_suffix(window: &Range<usize>, total: usize, label: &str) -> String {
    if window.start == 0 && window.end == total {
        String::new()
    } else {
        format!(" | {label} {}-{}/{}", window.start + 1, window.end, total)
    }
}

fn room_state_label(state: ParallelModePoolSlotState) -> &'static str {
    match state {
        ParallelModePoolSlotState::Idle => "available",
        ParallelModePoolSlotState::Leased => "starting",
        ParallelModePoolSlotState::Running => "working",
        ParallelModePoolSlotState::AwaitingCleanup => "finishing",
        ParallelModePoolSlotState::Blocked
        | ParallelModePoolSlotState::Missing
        | ParallelModePoolSlotState::Unavailable => "attention",
    }
}

fn moved_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len - 1;
    if delta < 0 {
        current.saturating_sub(delta.unsigned_abs()).min(last)
    } else {
        current.saturating_add(delta as usize).min(last)
    }
}

fn is_selected_room(ui_state: &SupersessionMudUiState, index: usize) -> bool {
    ui_state.focused_zone == SupersessionMudFocusZone::RealmMap
        && ui_state.selected_room_index == index
}

fn is_selected_actor(ui_state: &SupersessionMudUiState, index: usize) -> bool {
    matches!(
        ui_state.focused_zone,
        SupersessionMudFocusZone::Actors | SupersessionMudFocusZone::QuestLog
    ) && ui_state.selected_actor_index == index
}

fn is_selected_quest(ui_state: &SupersessionMudUiState, index: usize) -> bool {
    ui_state.focused_zone == SupersessionMudFocusZone::ExitCorridor
        && ui_state.selected_quest_index == index
}

fn selection_prefix(selected: bool) -> &'static str {
    if selected { "> " } else { "  " }
}

fn zone_label(zone: SupersessionMudFocusZone) -> &'static str {
    match zone {
        SupersessionMudFocusZone::RealmMap => "Capacity",
        SupersessionMudFocusZone::Actors => "Tasks",
        SupersessionMudFocusZone::QuestLog => "Current task",
        SupersessionMudFocusZone::ExitCorridor => "Delivery",
    }
}

pub(crate) fn lifecycle_progress_label(state_label: &str) -> &'static str {
    let normalized = state_label.trim().to_ascii_lowercase().replace('-', "_");
    if normalized.contains("block") || normalized.contains("fail") {
        "blocked"
    } else if normalized.contains("cleanup") || normalized.contains("clean") {
        "cleaned"
    } else if normalized.contains("deliver") || normalized.contains("queue") {
        "delivery"
    } else if normalized.contains("official") || normalized.contains("commit_ready") {
        "official"
    } else if normalized.contains("report") || normalized.contains("complete") {
        "reported"
    } else if normalized.contains("run") || normalized.contains("active") {
        "running"
    } else {
        "assigned"
    }
}

fn fit_line(text: String) -> String {
    truncate_text(&text, LINE_LIMIT)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
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
mod tests {
    use super::{
        SupersessionMudUiState, build_supersession_mud_lines, build_supersession_mud_view,
        parallel_mode_progress_summary,
    };
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
        ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
        ParallelModeDistributorQueueItem, ParallelModeDistributorSnapshot,
        ParallelModeOrchestratorStatus, ParallelModePoolBoardSnapshot,
        ParallelModePoolSlotSnapshot, ParallelModePoolSlotState, ParallelModeQueueItemState,
        ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
        ParallelModeSupervisorState,
    };
    use crate::domain::planning::{PriorityQueueProjection, PriorityQueueTask, TaskStatus};

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

        assert!(rendered.contains("Parallel  ◆ 2 delivery"));
        assert!(rendered.contains("1 available"));
        assert!(rendered.contains("! 1 attention"));
        assert!(rendered.contains("> slot-1  ·  working"));
        assert!(rendered.contains("Parallel Mode MUD"));
        assert!(rendered.contains("running  ·  04m12s"));
        assert!(rendered.contains("Current  Parallel Mode MUD Timeline UI Pack"));
        assert!(rendered.contains("Progress  assigned → running → official"));
        assert!(rendered.contains("Delivery  ·  queued  ·  2 items  ·  1 held"));
        assert!(rendered.contains("1 task(s) waiting behind the current delivery"));
        assert!(
            rendered.lines().all(|line| line.chars().count() <= 112),
            "MUD projection should keep line width bounded for narrow TUI panels:\n{rendered}"
        );
    }

    #[test]
    fn supersession_mud_projection_marks_focus_and_survives_narrow_copy() {
        let snapshot = ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            "/tmp/root/projects/codex-exec-loop",
            ParallelModePoolBoardSnapshot::new(
                2,
                "/tmp/root/projects/codex-exec-loop-akra-worktrees/pool",
                "idle",
                vec![
                    ParallelModePoolSlotSnapshot::new(
                        "slot-1",
                        ParallelModePoolSlotState::Idle,
                        "prerelease",
                        "akra-pool/slot-1",
                        "idle",
                    ),
                    ParallelModePoolSlotSnapshot::new(
                        "slot-2",
                        ParallelModePoolSlotState::Running,
                        "akra-agent/slot-2/parallel-mode-mud-ui-pack",
                        "akra-pool/slot-2",
                        "agent-2 / task-2",
                    ),
                ],
            ),
            ParallelModeAgentRosterSnapshot::new(
                vec![ParallelModeAgentRosterEntry::new(
                    "agent-2",
                    "Parallel Mode MUD Timeline UI Pack",
                    "slot-2",
                    "akra-agent/slot-2/parallel-mode-mud-ui-pack",
                    "running",
                    "04m12s",
                    "rendering the selected session timeline",
                )],
                "no active agents",
            ),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(
                vec![ParallelModeDistributorQueueItem::new(
                    "agent-2",
                    "Parallel Mode MUD Timeline UI Pack",
                    ParallelModeQueueItemState::Queued,
                    "akra-agent/slot-2/parallel-mode-mud-ui-pack",
                    "abc1234",
                    "ready",
                )],
                Vec::new(),
                "queued",
                "ready",
            ),
            None,
        );
        let mut ui_state = SupersessionMudUiState::default();
        ui_state.move_selection(&snapshot, 1);
        ui_state.inspect_focused(&snapshot);
        ui_state.focus_next_zone();
        let projection = build_supersession_mud_view(&snapshot, &ui_state);
        let rendered = [
            projection.summary_lines,
            projection.pool_lines,
            projection.roster_lines,
            projection.detail_lines,
            projection.distributor_lines,
        ]
        .concat()
        .join("\n");

        assert!(rendered.contains("Delivery  ·  Tab sections"));
        assert!(rendered.contains("slot-1  ·  available"));
        assert!(rendered.contains("slot-2  ·  working"));
        assert!(rendered.contains("> Delivery  ·  queued  ·  1 items"));
        assert!(rendered.contains("Current  Parallel Mode MUD Timeline UI Pack"));
        assert!(
            rendered.lines().all(|line| line.chars().count() <= 112),
            "focused MUD projection should remain bounded:\n{rendered}"
        );
    }

    #[test]
    fn supersession_mud_windows_keep_deep_selection_inside_panel_capacity() {
        let snapshot = large_supervisor_snapshot(12);
        let mut ui_state = SupersessionMudUiState::default();

        ui_state.move_selection(&snapshot, 10);
        let pool = build_supersession_mud_view(&snapshot, &ui_state).pool_lines;
        assert!(pool.len() <= super::PANEL_LINE_LIMIT);
        assert!(pool.iter().any(|line| line.starts_with("> slot-11  ")));

        ui_state.focus_next_zone();
        ui_state.move_selection(&snapshot, 10);
        let roster = build_supersession_mud_view(&snapshot, &ui_state).roster_lines;
        assert!(roster.len() <= super::PANEL_LINE_LIMIT);
        assert!(
            roster
                .iter()
                .any(|line| line.starts_with("> Task 11  ·  running"))
        );

        ui_state.focus_next_zone();
        ui_state.focus_next_zone();
        ui_state.move_selection(&snapshot, 10);
        let distributor = build_supersession_mud_view(&snapshot, &ui_state).distributor_lines;
        assert!(distributor.len() <= super::PANEL_LINE_LIMIT);
        assert!(
            distributor
                .iter()
                .any(|line| line.starts_with("> Task 11  ·  queued  ·  next"))
        );
    }

    #[test]
    fn progress_summary_aggregates_large_pool_without_recounting_active_tasks() {
        let snapshot = large_supervisor_snapshot(12);
        let tasks = (1..=15).map(queue_task).collect::<Vec<_>>();
        let queue_projection = PriorityQueueProjection {
            next_task: tasks.first().cloned(),
            active_tasks: tasks,
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        };

        let progress = parallel_mode_progress_summary(&snapshot, Some(&queue_projection), false);

        assert_eq!(progress.working, 0);
        assert_eq!(progress.delivering, 12);
        assert_eq!(progress.queued, 3);
        assert_eq!(progress.available, 0);
        assert_eq!(progress.compact_line(), "◆ 12 delivery  ·  ○ 3 queued");
    }

    #[test]
    fn quest_log_uses_selected_actor_when_snapshot_detail_belongs_to_another_actor() {
        let snapshot = large_supervisor_snapshot(2);
        let mut ui_state = SupersessionMudUiState::default();
        ui_state.focus_next_zone();
        ui_state.move_selection(&snapshot, 1);
        ui_state.focus_next_zone();

        let detail = build_supersession_mud_view(&snapshot, &ui_state).detail_lines;
        let rendered = detail.join("\n");

        assert!(rendered.contains("> Current  Task 2"));
        assert!(rendered.contains("Latest  progress 2"));
        assert!(!rendered.contains("agent-1"));
        assert!(!rendered.contains("Progress  assigned"));
    }

    fn large_supervisor_snapshot(count: usize) -> ParallelModeSupervisorSnapshot {
        let slots = (1..=count)
            .map(|index| {
                ParallelModePoolSlotSnapshot::new(
                    format!("slot-{index}"),
                    ParallelModePoolSlotState::Running,
                    format!("agent/{index}"),
                    format!("pool/{index}"),
                    format!("agent-{index}"),
                )
            })
            .collect();
        let roster = (1..=count)
            .map(|index| {
                ParallelModeAgentRosterEntry::new(
                    format!("agent-{index}"),
                    format!("Task {index}"),
                    format!("slot-{index}"),
                    format!("agent/{index}"),
                    "running",
                    format!("{index}m"),
                    format!("progress {index}"),
                )
                .with_lease_identity(
                    format!("task-{index}"),
                    format!("slot-{index}:task-{index}"),
                    None,
                )
            })
            .collect();
        let queue_items = (1..=count)
            .map(|index| {
                ParallelModeDistributorQueueItem::new(
                    format!("agent-{index}"),
                    format!("Task {index}"),
                    ParallelModeQueueItemState::Queued,
                    format!("agent/{index}"),
                    format!("sha{index}"),
                    format!("queued {index}"),
                )
            })
            .collect();
        let detail = ParallelModeAgentSessionDetailSnapshot::new(
            "slot-1:task-1",
            "agent-1",
            "task-1",
            "Task 1",
            "slot-1",
            Some("thread-1".to_string()),
            "/tmp/pool/1",
            "agent/1",
            "2026-07-15T00:00:00Z",
            "running",
            "running",
            "progress 1",
            "tests pending",
            "ledger pending",
            None,
            Vec::new(),
            "2026-07-15T00:01:00Z",
        );
        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            "/tmp/root",
            ParallelModePoolBoardSnapshot::new(count, "/tmp/pool", "running", slots),
            ParallelModeAgentRosterSnapshot::new(roster, "empty"),
            ParallelModeSupervisorDetailSnapshot::new(Some(detail), "empty"),
            ParallelModeDistributorSnapshot::new(queue_items, Vec::new(), "queued", "queue active"),
            None,
        )
    }

    fn queue_task(index: usize) -> PriorityQueueTask {
        PriorityQueueTask {
            rank: index,
            task_id: format!("task-{index}"),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction".to_string(),
            task_title: format!("Task {index}"),
            status: TaskStatus::Ready,
            combined_priority: 10,
            updated_at: "2026-07-15T00:00:00Z".to_string(),
            rank_reasons: vec!["status=ready".to_string()],
        }
    }
}
