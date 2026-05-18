use std::collections::{HashSet, VecDeque};

use chrono::Utc;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModePoolSlotState,
    ParallelModeRuntimeEventFeedEntry, ParallelModeSupervisorSnapshot,
};

use super::AkraTheme;

const MAX_PARALLEL_SUPERVISOR_EVENTS: usize = 96;
const MAX_PARALLEL_SUPERVISOR_SCROLLBACK_EVENTS: usize = 512;
pub(super) const PARALLEL_SUPERVISOR_OPERATOR_ACTOR: &str = "You";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelSupervisorEventEntry {
    line: Line<'static>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ParallelSupervisorStreamEvent {
    timestamp_label: String,
    actor: String,
    body: String,
}

impl ParallelSupervisorStreamEvent {
    fn new(
        timestamp_label: impl Into<String>,
        actor: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_label: timestamp_label.into(),
            actor: actor.into(),
            body: body.into(),
        }
    }

    fn key(&self) -> String {
        format!("{}|{}|{}", self.timestamp_label, self.actor, self.body)
    }

    fn into_line(self) -> Line<'static> {
        parallel_supervisor_event_line(&self.timestamp_label, &self.actor, &self.body)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ParallelSupervisorEventLog {
    entries: VecDeque<ParallelSupervisorEventEntry>,
    scrollback_entries: VecDeque<ParallelSupervisorEventEntry>,
    runtime_feed_workspace: Option<String>,
    last_runtime_sequence_seen: Option<i64>,
    snapshot_stream_workspace: Option<String>,
    seen_snapshot_stream_events: HashSet<String>,
}

impl ParallelSupervisorEventLog {
    pub(super) fn push_now(&mut self, actor: impl Into<String>, body: impl Into<String>) {
        self.push(
            Utc::now().format("%H:%M:%S").to_string(),
            actor.into(),
            body.into(),
        );
    }

    fn push(&mut self, timestamp_label: String, actor: String, body: String) {
        let entry = ParallelSupervisorEventEntry {
            line: parallel_supervisor_event_line(&timestamp_label, &actor, &body),
        };
        self.scrollback_entries.push_back(entry.clone());
        self.trim_scrollback_entries();
        self.push_live_entry(entry);
    }

    fn push_live_entry(&mut self, entry: ParallelSupervisorEventEntry) {
        self.entries.push_back(entry);
        while self.entries.len() > MAX_PARALLEL_SUPERVISOR_EVENTS {
            self.entries.pop_front();
        }
    }

