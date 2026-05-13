use std::collections::VecDeque;

use chrono::Utc;
use ratatui::text::{Line, Span};

use super::AkraTheme;

const MAX_PARALLEL_SUPERVISOR_EVENTS: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelSupervisorEventEntry {
    timestamp_label: String,
    actor: String,
    body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ParallelSupervisorEventLog {
    entries: VecDeque<ParallelSupervisorEventEntry>,
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
        self.entries.push_back(ParallelSupervisorEventEntry {
            timestamp_label,
            actor,
            body,
        });
        while self.entries.len() > MAX_PARALLEL_SUPERVISOR_EVENTS {
            self.entries.pop_front();
        }
    }

    pub(super) fn lines(&self) -> Vec<Line<'static>> {
        self.entries
            .iter()
            .map(|entry| {
                parallel_supervisor_event_line(&entry.timestamp_label, &entry.actor, &entry.body)
            })
            .collect()
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

fn parallel_supervisor_event_line(timestamp: &str, actor: &str, body: &str) -> Line<'static> {
    if actor == "You" {
        return Line::from(vec![
            Span::raw(format!("[{timestamp}] ")),
            Span::styled(actor.to_string(), AkraTheme::shortcut()),
            Span::raw(format!(": {body}")),
        ]);
    }

    Line::from(format!("[{timestamp}] {actor}: {body}"))
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
    use ratatui::style::{Color, Modifier, Style};

    use super::*;

    #[test]
    fn user_prompt_line_uses_you_label_with_user_emphasis() {
        let mut log = ParallelSupervisorEventLog::default();

        log.push_for_test("11:31:18", "You", "안녕하세요?");

        let lines = log.lines();
        assert_eq!(lines[0].to_string(), "[11:31:18] You: 안녕하세요?");
        assert_eq!(lines[0].spans[1].content.as_ref(), "You");
        assert_eq!(lines[0].spans[1].style.fg, Some(Color::Yellow));
        assert!(
            lines[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn non_user_event_line_keeps_plain_log_style() {
        let mut log = ParallelSupervisorEventLog::default();

        log.push_for_test("11:31:19", "Task Intake", "task generation started.");

        let lines = log.lines();
        assert_eq!(
            lines[0].to_string(),
            "[11:31:19] Task Intake: task generation started."
        );
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].style, Style::default());
    }
}
