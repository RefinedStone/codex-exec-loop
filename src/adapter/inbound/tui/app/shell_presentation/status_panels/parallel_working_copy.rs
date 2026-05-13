use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::text::{Line, Span};

use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
    ParallelModeSupervisorSnapshot,
};

use super::super::{
    AkraTheme, INLINE_TAIL_STATUS_DETAIL_LIMIT, Modifier, NativeTuiApp, compact_inline_detail,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelSlotWorkingStatus {
    slot_id: String,
    state_label: String,
    duration_label: Option<String>,
    detail: String,
}

pub(super) fn build_parallel_slot_working_line(app: &NativeTuiApp) -> Option<Line<'static>> {
    if !app.parallel_mode_enabled() {
        return None;
    }

    let snapshot = app.parallel_mode_supervisor_snapshot();
    let statuses = parallel_slot_working_statuses(&snapshot);
    let selected_index = rotated_parallel_slot_status_index(
        statuses.len(),
        current_parallel_slot_rotation_elapsed_seconds(),
    )?;
    let status = statuses.get(selected_index)?;
    let mut segments = vec![format!("pool {}", status.slot_id)];
    if statuses.len() > 1 {
        segments.push(format!("{}/{}", selected_index + 1, statuses.len()));
    }
    segments.push(format!("state: {}", status.state_label));
    if let Some(duration_label) = status.duration_label.as_deref() {
        segments.push(duration_label.to_string());
    }
    segments.push(compact_inline_detail(
        &status.detail,
        INLINE_TAIL_STATUS_DETAIL_LIMIT,
    ));

    Some(Line::from(vec![
        Span::styled(
            "◦ Working".to_string(),
            AkraTheme::muted().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" ({})", segments.join(" • ")), AkraTheme::subtle()),
    ]))
}

fn parallel_slot_working_statuses(
    snapshot: &ParallelModeSupervisorSnapshot,
) -> Vec<ParallelSlotWorkingStatus> {
    let mut statuses = snapshot
        .pool
        .slots
        .iter()
        .filter(|slot| slot.state != ParallelModePoolSlotState::Idle)
        .map(|slot| {
            let roster_entry = roster_entry_for_slot(snapshot, &slot.slot_id);
            parallel_slot_status_from_pool_slot(slot, roster_entry)
        })
        .collect::<Vec<_>>();

    if statuses.is_empty() {
        statuses = snapshot
            .roster
            .entries
            .iter()
            .filter(|entry| entry.counts_as_active())
            .map(parallel_slot_status_from_roster_entry)
            .collect();
    }

    statuses.sort_by(|left, right| compare_parallel_slot_ids(&left.slot_id, &right.slot_id));
    statuses
}

fn compare_parallel_slot_ids(left: &str, right: &str) -> Ordering {
    match (
        parse_slot_numeric_suffix(left),
        parse_slot_numeric_suffix(right),
    ) {
        (Some((left_prefix, left_number)), Some((right_prefix, right_number)))
            if left_prefix == right_prefix =>
        {
            left_number.cmp(&right_number).then_with(|| left.cmp(right))
        }
        _ => left.cmp(right),
    }
}

fn parse_slot_numeric_suffix(slot_id: &str) -> Option<(&str, u64)> {
    let (prefix, suffix) = slot_id.rsplit_once('-')?;
    let number = suffix.parse::<u64>().ok()?;
    Some((prefix, number))
}

fn roster_entry_for_slot<'a>(
    snapshot: &'a ParallelModeSupervisorSnapshot,
    slot_id: &str,
) -> Option<&'a ParallelModeAgentRosterEntry> {
    snapshot
        .roster
        .entries
        .iter()
        .find(|entry| entry.slot_id == slot_id)
}

fn parallel_slot_status_from_pool_slot(
    slot: &ParallelModePoolSlotSnapshot,
    roster_entry: Option<&ParallelModeAgentRosterEntry>,
) -> ParallelSlotWorkingStatus {
    ParallelSlotWorkingStatus {
        slot_id: slot.slot_id.clone(),
        state_label: parallel_slot_state_label(slot.state, roster_entry),
        duration_label: roster_entry.and_then(parallel_slot_duration_label),
        detail: roster_entry
            .map(parallel_slot_detail_from_roster_entry)
            .unwrap_or_else(|| parallel_slot_detail_from_pool_slot(slot)),
    }
}

fn parallel_slot_status_from_roster_entry(
    entry: &ParallelModeAgentRosterEntry,
) -> ParallelSlotWorkingStatus {
    ParallelSlotWorkingStatus {
        slot_id: entry.slot_id.clone(),
        state_label: humanize_parallel_status_label(&entry.state_label),
        duration_label: parallel_slot_duration_label(entry),
        detail: parallel_slot_detail_from_roster_entry(entry),
    }
}

fn parallel_slot_state_label(
    pool_state: ParallelModePoolSlotState,
    roster_entry: Option<&ParallelModeAgentRosterEntry>,
) -> String {
    let pool_label = pool_slot_state_label(pool_state);
    let Some(roster_entry) = roster_entry else {
        return pool_label.to_string();
    };
    let roster_label = humanize_parallel_status_label(&roster_entry.state_label);
    if roster_label == pool_label {
        pool_label.to_string()
    } else {
        format!("{pool_label} / {roster_label}")
    }
}

