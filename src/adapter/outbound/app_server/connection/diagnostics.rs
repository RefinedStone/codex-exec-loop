use std::collections::VecDeque;

use anyhow::anyhow;

use crate::adapter::outbound::app_server::protocol::{
    AppServerNotification, sort_and_dedup_warnings,
};

/*
 * fatal stderr는 최종 anyhow error에 붙여야 하므로 무한히 쌓지 않는다.
 * 마지막 몇 줄이면 transport close나 app-server panic의 실제 원인을 보기에 충분하고,
 * 오래된 noise가 최신 실패 메시지를 밀어내지 않게 한다.
 */
const MAX_FATAL_STDERR_LINES: usize = 4;
pub(super) const MAX_PENDING_NOTIFICATIONS: usize = 4_096;
pub(super) const MAX_PENDING_NOTIFICATION_BYTES: usize = 64 * 1024 * 1024;
const MAX_WARNING_ENTRIES: usize = 128;
const MAX_DIAGNOSTIC_TEXT_BYTES: usize = 4 * 1024;

/*
 * app-server notification은 request/response JSON-RPC 흐름 밖에서 도착한다.
 * turn stream consumer가 열려 있으면 pop_front로 즉시 소비하지만, request가 끝날 때까지
 * 소비자가 없으면 "응답 이후 흘러온 알림"으로 경고화해야 하므로 FIFO queue로 보관한다.
 */
#[derive(Default)]
pub(super) struct PendingNotifications {
    entries: VecDeque<(AppServerNotification, usize)>,
    encoded_bytes: usize,
}

impl PendingNotifications {
    pub(super) fn try_push(&mut self, notification: AppServerNotification) -> bool {
        /*
         * connection read loop가 notification line을 만나면 순서를 보존해 뒤에 붙인다.
         * stream 소비자는 같은 순서로 pop해 app-server delta를 turn event로 환원한다.
         */
        let encoded_bytes = notification.encoded_size_bytes();
        if self.entries.len() >= MAX_PENDING_NOTIFICATIONS
            || encoded_bytes > MAX_PENDING_NOTIFICATION_BYTES.saturating_sub(self.encoded_bytes)
        {
            return false;
        }
        self.encoded_bytes += encoded_bytes;
        self.entries.push_back((notification, encoded_bytes));
        true
    }

    pub(super) fn pop_front(&mut self) -> Option<AppServerNotification> {
        /*
         * turn stream이 notification을 기다릴 때 가장 오래된 항목부터 가져간다.
         * queue가 비어 있으면 connection loop가 다음 line을 읽어 새 notification을 채운다.
         */
        let (notification, encoded_bytes) = self.entries.pop_front()?;
        self.encoded_bytes = self.encoded_bytes.saturating_sub(encoded_bytes);
        Some(notification)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn drain_warning_texts(&mut self) -> Vec<String> {
        /*
         * response가 끝났는데 consumer가 없던 notification은 정상 turn delta로 해석할 곳이 없다.
         * 버리면 원인 추적이 어려워지므로 protocol helper의 warning copy로 바꿔 diagnostics에 합류시킨다.
         */
        self.encoded_bytes = 0;
        self.entries
            .drain(..)
            .map(|(notification, _)| {
                notification
                    .warning_text("after the response completed without a turn stream consumer")
            })
            .collect()
    }
}

/*
 * ConnectionDiagnostics는 app-server 연결 하나가 수집한 비정상 신호의 임시 저장소다.
 * warning은 성공 응답과 함께 operator notice로 반환될 수 있고, fatal stderr는 실패 error에 붙어
 * "request failed"만 보이는 상황을 막는다.
 */
#[derive(Default)]
pub(super) struct ConnectionDiagnostics {
    warnings: VecDeque<String>,
    fatal_stderr: VecDeque<String>,
    dropped_warning_count: u64,
}

impl ConnectionDiagnostics {
    pub(super) fn record_warning(&mut self, warning: String) {
        /*
         * 빈 문자열 warning은 UI에 아무 정보도 주지 않고 dedup 대상만 늘린다.
         * trim으로 의미 없는 line을 먼저 걸러 connection caller가 별도 검증을 반복하지 않게 한다.
         */
        let warning = bounded_diagnostic_text(warning.trim());
        if warning.is_empty() {
            return;
        }
        if self.warnings.len() >= MAX_WARNING_ENTRIES {
            self.warnings.pop_front();
            self.dropped_warning_count = self.dropped_warning_count.saturating_add(1);
        }
        self.warnings.push_back(warning);
    }

