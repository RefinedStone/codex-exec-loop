use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolSlotState,
    ParallelModeRuntimeEventFeedEntry, ParallelModeSupervisorSnapshot,
};

use super::AkraTheme;
use super::language::{TUI_LOCALIZED_IMPORTANT_MARKERS, TuiLanguage};

#[cfg(test)]
const MAX_PARALLEL_SUPERVISOR_EVENTS: usize = 96;
const MAX_PARALLEL_EVENT_WINDOW: usize = 512;
pub(super) const PARALLEL_SUPERVISOR_OPERATOR_ACTOR: &str = "You";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct ParallelStreamEventId {
    stream_generation: u64,
    ordinal: u64,
}

impl ParallelStreamEventId {
    pub(super) fn new(stream_generation: u64, ordinal: u64) -> Self {
        Self {
            stream_generation,
            ordinal,
        }
    }

    pub(super) fn stream_generation(self) -> u64 {
        self.stream_generation
    }

    pub(super) fn ordinal(self) -> u64 {
        self.ordinal
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ParallelEventSourceId {
    Authority {
        workspace: String,
        sequence: i64,
    },
    LocalAccepted {
        operation_kind: String,
        correlation: u64,
    },
    ObservedStateTransition {
        subject: String,
        previous: Option<String>,
        current: String,
        observed_revision: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedParallelEvent {
    id: ParallelStreamEventId,
    source: ParallelEventSourceId,
    line: Line<'static>,
}

impl ProjectedParallelEvent {
    pub(super) fn id(&self) -> ParallelStreamEventId {
        self.id
    }

    #[cfg(test)]
    fn source(&self) -> &ParallelEventSourceId {
        &self.source
    }

    pub(super) fn line(&self) -> &Line<'static> {
        &self.line
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelSnapshotObservation {
    subject: String,
    fingerprint: String,
    timestamp_label: String,
    actor: String,
    body: String,
}

impl ParallelSnapshotObservation {
    fn new(
        subject: impl Into<String>,
        fingerprint: impl Into<String>,
        timestamp_label: impl Into<String>,
        actor: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            subject: subject.into(),
            fingerprint: fingerprint.into(),
            timestamp_label: timestamp_label.into(),
            actor: actor.into(),
            body: body.into(),
        }
    }

    fn into_line(self) -> Line<'static> {
        parallel_supervisor_event_line(&self.timestamp_label, &self.actor, &self.body)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelEventStreamState {
    stream_generation: u64,
    next_ordinal: u64,
    next_local_correlation: u64,
    next_observed_revision: u64,
    workspace: Option<String>,
    events: Arc<[ProjectedParallelEvent]>,
    last_runtime_sequence_seen: Option<i64>,
    observed_snapshot_fingerprints: HashMap<String, String>,
}

impl Default for ParallelEventStreamState {
    fn default() -> Self {
        Self {
            stream_generation: 0,
            next_ordinal: 0,
            next_local_correlation: 0,
            next_observed_revision: 0,
            workspace: None,
            events: Arc::from(Vec::<ProjectedParallelEvent>::new()),
            last_runtime_sequence_seen: None,
            observed_snapshot_fingerprints: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelEventStreamSnapshot {
    generation: u64,
    first_ordinal: u64,
    events: Arc<[ProjectedParallelEvent]>,
}

impl Default for ParallelEventStreamSnapshot {
    fn default() -> Self {
        ParallelEventStreamState::default().snapshot()
    }
}

impl ParallelEventStreamSnapshot {
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn first_ordinal(&self) -> u64 {
        self.first_ordinal
    }

    pub(super) fn events(&self) -> &[ProjectedParallelEvent] {
        &self.events
    }

    #[cfg(test)]
    pub(super) fn live_events(&self) -> &[ProjectedParallelEvent] {
        let live_start = self
            .events
            .len()
            .saturating_sub(MAX_PARALLEL_SUPERVISOR_EVENTS);
        &self.events[live_start..]
    }

    #[cfg(test)]
    pub(super) fn live_lines(&self) -> Vec<Line<'static>> {
        self.live_events()
            .iter()
            .map(|event| event.line().clone())
            .collect()
    }
}

impl ParallelEventStreamState {
    pub(super) fn push_now(&mut self, actor: impl Into<String>, body: impl Into<String>) {
        self.push_local(
            Utc::now().format("%H:%M:%S").to_string(),
            actor.into(),
            body.into(),
        );
    }

    fn push_local(&mut self, timestamp_label: String, actor: String, body: String) {
        let correlation = self.next_local_correlation;
        self.next_local_correlation = self
            .next_local_correlation
            .checked_add(1)
            .expect("parallel local correlation exhausted");
        self.append(
            ParallelEventSourceId::LocalAccepted {
                operation_kind: actor.clone(),
                correlation,
            },
            timestamp_label,
            actor,
            body,
        );
    }

    fn append(
        &mut self,
        source: ParallelEventSourceId,
        timestamp_label: String,
        actor: String,
        body: String,
    ) {
        let id = ParallelStreamEventId {
            stream_generation: self.stream_generation,
            ordinal: self.next_ordinal,
        };
        self.next_ordinal = self
            .next_ordinal
            .checked_add(1)
            .expect("parallel stream ordinal exhausted");
        let mut events = self.events.to_vec();
        events.push(ProjectedParallelEvent {
            id,
            source,
            line: parallel_supervisor_event_line(&timestamp_label, &actor, &body),
        });
        let expired = events.len().saturating_sub(MAX_PARALLEL_EVENT_WINDOW);
        if expired > 0 {
            events.drain(..expired);
        }
        self.events = Arc::from(events);
    }

    pub(super) fn snapshot(&self) -> ParallelEventStreamSnapshot {
        ParallelEventStreamSnapshot {
            generation: self.stream_generation,
            first_ordinal: self
                .events
                .first()
                .map_or(self.next_ordinal, |event| event.id().ordinal()),
            events: Arc::clone(&self.events),
        }
    }

    #[cfg(test)]
    pub(super) fn lines(&self) -> Vec<Line<'static>> {
        self.snapshot().live_lines()
    }

    #[cfg(test)]
    pub(super) fn window_lines(&self) -> Vec<Line<'static>> {
        self.events
            .iter()
            .map(|event| event.line().clone())
            .collect()
    }

    pub(super) fn record_snapshot_stream_from_supervisor_snapshot(
        &mut self,
        snapshot: &ParallelModeSupervisorSnapshot,
        language: TuiLanguage,
    ) {
        let workspace_path = snapshot.workspace_path.as_str();
        self.observe_workspace(workspace_path);
        for observation in parallel_supervisor_snapshot_stream_events(snapshot, language) {
            let previous = self
                .observed_snapshot_fingerprints
                .get(&observation.subject)
                .cloned();
            if previous.as_deref() == Some(observation.fingerprint.as_str()) {
                continue;
            }
            self.observed_snapshot_fingerprints
                .insert(observation.subject.clone(), observation.fingerprint.clone());
            let observed_revision = self.next_observed_revision;
            self.next_observed_revision = self
                .next_observed_revision
                .checked_add(1)
                .expect("parallel snapshot observation revision exhausted");
            self.append(
                ParallelEventSourceId::ObservedStateTransition {
                    subject: observation.subject,
                    previous,
                    current: observation.fingerprint,
                    observed_revision,
                },
                observation.timestamp_label,
                observation.actor,
                observation.body,
            );
        }
    }

    pub(super) fn record_runtime_feed_from_supervisor_snapshot(
        &mut self,
        snapshot: &ParallelModeSupervisorSnapshot,
    ) {
        let workspace_path = snapshot.workspace_path.as_str();
        self.observe_workspace(workspace_path);
        self.record_runtime_feed_entries(workspace_path, &snapshot.distributor.runtime_event_feed);
    }

    fn record_runtime_feed_entries(
        &mut self,
        workspace: &str,
        entries: &[ParallelModeRuntimeEventFeedEntry],
    ) {
        let Some(latest_sequence) = entries.iter().map(|entry| entry.sequence).max() else {
            return;
        };
        let Some(previous_sequence) = self.last_runtime_sequence_seen else {
            self.last_runtime_sequence_seen = Some(latest_sequence);
            return;
        };

        let mut entries = entries
            .iter()
            .filter(|entry| entry.sequence > previous_sequence)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.sequence);
        entries.dedup_by_key(|entry| entry.sequence);
        for entry in entries {
            self.append(
                ParallelEventSourceId::Authority {
                    workspace: workspace.to_string(),
                    sequence: entry.sequence,
                },
                compact_stream_timestamp_label(&entry.recorded_at),
                "Supervisor".to_string(),
                format!(
                    "{}:{} {} / rev {} / {}",
                    display_runtime_event_label(&entry.projection_kind),
                    entry.projection_key,
                    display_runtime_event_label(&entry.event_kind),
                    entry.observed_planning_revision,
                    truncate_event_text(&entry.summary, 76)
                ),
            );
        }
        self.last_runtime_sequence_seen = Some(previous_sequence.max(latest_sequence));
    }

    fn observe_workspace(&mut self, workspace: &str) {
        match self.workspace.as_deref() {
            None => {
                self.workspace = Some(workspace.to_string());
            }
            Some(current) if current == workspace => {}
            Some(_) => {
                self.stream_generation = self
                    .stream_generation
                    .checked_add(1)
                    .expect("parallel stream generation exhausted");
                self.next_ordinal = 0;
                self.next_observed_revision = 0;
                self.workspace = Some(workspace.to_string());
                self.events = Arc::from(Vec::<ProjectedParallelEvent>::new());
                self.last_runtime_sequence_seen = None;
                self.observed_snapshot_fingerprints.clear();
            }
        }
    }

    #[cfg(test)]
    pub(super) fn push_for_test(
        &mut self,
        timestamp_label: impl Into<String>,
        actor: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.push_local(timestamp_label.into(), actor.into(), body.into());
    }
}

pub(super) fn parallel_supervisor_snapshot_stream_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
    language: TuiLanguage,
) -> Vec<Line<'static>> {
    parallel_supervisor_snapshot_stream_events(snapshot, language)
        .into_iter()
        .map(ParallelSnapshotObservation::into_line)
        .collect()
}

fn parallel_supervisor_snapshot_stream_events(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
    language: TuiLanguage,
) -> Vec<ParallelSnapshotObservation> {
    let mut events = Vec::new();

    if let Some(notice) = supervisor_snapshot.top_notice.as_deref() {
        let fingerprint = format!("top_notice|{notice}");
        let notice = truncate_event_text(notice, 96);
        events.push(ParallelSnapshotObservation::new(
            "top_notice",
            fingerprint,
            "--:--:--",
            "Supervisor",
            language.parallel_board_refreshed(&notice),
        ));
    }

    for slot in &supervisor_snapshot.pool.slots {
        if !matches!(
            slot.state,
            ParallelModePoolSlotState::Idle | ParallelModePoolSlotState::Missing
        ) {
            let owner_label = truncate_event_text(&slot.owner_label, 56);
            events.push(ParallelSnapshotObservation::new(
                format!("slot|{}", slot.slot_id),
                format!(
                    "slot|{}|{}|{}",
                    slot.slot_id,
                    slot.state.label(),
                    slot.owner_label
                ),
                "--:--:--",
                "Pool",
                language.pool_slot_state(&slot.slot_id, slot.state.label(), &owner_label),
            ));
        }
    }

    for entry in &supervisor_snapshot.roster.entries {
        let task_title = truncate_event_text(&entry.task_title, 52);
        let state_label = display_supersession_state_label(&entry.state_label);
        let summary = truncate_event_text(&entry.latest_summary, 72);
        events.push(ParallelSnapshotObservation::new(
            format!("roster|{}", entry.agent_id),
            format!(
                "roster|{}|{}|{}|{}|{}",
                entry.agent_id,
                entry.task_title,
                entry.slot_id,
                entry.state_label,
                entry.latest_summary
            ),
            "--:--:--",
            format!("Agent {}", entry.agent_id),
            language.agent_roster_state(&task_title, &entry.slot_id, &state_label, &summary),
        ));
    }

    if let Some(detail) = supervisor_snapshot.detail.session.as_ref() {
        for history in &detail.history {
            events.push(ParallelSnapshotObservation::new(
                format!("history|{}|{}", detail.agent_id, history.timestamp),
                format!(
                    "history|{}|{}|{}|{}",
                    history.timestamp, detail.agent_id, history.state_label, history.summary
                ),
                compact_stream_timestamp_label(&history.timestamp),
                parallel_history_actor(&history.state_label, &detail.agent_id),
                parallel_history_summary(detail, &history.state_label, &history.summary, language),
            ));
        }

        let current_already_recorded = detail.history.last().is_some_and(|history| {
            history.state_label == detail.state_label && history.timestamp == detail.updated_at
        });
        if !current_already_recorded {
            events.push(ParallelSnapshotObservation::new(
                format!("current|{}", detail.agent_id),
                format!(
                    "current|{}|{}|{}|{}",
                    detail.updated_at, detail.agent_id, detail.state_label, detail.latest_summary
                ),
                compact_stream_timestamp_label(&detail.updated_at),
                parallel_history_actor(&detail.state_label, &detail.agent_id),
                parallel_history_summary(
                    detail,
                    &detail.state_label,
                    &detail.latest_summary,
                    language,
                ),
            ));
        }
    }

    for item in &supervisor_snapshot.distributor.queue_items {
        let task_title = truncate_event_text(&item.task_title, 52);
        let branch_name = truncate_event_text(&item.branch_name, 40);
        let integration_note = truncate_event_text(&item.integration_note, 72);
        let subject = item.identity.as_ref().map_or_else(
            || {
                format!(
                    "queue|{}|{}|{}",
                    item.source_agent, item.task_title, item.branch_name
                )
            },
            |identity| format!("queue|{}", identity.queue_item_id),
        );
        events.push(ParallelSnapshotObservation::new(
            subject,
            format!(
                "queue|{}|{}|{}|{}|{}",
                item.source_agent,
                item.task_title,
                item.queue_state.label(),
                item.branch_name,
                item.integration_note
            ),
            "--:--:--",
            "Distributor",
            language.distributor_queue_item(
                &task_title,
                item.queue_state.label(),
                &branch_name,
                &integration_note,
            ),
        ));
    }

    for entry in &supervisor_snapshot.distributor.completion_feed {
        let stage_label = display_runtime_event_label(&entry.stage_label);
        let summary = truncate_event_text(&entry.summary, 88);
        events.push(ParallelSnapshotObservation::new(
            format!("completion|{}", entry.stage_label),
            format!("completion|{}|{}", entry.stage_label, entry.summary),
            "--:--:--",
            "Ledger",
            language.ledger_stage_record(&stage_label, &summary),
        ));
    }

    let orchestrator = &supervisor_snapshot.distributor.orchestrator_status;
    if let Some(reason) = orchestrator.blocked_reason.as_deref() {
        let fingerprint = format!("orchestrator_blocked|{reason}");
        let reason = truncate_event_text(reason, 88);
        events.push(ParallelSnapshotObservation::new(
            "orchestrator_blocked",
            fingerprint,
            "--:--:--",
            "Orchestrator",
            language.integration_blocked(&reason),
        ));
    }
    if let Some(reason) = orchestrator.slot_return_wait_reason.as_deref() {
        let fingerprint = format!("slot_return_wait|{reason}");
        let reason = truncate_event_text(reason, 88);
        events.push(ParallelSnapshotObservation::new(
            "slot_return_wait",
            fingerprint,
            "--:--:--",
            "Orchestrator",
            language.slot_return_withheld(&reason),
        ));
    }

    events
}

pub(super) fn parallel_supervisor_event_line(
    timestamp: &str,
    actor: &str,
    body: &str,
) -> Line<'static> {
    let actor_style = parallel_supervisor_actor_style(actor);
    let body_style = parallel_supervisor_body_style(actor, body);
    Line::from(vec![
        Span::styled(format!("[{timestamp}] "), AkraTheme::subtle()),
        Span::styled(format!("{actor}: "), actor_style),
        Span::styled(body.to_string(), body_style),
    ])
}

fn parallel_supervisor_actor_style(actor: &str) -> Style {
    if actor == PARALLEL_SUPERVISOR_OPERATOR_ACTOR {
        return AkraTheme::shortcut();
    }
    match actor {
        "Ledger" => AkraTheme::brand(),
        "Orchestrator" => AkraTheme::danger().add_modifier(Modifier::BOLD),
        "Distributor" | "Pool" => AkraTheme::accent().add_modifier(Modifier::BOLD),
        "Supervisor" => AkraTheme::warning().add_modifier(Modifier::BOLD),
        _ if actor.starts_with("Agent ") => AkraTheme::tool().add_modifier(Modifier::BOLD),
        _ => AkraTheme::muted().add_modifier(Modifier::BOLD),
    }
}

fn parallel_supervisor_body_style(actor: &str, body: &str) -> Style {
    if actor == PARALLEL_SUPERVISOR_OPERATOR_ACTOR {
        return AkraTheme::shortcut();
    }
    if actor == "Ledger" || actor == "Orchestrator" || actor == "Distributor" {
        return parallel_supervisor_actor_style(actor);
    }
    if actor.starts_with("Agent ") {
        return AkraTheme::tool();
    }
    if is_important_parallel_message(body) {
        return parallel_supervisor_actor_style(actor);
    }
    Style::default()
}

fn is_important_parallel_message(body: &str) -> bool {
    const IMPORTANT_MARKERS: [&str; 8] = [
        "blocked",
        "failed",
        "failure",
        "error",
        "completed",
        "complete",
        "merged",
        "official",
    ];
    let body = body.to_ascii_lowercase();
    IMPORTANT_MARKERS.iter().any(|marker| body.contains(marker))
        || TUI_LOCALIZED_IMPORTANT_MARKERS
            .iter()
            .any(|marker| body.contains(marker))
}

fn display_runtime_event_label(label: &str) -> String {
    label.replace('_', " ")
}

fn display_supersession_state_label(state_label: &str) -> String {
    match state_label {
        "reported_complete" => "reported".to_string(),
        "commit_ready" => "official".to_string(),
        other => other.replace('_', " "),
    }
}

fn parallel_history_actor(state_label: &str, agent_id: &str) -> String {
    match state_label {
        "assigned" | "starting" | "merge_queued" | "pushing" | "pr_pending" | "merge_pending"
        | "integrating" | "merged" | "cleanup_pending" | "cleaned" => "Distributor".to_string(),
        "ledger_refreshing" | "commit_ready" => "Ledger".to_string(),
        "failed" | "official_refresh_recovery_needed" => "Supervisor".to_string(),
        _ => format!("Agent {agent_id}"),
    }
}

fn parallel_history_summary(
    detail: &ParallelModeAgentSessionDetailSnapshot,
    state_label: &str,
    fallback_summary: &str,
    language: TuiLanguage,
) -> String {
    let task_title = truncate_event_text(&detail.task_title, 52);
    let fallback_summary = truncate_event_text(fallback_summary, 96);
    language.parallel_history_summary(
        state_label,
        &task_title,
        &detail.slot_id,
        &detail.agent_id,
        &fallback_summary,
    )
}

fn compact_stream_timestamp_label(timestamp: &str) -> String {
    let trimmed = timestamp.trim();
    if trimmed.is_empty() {
        return "--:--:--".to_string();
    }

    let time_part = trimmed
        .split_once('T')
        .map(|(_, time)| time)
        .unwrap_or(trimmed)
        .trim_end_matches('Z');

    let mut parts = time_part.split(':');
    let Some(hour) = parts.next() else {
        return "--:--:--".to_string();
    };
    let Some(minute) = parts.next() else {
        return "--:--:--".to_string();
    };
    let Some(second) = parts.next() else {
        return format!("{hour}:{minute}:00");
    };
    let second = second
        .split_once('.')
        .map(|(head, _)| head)
        .unwrap_or(second);
    format!("{hour}:{minute}:{second}")
}

#[cfg(test)]
fn rendered_tail_start_index(lines: &[Line<'static>], live_tail_rows: usize, width: u16) -> usize {
    rendered_parallel_event_tail_start_index(lines, live_tail_rows, width)
}

#[cfg(test)]
fn rendered_parallel_event_tail_start_index(
    lines: &[Line<'static>],
    live_tail_rows: usize,
    width: u16,
) -> usize {
    if lines.is_empty() {
        return 0;
    }
    if live_tail_rows == 0 || width == 0 {
        return lines.len();
    }

    let total_rendered_rows = lines
        .iter()
        .map(|line| rendered_parallel_event_line_rows(line, width))
        .sum::<usize>();
    let minimum_scroll_offset = total_rendered_rows.saturating_sub(live_tail_rows);
    if minimum_scroll_offset == 0 {
        return 0;
    }

    let mut rendered_rows_before_entry = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let rendered_rows_after_entry =
            rendered_rows_before_entry + rendered_parallel_event_line_rows(line, width);
        if minimum_scroll_offset < rendered_rows_after_entry {
            /*
             * Ratatui can only scroll by rendered row, but the durable/live
             * contract splits at logical events. If the viewport boundary lands
             * inside an event, keep that whole event durable and begin the live
             * suffix at the next event so no wrapped rows disappear below the
             * inline panel.
             */
            return index + 1;
        }
        if minimum_scroll_offset == rendered_rows_after_entry {
            return index + 1;
        }
        rendered_rows_before_entry = rendered_rows_after_entry;
    }

    lines.len()
}

pub(super) fn rendered_parallel_event_line_rows(line: &Line<'_>, width: u16) -> usize {
    if width == 0 {
        return 0;
    }
    Paragraph::new(vec![line.clone()])
        .wrap(Wrap { trim: false })
        .line_count(width)
}

fn truncate_event_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let keep = max_chars.saturating_sub(3);
    let mut truncated = trimmed.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}

impl super::NativeTuiApp {
    pub(super) fn record_parallel_supervisor_event(
        &mut self,
        actor: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.shell.parallel_event_stream.push_now(actor, body);
    }

