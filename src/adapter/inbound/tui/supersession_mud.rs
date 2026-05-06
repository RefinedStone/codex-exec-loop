use crate::domain::parallel_mode::{
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeSupervisorSnapshot,
};

const LINE_LIMIT: usize = 112;
const FIELD_LIMIT: usize = 34;
const SUMMARY_LIMIT: usize = 56;

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
        pool_lines: build_mud_pool_lines(&supervisor_snapshot.pool, ui_state),
        roster_lines: build_mud_roster_lines(supervisor_snapshot, ui_state),
        detail_lines: build_mud_detail_lines(supervisor_snapshot, ui_state),
        distributor_lines: build_mud_distributor_lines(&supervisor_snapshot.distributor, ui_state),
    }
}

fn build_mud_summary_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    vec![
        fit_line(format!(
            "realm: {} | lanes {} | actors {} | corridor {}",
            snapshot.state_label(),
            pool_pressure_label(&snapshot.pool),
            snapshot.roster.active_count(),
            snapshot.distributor.compact_summary()
        )),
        fit_line(format!(
            "quest board: {} | workspace {}",
            snapshot
                .top_notice
                .as_deref()
                .map(|notice| truncate_text(notice, SUMMARY_LIMIT))
                .unwrap_or_else(|| "no active notice".to_string()),
            truncate_text(&snapshot.workspace_path, FIELD_LIMIT)
        )),
        fit_line(format!(
            "focus: {} | move Tab/arrows | inspect Enter/Space",
            zone_label(ui_state.focused_zone)
        )),
    ]
}

fn build_mud_pool_lines(
    pool: &ParallelModePoolBoardSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    let mut lines = vec![
        fit_line(format!(
            "realm map: {}",
            pool.slots
                .iter()
                .enumerate()
                .map(|(index, slot)| selected_token(
                    slot_room_token(slot),
                    is_selected_room(ui_state, index)
                ))
                .collect::<Vec<_>>()
                .join(" ")
        )),
        fit_line(format!(
            "lane map: {}",
            pool.slots
                .iter()
                .map(slot_room_token)
                .collect::<Vec<_>>()
                .join(" ")
        )),
    ];
    if pool.slots.is_empty() {
        lines.push(fit_line(format!(
            "rooms: waiting for {} lanes at {}",
            pool.configured_size,
            truncate_text(&pool.pool_root_label, FIELD_LIMIT)
        )));
        return lines;
    }

    lines.extend(pool.slots.iter().enumerate().map(|(index, slot)| {
        fit_line(format!(
            "{}room {} {} | branch {} | gate {} | owner {}",
            selection_prefix(is_selected_room(ui_state, index)),
            slot.slot_id,
            room_state_label(slot.state),
            truncate_text(&slot.branch_name, FIELD_LIMIT),
            slot_exit_label(slot),
            truncate_text(&slot.owner_label, FIELD_LIMIT)
        ))
    }));
    lines
}

fn build_mud_roster_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    if snapshot.roster.entries.is_empty() {
        return vec![fit_line(format!(
            "actors: tavern quiet | {}",
            truncate_text(&snapshot.roster.empty_state, SUMMARY_LIMIT)
        ))];
    }

    snapshot
        .roster
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            fit_line(format!(
                "{}actor {} in {} | quest {} | progress {} | signal {}",
                selection_prefix(is_selected_actor(ui_state, index)),
                entry.agent_id,
                entry.slot_id,
                truncate_text(&entry.task_title, FIELD_LIMIT),
                lifecycle_progress_label(&entry.state_label),
                truncate_text(&entry.latest_summary, SUMMARY_LIMIT)
            ))
        })
        .collect()
}

fn build_mud_detail_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    let Some(detail) = snapshot.detail.session.as_ref() else {
        return vec![
            "quest log: no selected actor".to_string(),
            "trail: room -> thread -> report -> ledger -> corridor".to_string(),
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
            "{}quest log: {} / {} / {}",
            selection_prefix(ui_state.focused_zone == SupersessionMudFocusZone::QuestLog),
            detail.slot_id,
            detail.agent_id,
            truncate_text(&detail.task_title, FIELD_LIMIT)
        )),
        fit_line(format!(
            "trail: {}",
            truncate_text(&trail.join(" -> "), LINE_LIMIT.saturating_sub(7))
        )),
        fit_line(format!(
            "last signal: {}",
            truncate_text(&detail.latest_summary, SUMMARY_LIMIT)
        )),
    ];
    if let Some(outcome) = detail.distributor_outcome.as_deref() {
        lines.push(fit_line(format!(
            "corridor handoff: {}",
            truncate_text(outcome, SUMMARY_LIMIT)
        )));
    }
    lines
}