    pub(super) fn record_warnings<I>(&mut self, warnings: I)
    where
        I: IntoIterator<Item = String>,
    {
        /*
         * pending notification drain처럼 여러 warning이 한 번에 들어오는 경로를 위한 bulk helper다.
         * 단건 record_warning과 같은 empty-filter 정책을 유지한다.
         */
        for warning in warnings {
            self.record_warning(warning);
        }
    }

    pub(super) fn record_stderr(&mut self, line: String) {
        /*
         * app-server stderr는 두 종류다. 운영자가 알아야 할 일반 warning과,
         * request 실패 error에 붙어야 하는 fatal context다. stdout JSON parsing과 별도로
         * stderr line을 이 함수에 모아 두면 connection loop는 transport 상태 처리에 집중할 수 있다.
         */
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }

        if is_fatal_stderr_line(trimmed) {
            self.fatal_stderr
                .push_back(bounded_diagnostic_text(trimmed));
            /*
             * fatal context는 최신 원인 중심으로 유지한다. 작은 bounded deque에서 오래된
             * 원소 하나만 제거해 최신 오류가 항상 남도록 한다.
             */
            if self.fatal_stderr.len() > MAX_FATAL_STDERR_LINES {
                self.fatal_stderr.pop_front();
            }
        } else {
            self.record_warning(trimmed.to_string());
        }
    }

    pub(super) fn take_warnings(&mut self) -> Vec<String> {
        /*
         * warning은 connection caller가 한 번 가져가 operator notice로 전파한다.
         * 반환 직전 정렬/dedup해 같은 stderr나 delayed notification이 화면을 반복해서 차지하지 않게 한다.
         */
        if self.dropped_warning_count > 0 {
            if self.warnings.len() >= MAX_WARNING_ENTRIES {
                self.warnings.pop_front();
                self.dropped_warning_count = self.dropped_warning_count.saturating_add(1);
            }
            self.warnings.push_back(format!(
                "app-server diagnostics dropped {} warning entries after reaching the bounded history limit",
                self.dropped_warning_count
            ));
            self.dropped_warning_count = 0;
        }
        let mut warnings = std::mem::take(&mut self.warnings)
            .into_iter()
            .collect::<Vec<_>>();
        sort_and_dedup_warnings(&mut warnings);
        warnings
    }

    pub(super) fn error(&self, message: impl Into<String>) -> anyhow::Error {
        /*
         * request 실패 메시지에 최근 fatal stderr를 덧붙여 app-server 쪽 panic, fatal transport close,
         * backtrace 같은 원인을 한 줄 error에서도 볼 수 있게 한다. 일반 warning은 성공/실패와 별개로
         * take_warnings 경로에서 다루므로 여기에는 fatal bucket만 붙인다.
         */
        let mut message = message.into();
        if !self.fatal_stderr.is_empty() {
            message.push_str(" / recent stderr: ");
            message.push_str(
                &self
                    .fatal_stderr
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" | "),
            );
        }
        anyhow!(message)
    }
}

fn bounded_diagnostic_text(text: &str) -> String {
    if text.len() <= MAX_DIAGNOSTIC_TEXT_BYTES {
        return text.to_string();
    }

    let suffix = format!("...[truncated from {} bytes]", text.len());
    let mut end = MAX_DIAGNOSTIC_TEXT_BYTES.saturating_sub(suffix.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &text[..end], suffix)
}