    #[cfg(test)]
    pub(crate) fn parallel_supervisor_event_lines(&self) -> Vec<Line<'static>> {
        self.shell.parallel_event_stream.lines()
    }

    #[cfg(test)]
    pub(crate) fn parallel_supervisor_event_scrollback_lines(&self) -> Vec<Line<'static>> {
        self.shell.parallel_event_stream.window_lines()
    }

    pub(super) fn record_parallel_supervisor_snapshot_for_stream(
        &mut self,
        snapshot: &ParallelModeSupervisorSnapshot,
    ) {
        self.shell
            .parallel_event_stream
            .record_snapshot_stream_from_supervisor_snapshot(snapshot, self.shell.tui_language);
        self.shell
            .parallel_event_stream
            .record_runtime_feed_from_supervisor_snapshot(snapshot);
    }

    #[cfg(test)]
    pub(crate) fn push_parallel_supervisor_event_for_test(
        &mut self,
        timestamp_label: impl Into<String>,
        actor: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.shell
            .parallel_event_stream
            .push_for_test(timestamp_label, actor, body);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Modifier, Style};

    use super::*;
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterSnapshot, ParallelModeCompletionFeedEntry,
        ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot,
        ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorState,
    };

    #[test]
    fn user_prompt_line_uses_you_label_with_user_emphasis() {
        let mut log = ParallelEventStreamState::default();

        log.push_for_test(
            "11:31:18",
            PARALLEL_SUPERVISOR_OPERATOR_ACTOR,
            "안녕하세요?",
        );

        let lines = log.lines();
        assert_eq!(lines[0].to_string(), "[11:31:18] You: 안녕하세요?");
        assert_eq!(lines[0].spans[1].content.as_ref(), "You: ");
        assert_eq!(lines[0].spans[1].style, AkraTheme::shortcut());
        assert!(
            lines[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(lines[0].spans[2].content.as_ref(), "안녕하세요?");
        assert_eq!(lines[0].spans[2].style, AkraTheme::shortcut());
        assert!(
            lines[0].spans[2]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn non_user_event_line_highlights_actor_label() {
        let mut log = ParallelEventStreamState::default();

        log.push_for_test("11:31:19", "Task Intake", "task generation started.");

        let lines = log.lines();
        assert_eq!(
            lines[0].to_string(),
            "[11:31:19] Task Intake: task generation started."
        );
        assert_eq!(lines[0].spans[1].content.as_ref(), "Task Intake: ");
        assert_eq!(
            lines[0].spans[1].style,
            AkraTheme::muted().add_modifier(Modifier::BOLD)
        );
        assert_eq!(lines[0].spans[2].style, Style::default());
    }

    #[test]
    fn important_event_line_highlights_message_body() {
        let mut log = ParallelEventStreamState::default();

        log.push_for_test("11:31:20", "Ledger", "official completion을 확인했습니다.");

        let lines = log.lines();
        assert_eq!(
            lines[0].to_string(),
            "[11:31:20] Ledger: official completion을 확인했습니다."
        );
        assert_eq!(lines[0].spans[1].content.as_ref(), "Ledger: ");
        assert_ne!(lines[0].spans[1].style, Style::default());
        assert_ne!(lines[0].spans[2].style, Style::default());
    }

    #[test]
    fn log_keeps_recent_events_without_reformatting_on_read() {
        let mut log = ParallelEventStreamState::default();

        for index in 0..(MAX_PARALLEL_SUPERVISOR_EVENTS + 4) {
            log.push_for_test("11:45:02", "Supervisor", format!("event-{index:03}"));
        }

        let rendered = log.lines();
        assert_eq!(rendered.len(), MAX_PARALLEL_SUPERVISOR_EVENTS);
        assert!(!rendered[0].to_string().contains("event-000"));
        assert!(
            rendered[0].to_string().contains("event-004"),
            "oldest retained event should be the first item after capping"
        );
    }

    #[test]
    fn durable_tail_boundary_keeps_wrapped_event_out_of_scrollback_until_complete() {
        let lines = vec![Line::from("123456 123456 123456"), Line::from("tail event")];

        assert_eq!(
            rendered_parallel_event_line_rows(&lines[0], 10),
            3,
            "event row measurement must match Ratatui word wrapping rather than raw width division"
        );
        assert_eq!(
            rendered_tail_start_index(&lines, 3, 10),
            1,
            "an event that does not fully fit in the live suffix must remain durable"
        );
        assert_eq!(
            rendered_tail_start_index(&lines, 1, 10),
            1,
            "boundary exactly after a wrapped event may start at the next event"
        );
    }

    #[test]
    fn event_log_keeps_runtime_feed_append_only_after_baseline() {
        let mut log = ParallelEventStreamState::default();

        log.push_for_test(
            "11:45:02",
            PARALLEL_SUPERVISOR_OPERATOR_ACTOR,
            "안녕하세요?",
        );
        log.record_runtime_feed_entries(
            "/tmp/root",
            &[
                runtime_feed_entry(2, "slot_lease", "slot-2", "slot_lease_upsert"),
                runtime_feed_entry(1, "session_detail", "slot-1", "session_detail_upsert"),
            ],
        );
        assert_eq!(
            log.window_lines()
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>(),
            vec!["[11:45:02] You: 안녕하세요?".to_string()],
            "initial runtime feed should establish the append baseline without backfilling old DB events"
        );
        log.record_runtime_feed_entries(
            "/tmp/root",
            &[
                runtime_feed_entry(
                    3,
                    "distributor_queue",
                    "queue-1",
                    "distributor_queue_upsert",
                ),
                runtime_feed_entry(2, "slot_lease", "slot-2", "slot_lease_upsert"),
                runtime_feed_entry(1, "session_detail", "slot-1", "session_detail_upsert"),
            ],
        );
        let before_tail = log
            .window_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let live_tail = log
            .lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(before_tail.matches("session detail:slot-1").count(), 0);
        assert_eq!(before_tail.matches("slot lease:slot-2").count(), 0);
        assert_eq!(before_tail.matches("distributor queue:queue-1").count(), 1);
        assert_eq!(live_tail.matches("session detail:slot-1").count(), 0);
        assert_eq!(live_tail.matches("slot lease:slot-2").count(), 0);
        assert_eq!(live_tail.matches("distributor queue:queue-1").count(), 1);
        let durable_operator_index = before_tail
            .find("You: 안녕하세요?")
            .expect("operator event should stay in durable stream history");
        let durable_runtime_index = before_tail
            .find("distributor queue:queue-1")
            .expect("new runtime event should append to durable stream history");
        assert!(durable_operator_index < durable_runtime_index);
        let live_operator_index = live_tail
            .find("You: 안녕하세요?")
            .expect("operator event should stay in live stream");
        let live_runtime_index = live_tail
            .find("distributor queue:queue-1")
            .expect("new runtime event should append to live stream");
        assert!(live_operator_index < live_runtime_index);

        for index in 0..MAX_PARALLEL_EVENT_WINDOW {
            log.push_for_test("11:45:03", "Supervisor", format!("tail-{index:03}"));
        }

        let rendered = log
            .window_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(log.window_lines().len(), MAX_PARALLEL_EVENT_WINDOW);
        assert!(!rendered.contains("Parallel Event Stream"));
        assert!(!rendered.contains("slot lease:slot-2"));
        assert!(rendered.contains("tail-000"));
        assert_eq!(rendered.matches("tail-511").count(), 1);
    }

    #[test]
    fn authority_sequence_rejects_stale_and_duplicate_entries() {
        let mut stream = ParallelEventStreamState::default();

        stream.record_runtime_feed_entries(
            "/tmp/root",
            &[runtime_feed_entry(
                1,
                "session_detail",
                "slot-1",
                "session_detail_upsert",
            )],
        );
        stream.record_runtime_feed_entries(
            "/tmp/root",
            &[
                runtime_feed_entry(3, "slot_lease", "slot-3", "slot_lease_upsert"),
                runtime_feed_entry(2, "slot_lease", "slot-2", "slot_lease_upsert"),
                runtime_feed_entry(3, "slot_lease", "slot-3", "slot_lease_upsert"),
            ],
        );
        stream.record_runtime_feed_entries(
            "/tmp/root",
            &[
                runtime_feed_entry(1, "session_detail", "slot-1", "session_detail_upsert"),
                runtime_feed_entry(2, "slot_lease", "slot-2", "slot_lease_upsert"),
            ],
        );

        let snapshot = stream.snapshot();
        assert_eq!(snapshot.events().len(), 2);
        assert_eq!(
            snapshot
                .events()
                .iter()
                .map(|event| event.id().ordinal())
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(
            snapshot
                .events()
                .iter()
                .filter_map(|event| match event.source() {
                    ParallelEventSourceId::Authority { sequence, .. } => Some(*sequence),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn mixed_sources_share_one_total_order() {
        let mut stream = ParallelEventStreamState::default();
        stream.push_for_test("11:45:02", "You", "start work");
        stream.record_snapshot_stream_from_supervisor_snapshot(
            &snapshot_with_notice("/tmp/root", "board ready"),
            TuiLanguage::English,
        );
        stream.record_runtime_feed_entries(
            "/tmp/root",
            &[runtime_feed_entry(
                1,
                "session_detail",
                "slot-1",
                "session_detail_upsert",
            )],
        );
        stream.record_runtime_feed_entries(
            "/tmp/root",
            &[
                runtime_feed_entry(1, "session_detail", "slot-1", "session_detail_upsert"),
                runtime_feed_entry(2, "slot_lease", "slot-2", "slot_lease_upsert"),
            ],
        );

        let snapshot = stream.snapshot();
        assert_eq!(
            snapshot
                .events()
                .iter()
                .map(|event| event.id().ordinal())
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(matches!(
            snapshot.events()[0].source(),
            ParallelEventSourceId::LocalAccepted { .. }
        ));
        assert!(matches!(
            snapshot.events()[1].source(),
            ParallelEventSourceId::ObservedStateTransition { .. }
        ));
        assert!(matches!(
            snapshot.events()[2].source(),
            ParallelEventSourceId::Authority { sequence: 2, .. }
        ));
    }

    #[test]
    fn snapshot_observation_retains_a_to_b_to_a() {
        let mut stream = ParallelEventStreamState::default();

        for notice in ["state A", "state B", "state A"] {
            stream.record_snapshot_stream_from_supervisor_snapshot(
                &snapshot_with_notice("/tmp/root", notice),
                TuiLanguage::English,
            );
        }

        let snapshot = stream.snapshot();
        assert_eq!(snapshot.events().len(), 3);
        assert_eq!(
            snapshot
                .events()
                .iter()
                .map(|event| event.id().ordinal())
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(
            snapshot
                .events()
                .iter()
                .filter(|event| event.line().to_string().contains("state A"))
                .count(),
            2
        );
        assert!(matches!(
            snapshot.events()[2].source(),
            ParallelEventSourceId::ObservedStateTransition {
                previous: Some(previous),
                current,
                ..
            } if previous.ends_with("state B") && current.ends_with("state A")
        ));
    }

    #[test]
    fn workspace_a_b_a_creates_new_stream_generations() {
        let mut stream = ParallelEventStreamState::default();

        stream.record_snapshot_stream_from_supervisor_snapshot(
            &snapshot_with_notice("/tmp/a", "A first"),
            TuiLanguage::English,
        );
        let first = stream.snapshot();
        stream.record_snapshot_stream_from_supervisor_snapshot(
            &snapshot_with_notice("/tmp/b", "B"),
            TuiLanguage::English,
        );
        let second = stream.snapshot();
        stream.record_snapshot_stream_from_supervisor_snapshot(
            &snapshot_with_notice("/tmp/a", "A again"),
            TuiLanguage::English,
        );
        let third = stream.snapshot();

        assert_eq!(
            [first.generation(), second.generation(), third.generation()],
            [0, 1, 2]
        );
        for snapshot in [&first, &second, &third] {
            assert_eq!(snapshot.first_ordinal(), 0);
            assert_eq!(snapshot.events().len(), 1);
            assert_eq!(
                snapshot.events()[0].id().stream_generation(),
                snapshot.generation()
            );
        }
        assert!(third.events()[0].line().to_string().contains("A again"));
        assert!(!third.events()[0].line().to_string().contains("A first"));
    }

    #[test]
    fn canonical_window_retains_one_bounded_identity_range() {
        let mut stream = ParallelEventStreamState::default();

        for index in 0..(MAX_PARALLEL_EVENT_WINDOW + 3) {
            stream.push_for_test("11:45:03", "Supervisor", format!("event-{index:03}"));
        }

        let snapshot = stream.snapshot();
        assert!(
            Arc::ptr_eq(&stream.events, &snapshot.events),
            "capturing a stream snapshot should share the immutable canonical window"
        );
        assert_eq!(snapshot.events().len(), MAX_PARALLEL_EVENT_WINDOW);
        assert_eq!(snapshot.first_ordinal(), 3);
        assert_eq!(
            snapshot.events().last().map(ProjectedParallelEvent::id),
            Some(ParallelStreamEventId {
                stream_generation: 0,
                ordinal: (MAX_PARALLEL_EVENT_WINDOW + 2) as u64,
            })
        );
        assert_eq!(snapshot.live_events().len(), MAX_PARALLEL_SUPERVISOR_EVENTS);
    }

    #[test]
    fn snapshot_stream_uses_selected_language_for_system_copy() {
        let snapshot = localized_snapshot();

        let english = parallel_supervisor_snapshot_stream_lines(&snapshot, TuiLanguage::English)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let korean = parallel_supervisor_snapshot_stream_lines(&snapshot, TuiLanguage::Korean)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(english.contains("parallel board refreshed. control tower is live"));
        assert!(english.contains("reported stage record: no agent results reported yet"));
        assert!(!english.contains("상태를 갱신했습니다"));
        assert!(korean.contains("parallel board 상태를 갱신했습니다. control tower is live"));
        assert!(korean.contains("reported 단계 기록: no agent results reported yet"));
    }

    #[test]
    fn localized_snapshot_stream_dedupes_with_language_independent_keys() {
        let snapshot = localized_snapshot();
        let mut log = ParallelEventStreamState::default();

        log.record_snapshot_stream_from_supervisor_snapshot(&snapshot, TuiLanguage::Korean);
        let first = log.snapshot();
        log.record_snapshot_stream_from_supervisor_snapshot(&snapshot, TuiLanguage::English);
        let second = log.snapshot();

        let rendered = log
            .window_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rendered.matches("control tower is live").count(), 1);
        assert_eq!(rendered.matches("no agent results reported yet").count(), 1);
        assert_eq!(
            first
                .events()
                .iter()
                .map(ProjectedParallelEvent::id)
                .collect::<Vec<_>>(),
            second
                .events()
                .iter()
                .map(ProjectedParallelEvent::id)
                .collect::<Vec<_>>(),
            "language-specific copy must not create a new event identity"
        );
    }

    fn runtime_feed_entry(
        sequence: i64,
        projection_kind: &str,
        projection_key: &str,
        event_kind: &str,
    ) -> ParallelModeRuntimeEventFeedEntry {
        ParallelModeRuntimeEventFeedEntry::new(
            sequence,
            event_kind,
            projection_kind,
            projection_key,
            238,
            format!("runtime {projection_kind} stored"),
            "2026-05-13T11:45:05.330826165+00:00",
        )
    }

    fn localized_snapshot() -> ParallelModeSupervisorSnapshot {
        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            "/tmp/root",
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(
                Vec::new(),
                vec![ParallelModeCompletionFeedEntry::new(
                    "reported",
                    "no agent results reported yet",
                )],
                "idle",
                "queue idle",
            ),
            Some("control tower is live".to_string()),
        )
    }

    fn snapshot_with_notice(workspace: &str, notice: &str) -> ParallelModeSupervisorSnapshot {
        let mut snapshot = localized_snapshot();
        snapshot.workspace_path = workspace.to_string();
        snapshot.distributor.completion_feed.clear();
        snapshot.top_notice = Some(notice.to_string());
        snapshot
    }
}
