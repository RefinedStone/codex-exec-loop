use std::collections::VecDeque;

use chrono::Utc;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::domain::parallel_mode::{
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ParallelSupervisorEventLog {
    entries: VecDeque<ParallelSupervisorEventEntry>,
    scrollback_entries: VecDeque<ParallelSupervisorEventEntry>,
    runtime_feed_workspace: Option<String>,
    last_runtime_sequence_seen: Option<i64>,
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

    fn push_live_only(&mut self, timestamp_label: String, actor: String, body: String) {
        self.push_live_entry(ParallelSupervisorEventEntry {
            line: parallel_supervisor_event_line(&timestamp_label, &actor, &body),
        });
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

    pub(super) fn scrollback_lines(&self) -> Vec<Line<'static>> {
        self.scrollback_entries
            .iter()
            .map(|entry| entry.line.clone())
            .collect()
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
            self.push_live_only(
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

    pub(crate) fn parallel_supervisor_event_scrollback_lines(&self) -> Vec<Line<'static>> {
        self.parallel_supervisor_event_log.scrollback_lines()
    }

    pub(super) fn record_parallel_supervisor_runtime_feed_for_scrollback(
        &mut self,
        snapshot: &ParallelModeSupervisorSnapshot,
    ) {
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
    fn event_log_keeps_runtime_feed_live_only_after_baseline() {
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
        assert_eq!(before_tail.matches("distributor queue:queue-1").count(), 0);
        assert_eq!(live_tail.matches("session detail:slot-1").count(), 0);
        assert_eq!(live_tail.matches("slot lease:slot-2").count(), 0);
        assert_eq!(live_tail.matches("distributor queue:queue-1").count(), 1);
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
        assert!(!rendered.contains("distributor queue:queue-1"));
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