fn is_fatal_stderr_line(line: &str) -> bool {
    /*
     * app-server가 항상 구조화된 stderr severity를 주지는 않으므로 keyword 기반으로 분류한다.
     * prefix와 infix를 함께 보는 이유는 Rust panic/backtrace, Node-style error, shell failure copy가
     * 서로 다른 형식으로 섞여 들어오기 때문이다.
     */
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

    use super::{
        ConnectionDiagnostics, MAX_DIAGNOSTIC_TEXT_BYTES, MAX_PENDING_NOTIFICATION_BYTES,
        MAX_PENDING_NOTIFICATIONS, MAX_WARNING_ENTRIES, PendingNotifications,
    };
    use crate::adapter::outbound::app_server::protocol::AppServerNotification;

    #[test]
    fn pending_notifications_become_warnings_if_no_turn_stream_consumes_them() {
        /*
         * notification은 보통 stream consumer가 turn delta로 소비한다. 이 테스트는 consumer 없이
         * response가 끝난 예외 경로에서 queue가 silent drop 대신 warning text를 만드는지 고정한다.
         */
        let mut pending = PendingNotifications::default();
        assert!(
            pending.try_push(
                AppServerNotification::from_value(json!({
                    "method": "item/agentMessage/delta",
                    "params": {
                        "turnId": "turn-1"
                    }
                }))
                .expect("notification should parse"),
            )
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
    fn pending_notification_limit_rejects_the_first_overflow_without_dropping_history() {
        let mut pending = PendingNotifications::default();
        for index in 0..MAX_PENDING_NOTIFICATIONS {
            assert!(
                pending.try_push(
                    AppServerNotification::from_value(json!({
                        "method": "item/agentMessage/delta",
                        "params": { "turnId": "turn-1", "delta": index.to_string() }
                    }))
                    .expect("notification should parse"),
                )
            );
        }

        assert!(
            !pending.try_push(
                AppServerNotification::from_value(json!({
                    "method": "item/agentMessage/delta",
                    "params": { "turnId": "turn-1", "delta": "overflow" }
                }))
                .expect("notification should parse"),
            )
        );
        assert_eq!(
            pending.drain_warning_texts().len(),
            MAX_PENDING_NOTIFICATIONS
        );
    }

    #[test]
    fn pending_notification_byte_budget_rejects_overflow_without_losing_queued_entries() {
        let mut pending = PendingNotifications::default();
        let first = AppServerNotification::from_value(json!({
            "method": "item/agentMessage/delta",
            "params": { "turnId": "turn-1", "delta": "first" }
        }))
        .expect("notification should parse");
        let first_bytes = first.encoded_size_bytes();
        assert!(pending.try_push(first));

        pending.encoded_bytes = MAX_PENDING_NOTIFICATION_BYTES - 1;
        assert!(
            !pending.try_push(
                AppServerNotification::from_value(json!({
                    "method": "item/agentMessage/delta",
                    "params": { "turnId": "turn-1", "delta": "overflow" }
                }))
                .expect("notification should parse"),
            )
        );
        pending.encoded_bytes = first_bytes;
        assert_eq!(
            pending.pop_front().map(|entry| entry.method().to_string()),
            Some("item/agentMessage/delta".to_string())
        );
        assert_eq!(pending.encoded_bytes, 0);
    }

    #[test]
    fn warning_history_is_bounded_and_reports_all_dropped_entries_once() {
        let mut diagnostics = ConnectionDiagnostics::default();
        for index in 0..(MAX_WARNING_ENTRIES + 7) {
            diagnostics.record_warning(format!("warning-{index}"));
        }

        let warnings = diagnostics.take_warnings();
        assert_eq!(warnings.len(), MAX_WARNING_ENTRIES);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("dropped 8 warning entries"))
        );
        assert!(diagnostics.take_warnings().is_empty());
    }

    #[test]
    fn stderr_and_warning_text_are_truncated_to_the_diagnostic_byte_limit() {
        let mut diagnostics = ConnectionDiagnostics::default();
        diagnostics.record_stderr("w".repeat(MAX_DIAGNOSTIC_TEXT_BYTES + 1));
        diagnostics.record_stderr(format!(
            "fatal: {}",
            "f".repeat(MAX_DIAGNOSTIC_TEXT_BYTES + 1)
        ));

        let warnings = diagnostics.take_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].len() <= MAX_DIAGNOSTIC_TEXT_BYTES);
        assert!(warnings[0].contains("truncated from"));
        let error = diagnostics.error("failed").to_string();
        let fatal = error
            .strip_prefix("failed / recent stderr: ")
            .expect("fatal stderr should be attached");
        assert!(fatal.len() <= MAX_DIAGNOSTIC_TEXT_BYTES);
        assert!(fatal.contains("truncated from"));
    }

    #[test]
    fn fatal_stderr_is_attached_to_errors_instead_of_warning_bucket() {
        /*
         * fatal stderr는 operator warning 목록에 섞이면 실패 error와 분리되어 원인 파악이 어려워진다.
         * 이 테스트는 fatal line은 error context로, 일반 stderr는 warning bucket으로 갈라지는 계약을 확인한다.
         */
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
