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
            "supervisor: {} | slots {} | agents {} | distributor {}",
            snapshot.state_label(),
            pool_pressure_label(&snapshot.pool),
            snapshot.roster.active_count(),
            snapshot.distributor.compact_summary()
        )),
        fit_line(format!(
            "task board: {} | workspace {}",
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
    let mut lines = vec![fit_line(format!(
        "pool board: {}",
        pool.slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                selected_token(slot_room_token(slot), is_selected_room(ui_state, index))
            })
            .collect::<Vec<_>>()
            .join(" ")
    ))];
    if pool.slots.is_empty() {
        lines.push(fit_line(format!(
            "slots: waiting for {} pool slots at {}",
            pool.configured_size,
            truncate_text(&pool.pool_root_label, FIELD_LIMIT)
        )));
        return lines;
    }

    lines.extend(pool.slots.iter().enumerate().map(|(index, slot)| {
        fit_line(format!(
            "{}slot {} {} | branch {} | lease {} | owner {}",
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
            "agents: none active | {}",
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
                "{}agent {} in {} | task {} | progress {} | summary {}",
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
            "session detail: no selected agent".to_string(),
            "flow: slot -> thread -> report -> ledger -> distributor".to_string(),
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
            "{}session detail: {} / {} / {}",
            selection_prefix(ui_state.focused_zone == SupersessionMudFocusZone::QuestLog),
            detail.slot_id,
            detail.agent_id,
            truncate_text(&detail.task_title, FIELD_LIMIT)
        )),
        fit_line(format!(
            "flow: {}",
            truncate_text(&trail.join(" -> "), LINE_LIMIT.saturating_sub(7))
        )),
        fit_line(format!(
            "latest summary: {}",
            truncate_text(&detail.latest_summary, SUMMARY_LIMIT)
        )),
    ];
    if let Some(outcome) = detail.distributor_outcome.as_deref() {
        lines.push(fit_line(format!(
            "distributor handoff: {}",
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
            "{}distributor queue: head {} | depth {} | barrier {}",
            selection_prefix(ui_state.focused_zone == SupersessionMudFocusZone::ExitCorridor),
            distributor.head_summary,
            distributor.queue_depth(),
            distributor.orchestrator_status.barrier_state
        )),
        fit_line(format!(
            "integration check: {}",
            truncate_text(
                &distributor
                    .orchestrator_status
                    .integration_worktree_readiness,
                SUMMARY_LIMIT
            )
        )),
    ];
    if distributor.queue_items.is_empty() {
        lines.push("queue: no distributor items".to_string());
    } else {
        lines.extend(
            distributor
                .queue_items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    fit_line(format!(
                        "{}{} {} | agent {} | task {} | branch {}",
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
            "held behind head: {} task(s)",
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
        ParallelModePoolSlotState::Idle => "idle",
        ParallelModePoolSlotState::Leased => "leased",
        ParallelModePoolSlotState::Running => "running",
        ParallelModePoolSlotState::AwaitingCleanup => "cleanup pending",
        ParallelModePoolSlotState::Blocked => "blocked",
        ParallelModePoolSlotState::Missing => "missing",
        ParallelModePoolSlotState::Unavailable => "unavailable",
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
        SupersessionMudFocusZone::RealmMap => "pool board",
        SupersessionMudFocusZone::Actors => "agent roster",
        SupersessionMudFocusZone::QuestLog => "session detail",
        SupersessionMudFocusZone::ExitCorridor => "distributor queue",
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

#[cfg(test)]
mod tests {
    use super::{
        SupersessionMudUiState, build_supersession_mud_lines, build_supersession_mud_view,
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

        assert!(rendered.contains("supervisor: supervise"));
        assert!(rendered.contains("pool board: >[slot-1:RUN]< [slot-2:IDLE] [slot-3:BLOCK]"));
        assert!(rendered.contains("agent agent-1 in slot-1"));
        assert!(rendered.contains("session detail: slot-1 / agent-1"));
        assert!(rendered.contains("flow: assigned -> running -> official"));
        assert!(rendered.contains("distributor queue: head queued | depth 2"));
        assert!(rendered.contains("held behind head: 1 task(s)"));
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

        assert!(rendered.contains("focus: distributor queue"));
        assert!(rendered.contains("pool board: [slot-1:IDLE] [slot-2:RUN]"));
        assert!(rendered.contains("> distributor queue: head queued | depth 1"));
        assert!(rendered.contains("session detail: no selected agent"));
        assert!(
            rendered.lines().all(|line| line.chars().count() <= 112),
            "focused MUD projection should remain bounded:\n{rendered}"
        );
    }
}
