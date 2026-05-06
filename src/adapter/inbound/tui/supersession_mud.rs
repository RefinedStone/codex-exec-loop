use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
    ParallelModeSupervisorSnapshot,
};

const LINE_LIMIT: usize = 112;
const FIELD_LIMIT: usize = 34;
const SUMMARY_LIMIT: usize = 56;

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
    SupersessionMudLines {
        summary_lines: build_mud_summary_lines(supervisor_snapshot),
        pool_lines: build_mud_pool_lines(&supervisor_snapshot.pool),
        roster_lines: build_mud_roster_lines(supervisor_snapshot),
        detail_lines: build_mud_detail_lines(supervisor_snapshot.detail.session.as_ref()),
        distributor_lines: build_mud_distributor_lines(&supervisor_snapshot.distributor),
    }
}

fn build_mud_summary_lines(snapshot: &ParallelModeSupervisorSnapshot) -> Vec<String> {
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
    ]
}

fn build_mud_pool_lines(pool: &ParallelModePoolBoardSnapshot) -> Vec<String> {
    let mut lines = vec![fit_line(format!(
        "lane map: {}",
        pool.slots
            .iter()
            .map(slot_room_token)
            .collect::<Vec<_>>()
            .join(" ")
    ))];
    if pool.slots.is_empty() {
        lines.push(fit_line(format!(
            "rooms: waiting for {} lanes at {}",
            pool.configured_size,
            truncate_text(&pool.pool_root_label, FIELD_LIMIT)
        )));
        return lines;
    }

    lines.extend(pool.slots.iter().map(|slot| {
        fit_line(format!(
            "room {} {} | branch {} | gate {} | owner {}",
            slot.slot_id,
            room_state_label(slot.state),
            truncate_text(&slot.branch_name, FIELD_LIMIT),
            slot_exit_label(slot),
            truncate_text(&slot.owner_label, FIELD_LIMIT)
        ))
    }));
    lines
}

fn build_mud_roster_lines(snapshot: &ParallelModeSupervisorSnapshot) -> Vec<String> {
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
        .map(|entry| {
            fit_line(format!(
                "actor {} in {} | quest {} | state {} | signal {}",
                entry.agent_id,
                entry.slot_id,
                truncate_text(&entry.task_title, FIELD_LIMIT),
                entry.state_label.replace('_', " "),
                truncate_text(&entry.latest_summary, SUMMARY_LIMIT)
            ))
        })
        .collect()
}

fn build_mud_detail_lines(detail: Option<&ParallelModeAgentSessionDetailSnapshot>) -> Vec<String> {
    let Some(detail) = detail else {
        return vec![
            "quest log: no selected actor".to_string(),
            "trail: room -> thread -> report -> ledger -> corridor".to_string(),
        ];
    };
    let trail = detail
        .history
        .iter()
        .filter(|entry| !entry.state_label.trim().is_empty())
        .map(|entry| entry.state_label.replace('_', " "))
        .chain(std::iter::once(detail.state_label.replace('_', " ")))
        .fold(Vec::<String>::new(), |mut states, state| {
            if states.last() != Some(&state) {
                states.push(state);
            }
            states
        });
    let mut lines = vec![
        fit_line(format!(
            "quest log: {} / {} / {}",
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

fn build_mud_distributor_lines(distributor: &ParallelModeDistributorSnapshot) -> Vec<String> {
    let mut lines = vec![
        fit_line(format!(
            "exit corridor: head {} | depth {} | barrier {}",
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
                        "{} {} | actor {} | quest {} | branch {}",
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