fn build_mud_distributor_lines(
    distributor: &ParallelModeDistributorSnapshot,
    ui_state: &SupersessionMudUiState,
) -> Vec<String> {
    let mut lines = vec![
        fit_line(format!(
            "{}exit corridor: head {} | depth {} | barrier {}",
            selection_prefix(ui_state.focused_zone == SupersessionMudFocusZone::ExitCorridor),
            distributor.head_summary,
            distributor.queue_depth(),
            distributor.orchestrator_status.barrier_state
        )),
        fit_line(format!(
            "gate check: {}",
            truncate_text(
                &distributor
                    .orchestrator_status
                    .integration_worktree_readiness,
                SUMMARY_LIMIT
            )
        )),
    ];
    if distributor.queue_items.is_empty() {
        lines.push("queue: corridor empty".to_string());
    } else {
        lines.extend(
            distributor
                .queue_items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    fit_line(format!(
                        "{}{} {} | actor {} | quest {} | branch {}",
                        selection_prefix(is_selected_quest(ui_state, index)),
                        if index == 0 { "head" } else { "held" },
                        item.queue_state.label(),
                        item.source_agent,
                        truncate_text(&item.task_title, FIELD_LIMIT),
                        truncate_text(&item.branch_name, FIELD_LIMIT)
                    ))
                }),
        );
    }
    if distributor.orchestrator_status.held_queue_count > 0 {
        lines.push(fit_line(format!(
            "held behind head: {} quest(s)",
            distributor.orchestrator_status.held_queue_count
        )));
    }
    lines
}

fn pool_pressure_label(pool: &ParallelModePoolBoardSnapshot) -> String {
    format!(
        "idle {}/running {}/blocked {}",
        pool.idle_slots, pool.running_slots, pool.blocked_slots
    )
}

fn slot_room_token(slot: &ParallelModePoolSlotSnapshot) -> String {
    format!(
        "[{}:{}]",
        slot.slot_id,
        match slot.state {
            ParallelModePoolSlotState::Idle => "IDLE",
            ParallelModePoolSlotState::Leased => "LEASE",
            ParallelModePoolSlotState::Running => "RUN",
            ParallelModePoolSlotState::AwaitingCleanup => "CLEAN",
            ParallelModePoolSlotState::Blocked => "BLOCK",
            ParallelModePoolSlotState::Missing => "MISS",
            ParallelModePoolSlotState::Unavailable => "DOWN",
        }
    )
}

fn room_state_label(state: ParallelModePoolSlotState) -> &'static str {
    match state {
        ParallelModePoolSlotState::Idle => "rests",
        ParallelModePoolSlotState::Leased => "is claimed",
        ParallelModePoolSlotState::Running => "is active",
        ParallelModePoolSlotState::AwaitingCleanup => "awaits cleanup",
        ParallelModePoolSlotState::Blocked => "is blocked",
        ParallelModePoolSlotState::Missing => "is missing",
        ParallelModePoolSlotState::Unavailable => "is down",
    }
}

fn slot_exit_label(slot: &ParallelModePoolSlotSnapshot) -> &'static str {
    match slot.state {
        ParallelModePoolSlotState::Idle => "open",
        ParallelModePoolSlotState::Leased | ParallelModePoolSlotState::Running => "occupied",
        ParallelModePoolSlotState::AwaitingCleanup => "returning",
        ParallelModePoolSlotState::Blocked
        | ParallelModePoolSlotState::Missing
        | ParallelModePoolSlotState::Unavailable => "blocked",
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

fn selected_token(token: String, selected: bool) -> String {
    if selected {
        format!(">{token}<")
    } else {
        token
    }
}

fn selection_prefix(selected: bool) -> &'static str {
    if selected { "> " } else { "  " }
}

fn zone_label(zone: SupersessionMudFocusZone) -> &'static str {
    match zone {
        SupersessionMudFocusZone::RealmMap => "realm map",
        SupersessionMudFocusZone::Actors => "actors",
        SupersessionMudFocusZone::QuestLog => "quest log",
        SupersessionMudFocusZone::ExitCorridor => "exit corridor",
    }
}

fn lifecycle_progress_label(state_label: &str) -> &'static str {
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