    pub(super) fn lines(&self) -> Vec<Line<'static>> {
        self.entries
            .iter()
            .map(|entry| entry.line.clone())
            .collect()
    }

    #[cfg(test)]
    pub(super) fn scrollback_lines(&self) -> Vec<Line<'static>> {
        self.scrollback_entries
            .iter()
            .map(|entry| entry.line.clone())
            .collect()
    }

    pub(super) fn scrollback_lines_before_live_tail(
        &self,
        live_tail_lines: usize,
    ) -> Vec<Line<'static>> {
        let durable_len = self
            .scrollback_entries
            .len()
            .saturating_sub(live_tail_lines);
        self.scrollback_entries
            .iter()
            .take(durable_len)
            .map(|entry| entry.line.clone())
            .collect()
    }

    pub(super) fn record_snapshot_stream_from_supervisor_snapshot(
        &mut self,
        snapshot: &ParallelModeSupervisorSnapshot,
    ) {
        let workspace_path = snapshot.workspace_path.as_str();
        if self.snapshot_stream_workspace.as_deref() != Some(workspace_path) {
            self.snapshot_stream_workspace = Some(workspace_path.to_string());
            self.seen_snapshot_stream_events.clear();
        }
        for event in parallel_supervisor_snapshot_stream_events(snapshot) {
            let key = event.key();
            if self.seen_snapshot_stream_events.insert(key) {
                self.push(event.timestamp_label, event.actor, event.body);
            }
        }
    }

    pub(super) fn record_runtime_feed_from_supervisor_snapshot(
        &mut self,
        snapshot: &ParallelModeSupervisorSnapshot,
    ) {
        let workspace_path = snapshot.workspace_path.as_str();
        if self.runtime_feed_workspace.as_deref() != Some(workspace_path) {
            self.runtime_feed_workspace = Some(workspace_path.to_string());
            self.last_runtime_sequence_seen = None;
        }
        self.record_runtime_feed_entries(&snapshot.distributor.runtime_event_feed);
    }

    fn record_runtime_feed_entries(&mut self, entries: &[ParallelModeRuntimeEventFeedEntry]) {
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
        for entry in entries {
            self.push(
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

    fn trim_scrollback_entries(&mut self) {
        while self.scrollback_entries.len() > MAX_PARALLEL_SUPERVISOR_SCROLLBACK_EVENTS {
            self.scrollback_entries.pop_front();
        }
    }

    #[cfg(test)]
    pub(super) fn push_for_test(
        &mut self,
        timestamp_label: impl Into<String>,
        actor: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.push(timestamp_label.into(), actor.into(), body.into());
    }
}

pub(super) fn parallel_supervisor_snapshot_stream_lines(
    snapshot: &ParallelModeSupervisorSnapshot,
) -> Vec<Line<'static>> {
    parallel_supervisor_snapshot_stream_events(snapshot)
        .into_iter()
        .map(ParallelSupervisorStreamEvent::into_line)
        .collect()
}

fn parallel_supervisor_snapshot_stream_events(
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
) -> Vec<ParallelSupervisorStreamEvent> {
    let mut events = Vec::new();

    if let Some(notice) = supervisor_snapshot.top_notice.as_deref() {
        events.push(ParallelSupervisorStreamEvent::new(
            "--:--:--",
            "Supervisor",
            format!(
                "parallel board 상태를 갱신했습니다. {}",
                truncate_event_text(notice, 96)
            ),
        ));
    }

    for slot in &supervisor_snapshot.pool.slots {
        if !matches!(
            slot.state,
            ParallelModePoolSlotState::Idle | ParallelModePoolSlotState::Missing
        ) {
            events.push(ParallelSupervisorStreamEvent::new(
                "--:--:--",
                "Pool",
                format!(
                    "{} 상태는 {}이며 owner는 {}입니다.",
                    slot.slot_id,
                    slot.state.label(),
                    truncate_event_text(&slot.owner_label, 56)
                ),
            ));
        }
    }

    for entry in &supervisor_snapshot.roster.entries {
        events.push(ParallelSupervisorStreamEvent::new(
            "--:--:--",
            format!("Agent {}", entry.agent_id),
            format!(
                "{} 작업이 {}에서 {} 상태입니다. {}",
                truncate_event_text(&entry.task_title, 52),
                entry.slot_id,
                display_supersession_state_label(&entry.state_label),
                truncate_event_text(&entry.latest_summary, 72)
            ),
        ));
    }

    if let Some(detail) = supervisor_snapshot.detail.session.as_ref() {
        for history in &detail.history {
            events.push(ParallelSupervisorStreamEvent::new(
                compact_stream_timestamp_label(&history.timestamp),
                parallel_history_actor(&history.state_label, &detail.agent_id),
                parallel_history_summary(detail, &history.state_label, &history.summary),
            ));
        }

        let current_already_recorded = detail.history.last().is_some_and(|history| {
            history.state_label == detail.state_label && history.timestamp == detail.updated_at
        });
        if !current_already_recorded {
            events.push(ParallelSupervisorStreamEvent::new(
                compact_stream_timestamp_label(&detail.updated_at),
                parallel_history_actor(&detail.state_label, &detail.agent_id),
                parallel_history_summary(detail, &detail.state_label, &detail.latest_summary),
            ));
        }
    }

    for item in &supervisor_snapshot.distributor.queue_items {
        events.push(ParallelSupervisorStreamEvent::new(
            "--:--:--",
            "Distributor",
            format!(
                "{} 결과가 {} 상태로 대기 중입니다. branch {} / {}",
                truncate_event_text(&item.task_title, 52),
                item.queue_state.label(),
                truncate_event_text(&item.branch_name, 40),
                truncate_event_text(&item.integration_note, 72)
            ),
        ));
    }

    for entry in &supervisor_snapshot.distributor.completion_feed {
        events.push(ParallelSupervisorStreamEvent::new(
            "--:--:--",
            "Ledger",
            format!(
                "{} 단계 기록: {}",
                display_runtime_event_label(&entry.stage_label),
                truncate_event_text(&entry.summary, 88)
            ),
        ));
    }

    let orchestrator = &supervisor_snapshot.distributor.orchestrator_status;
    if let Some(reason) = orchestrator.blocked_reason.as_deref() {
        events.push(ParallelSupervisorStreamEvent::new(
            "--:--:--",
            "Orchestrator",
            format!(
                "integration이 차단되었습니다. {}",
                truncate_event_text(reason, 88)
            ),
        ));
    }
    if let Some(reason) = orchestrator.slot_return_wait_reason.as_deref() {
        events.push(ParallelSupervisorStreamEvent::new(
            "--:--:--",
            "Orchestrator",
            format!(
                "slot 반환을 보류했습니다. {}",
                truncate_event_text(reason, 88)
            ),
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
    const IMPORTANT_MARKERS: [&str; 14] = [
        "blocked",
        "failed",
        "failure",
        "error",
        "completed",
        "complete",
        "merged",
        "official",
        "차단",
        "실패",
        "오류",
        "완료",
        "병합",
        "보류",
    ];
    let body = body.to_ascii_lowercase();
    IMPORTANT_MARKERS.iter().any(|marker| body.contains(marker))
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
) -> String {
    let task_title = truncate_event_text(&detail.task_title, 52);
    match state_label {
        "assigned" | "starting" => {
            format!(
                "{}이 {}에게 대여되었습니다.",
                detail.slot_id, detail.agent_id
            )
        }
        "running" => format!("{task_title} 작업을 시작했습니다."),
        "reported_complete" => format!("{task_title} 완료를 보고했습니다."),
        "ledger_refreshing" => format!("{task_title} official completion을 확인하고 있습니다."),
        "commit_ready" => format!("{task_title} 결과를 official completion으로 승인했습니다."),
        "merge_queued" => format!("{task_title} 결과가 distributor queue에 등록되었습니다."),
        "pushing" | "pr_pending" | "merge_pending" | "integrating" => format!(
            "{task_title} delivery 단계가 {}입니다.",
            display_supersession_state_label(state_label)
        ),
        "merged" | "cleanup_pending" | "cleaned" => {
            format!("{task_title} 결과가 prerelease에 반영되었습니다.")
        }
        "failed" => format!("{task_title} 작업이 실패했습니다."),
        "official_refresh_recovery_needed" => {
            format!("{task_title} official completion 복구가 필요합니다.")
        }
        _ => truncate_event_text(fallback_summary, 96),
    }
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
        self.parallel_supervisor_event_log.push_now(actor, body);
    }

    pub(crate) fn parallel_supervisor_event_lines(&self) -> Vec<Line<'static>> {
        self.parallel_supervisor_event_log.lines()
    }

    #[cfg(test)]
    pub(crate) fn parallel_supervisor_event_scrollback_lines(&self) -> Vec<Line<'static>> {
        self.parallel_supervisor_event_log.scrollback_lines()
    }

    pub(crate) fn parallel_supervisor_event_scrollback_lines_before_live_tail(
        &self,
        live_tail_lines: usize,
    ) -> Vec<Line<'static>> {
        self.parallel_supervisor_event_log
            .scrollback_lines_before_live_tail(live_tail_lines)
    }

    pub(super) fn record_parallel_supervisor_snapshot_for_stream(
        &mut self,
        snapshot: &ParallelModeSupervisorSnapshot,
    ) {
        self.parallel_supervisor_event_log
            .record_snapshot_stream_from_supervisor_snapshot(snapshot);
        self.parallel_supervisor_event_log
            .record_runtime_feed_from_supervisor_snapshot(snapshot);
    }

    #[cfg(test)]
    pub(crate) fn push_parallel_supervisor_event_for_test(
        &mut self,
        timestamp_label: impl Into<String>,
        actor: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.parallel_supervisor_event_log
            .push_for_test(timestamp_label, actor, body);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Modifier, Style};

    use super::*;

    #[test]
    fn user_prompt_line_uses_you_label_with_user_emphasis() {
        let mut log = ParallelSupervisorEventLog::default();

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
        let mut log = ParallelSupervisorEventLog::default();

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
        let mut log = ParallelSupervisorEventLog::default();

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
        let mut log = ParallelSupervisorEventLog::default();

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
    fn event_log_keeps_runtime_feed_append_only_after_baseline() {
        let mut log = ParallelSupervisorEventLog::default();

        log.push_for_test(
            "11:45:02",
            PARALLEL_SUPERVISOR_OPERATOR_ACTOR,
            "안녕하세요?",
        );
        log.record_runtime_feed_entries(&[
            runtime_feed_entry(2, "slot_lease", "slot-2", "slot_lease_upsert"),
            runtime_feed_entry(1, "session_detail", "slot-1", "session_detail_upsert"),
        ]);
        assert_eq!(
            log.scrollback_lines()
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>(),
            vec!["[11:45:02] You: 안녕하세요?".to_string()],
            "initial runtime feed should establish the append baseline without backfilling old DB events"
        );
        log.record_runtime_feed_entries(&[
            runtime_feed_entry(
                3,
                "distributor_queue",
                "queue-1",
                "distributor_queue_upsert",
            ),
            runtime_feed_entry(2, "slot_lease", "slot-2", "slot_lease_upsert"),
            runtime_feed_entry(1, "session_detail", "slot-1", "session_detail_upsert"),
        ]);
        let before_tail = log
            .scrollback_lines()
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

        for index in 0..MAX_PARALLEL_SUPERVISOR_SCROLLBACK_EVENTS {
            log.push_for_test("11:45:03", "Supervisor", format!("tail-{index:03}"));
        }

        let rendered = log
            .scrollback_lines()
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            log.scrollback_lines().len(),
            MAX_PARALLEL_SUPERVISOR_SCROLLBACK_EVENTS
        );
        assert!(!rendered.contains("Parallel Event Stream"));
        assert!(!rendered.contains("slot lease:slot-2"));
        assert!(rendered.contains("tail-000"));
        assert_eq!(rendered.matches("tail-511").count(), 1);
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
}
