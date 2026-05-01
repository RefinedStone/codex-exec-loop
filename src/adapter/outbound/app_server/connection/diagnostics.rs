use std::collections::VecDeque;

use anyhow::anyhow;

use crate::adapter::outbound::app_server::protocol::{
    AppServerNotification, sort_and_dedup_warnings,
};

const MAX_FATAL_STDERR_LINES: usize = 4;

#[derive(Default)]
pub(super) struct PendingNotifications {
    entries: VecDeque<AppServerNotification>,
}

impl PendingNotifications {
    pub(super) fn push(&mut self, notification: AppServerNotification) {
        self.entries.push_back(notification);
    }

    pub(super) fn pop_front(&mut self) -> Option<AppServerNotification> {
        self.entries.pop_front()
    }

    pub(super) fn drain_warning_texts(&mut self) -> Vec<String> {
        self.entries
            .drain(..)
            .map(|notification| {
                notification
                    .warning_text("after the response completed without a turn stream consumer")
            })
            .collect()
    }
}

#[derive(Default)]
pub(super) struct ConnectionDiagnostics {
    warnings: Vec<String>,
    fatal_stderr: Vec<String>,
}

impl ConnectionDiagnostics {
    pub(super) fn record_warning(&mut self, warning: String) {
        if !warning.trim().is_empty() {
            self.warnings.push(warning);
        }
    }

    pub(super) fn record_warnings<I>(&mut self, warnings: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.warnings.extend(
            warnings
                .into_iter()
                .filter(|warning| !warning.trim().is_empty()),
        );
    }

    pub(super) fn record_stderr(&mut self, line: String) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }

        if is_fatal_stderr_line(trimmed) {
            self.fatal_stderr.push(trimmed.to_string());
            if self.fatal_stderr.len() > MAX_FATAL_STDERR_LINES {
                self.fatal_stderr.remove(0);
            }
        } else {
            self.warnings.push(trimmed.to_string());
        }
    }

    pub(super) fn take_warnings(&mut self) -> Vec<String> {
        sort_and_dedup_warnings(&mut self.warnings);
        std::mem::take(&mut self.warnings)
    }

    pub(super) fn error(&self, message: impl Into<String>) -> anyhow::Error {
        let mut message = message.into();
        if !self.fatal_stderr.is_empty() {
            message.push_str(" / recent stderr: ");
            message.push_str(&self.fatal_stderr.join(" | "));
        }
        anyhow!(message)
    }
}

fn is_fatal_stderr_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();

    lower.starts_with("fatal")
        || lower.starts_with("panic")
        || lower.starts_with("error")
        || lower.contains(" fatal ")
        || lower.contains(" panic")
        || lower.contains(" error")
        || lower.contains("failed")
        || lower.contains("backtrace")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ConnectionDiagnostics, PendingNotifications};
    use crate::adapter::outbound::app_server::protocol::AppServerNotification;

    #[test]
    fn pending_notifications_become_warnings_if_no_turn_stream_consumes_them() {
        let mut pending = PendingNotifications::default();
        pending.push(
            AppServerNotification::from_value(json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "turnId": "turn-1"
                }
            }))
            .expect("notification should parse"),
        );

        assert_eq!(
            pending.drain_warning_texts(),
            vec![
                "app-server sent notification `item/agentMessage/delta` after the response completed without a turn stream consumer"
                    .to_string()
            ]
        );
    }

    #[test]
    fn fatal_stderr_is_attached_to_errors_instead_of_warning_bucket() {
        let mut diagnostics = ConnectionDiagnostics::default();
        diagnostics.record_stderr("fatal: transport closed".to_string());
        diagnostics.record_stderr("workspace prompt missing".to_string());

        assert_eq!(
            diagnostics.take_warnings(),
            vec!["workspace prompt missing".to_string()]
        );
        assert!(
            diagnostics
                .error("turn failed")
                .to_string()
                .contains("fatal: transport closed")
        );
    }
}