fn pool_slot_state_label(state: ParallelModePoolSlotState) -> &'static str {
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

fn humanize_parallel_status_label(label: &str) -> String {
    label.replace('_', " ")
}

fn parallel_slot_duration_label(entry: &ParallelModeAgentRosterEntry) -> Option<String> {
    let duration_label = entry.duration_label.trim();
    if duration_label.is_empty() {
        return None;
    }
    Some(humanize_parallel_status_label(duration_label))
}

fn parallel_slot_detail_from_roster_entry(entry: &ParallelModeAgentRosterEntry) -> String {
    let task_title = entry.task_title.trim();
    let latest_summary = entry.latest_summary.trim();
    match (task_title.is_empty(), latest_summary.is_empty()) {
        (false, false) => format!("{task_title}: {latest_summary}"),
        (false, true) => task_title.to_string(),
        (true, false) => latest_summary.to_string(),
        (true, true) => entry.agent_id.clone(),
    }
}

fn parallel_slot_detail_from_pool_slot(slot: &ParallelModePoolSlotSnapshot) -> String {
    let owner_label = slot.owner_label.trim();
    if !owner_label.is_empty() {
        return owner_label.to_string();
    }
    let worktree_label = slot.worktree_label.trim();
    if !worktree_label.is_empty() {
        return worktree_label.to_string();
    }
    slot.branch_name.clone()
}

fn current_parallel_slot_rotation_elapsed_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn rotated_parallel_slot_status_index(status_count: usize, elapsed_seconds: u64) -> Option<usize> {
    const ROTATION_WINDOW_SECONDS: u64 = 3;
    if status_count == 0 {
        return None;
    }
    let status_count = status_count as u64;
    Some(((elapsed_seconds / ROTATION_WINDOW_SECONDS) % status_count) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterSnapshot, ParallelModeDistributorSnapshot,
        ParallelModePoolBoardSnapshot, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
        ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorState,
    };

    fn supervisor_snapshot_with_slots() -> ParallelModeSupervisorSnapshot {
        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::derive(
                true,
                Some(&ParallelModeReadinessSnapshot::new(
                    "/repo",
                    ParallelModeReadinessState::Ready,
                    Vec::new(),
                    None,
                )),
            ),
            "/repo",
            ParallelModePoolBoardSnapshot::new(
                3,
                "/tmp/pool",
                "running",
                vec![
                    ParallelModePoolSlotSnapshot::new(
                        "slot-2",
                        ParallelModePoolSlotState::Running,
                        "akra-agent/slot-2/task-two",
                        "akra-pool/slot-2",
                        "agent-2 / task-2",
                    ),
                    ParallelModePoolSlotSnapshot::new(
                        "slot-1",
                        ParallelModePoolSlotState::Blocked,
                        "akra-agent/slot-1/task-one",
                        "akra-pool/slot-1",
                        "agent-1 / task-1",
                    ),
                    ParallelModePoolSlotSnapshot::new(
                        "slot-10",
                        ParallelModePoolSlotState::Running,
                        "akra-agent/slot-10/task-ten",
                        "akra-pool/slot-10",
                        "agent-10 / task-10",
                    ),
                ],
            ),
            ParallelModeAgentRosterSnapshot::new(
                vec![
                    ParallelModeAgentRosterEntry::new(
                        "agent-1",
                        "Task One",
                        "slot-1",
                        "akra-agent/slot-1/task-one",
                        "running",
                        "1m 5s",
                        "editing the TUI tail",
                    ),
                    ParallelModeAgentRosterEntry::new(
                        "agent-2",
                        "Task Two",
                        "slot-2",
                        "akra-agent/slot-2/task-two",
                        "running",
                        "2m",
                        "checking status rotation",
                    ),
                    ParallelModeAgentRosterEntry::new(
                        "agent-10",
                        "Task Ten",
                        "slot-10",
                        "akra-agent/slot-10/task-ten",
                        "running",
                        "10m",
                        "checking natural sort",
                    ),
                ],
                "empty",
            ),
            ParallelModeSupervisorDetailSnapshot::new(None, "empty"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "idle"),
            None,
        )
    }

    #[test]
    fn parallel_slot_working_statuses_are_slot_ordered_and_join_pool_state() {
        let snapshot = supervisor_snapshot_with_slots();
        let statuses = parallel_slot_working_statuses(&snapshot);

        assert_eq!(statuses.len(), 3);
        assert_eq!(statuses[0].slot_id, "slot-1");
        assert_eq!(statuses[0].state_label, "blocked / running");
        assert_eq!(statuses[0].duration_label.as_deref(), Some("1m 5s"));
        assert!(statuses[0].detail.contains("Task One"));
        assert_eq!(statuses[1].slot_id, "slot-2");
        assert_eq!(statuses[2].slot_id, "slot-10");
    }

    #[test]
    fn parallel_slot_working_rotation_advances_every_three_seconds() {
        assert_eq!(rotated_parallel_slot_status_index(3, 0), Some(0));
        assert_eq!(rotated_parallel_slot_status_index(3, 2), Some(0));
        assert_eq!(rotated_parallel_slot_status_index(3, 3), Some(1));
        assert_eq!(rotated_parallel_slot_status_index(3, 6), Some(2));
        assert_eq!(rotated_parallel_slot_status_index(3, 9), Some(0));
        assert_eq!(rotated_parallel_slot_status_index(0, 9), None);
    }
}
