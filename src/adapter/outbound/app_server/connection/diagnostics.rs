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

/*
 * app-server notification은 request/response JSON-RPC 흐름 밖에서 도착한다.
 * turn stream consumer가 열려 있으면 pop_front로 즉시 소비하지만, request가 끝날 때까지
 * 소비자가 없으면 "응답 이후 흘러온 알림"으로 경고화해야 하므로 FIFO queue로 보관한다.
 */
#[derive(Default)]
pub(super) struct PendingNotifications {
    entries: VecDeque<AppServerNotification>,
}

impl PendingNotifications {
    pub(super) fn push(&mut self, notification: AppServerNotification) {
        /*
         * connection read loop가 notification line을 만나면 순서를 보존해 뒤에 붙인다.
         * stream 소비자는 같은 순서로 pop해 app-server delta를 turn event로 환원한다.
         */
        self.entries.push_back(notification);
    }

    pub(super) fn pop_front(&mut self) -> Option<AppServerNotification> {
        /*
         * turn stream이 notification을 기다릴 때 가장 오래된 항목부터 가져간다.
         * queue가 비어 있으면 connection loop가 다음 line을 읽어 새 notification을 채운다.
         */
        self.entries.pop_front()
    }

    pub(super) fn drain_warning_texts(&mut self) -> Vec<String> {
        /*
         * response가 끝났는데 consumer가 없던 notification은 정상 turn delta로 해석할 곳이 없다.
         * 버리면 원인 추적이 어려워지므로 protocol helper의 warning copy로 바꿔 diagnostics에 합류시킨다.
         */
        self.entries
            .drain(..)
            .map(|notification| {
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
    warnings: Vec<String>,
    fatal_stderr: Vec<String>,
}

impl ConnectionDiagnostics {
    pub(super) fn record_warning(&mut self, warning: String) {
        /*
         * 빈 문자열 warning은 UI에 아무 정보도 주지 않고 dedup 대상만 늘린다.
         * trim으로 의미 없는 line을 먼저 걸러 connection caller가 별도 검증을 반복하지 않게 한다.
         */
        if !warning.trim().is_empty() {
            self.warnings.push(warning);
        }
    }

    pub(super) fn record_warnings<I>(&mut self, warnings: I)
    where
        I: IntoIterator<Item = String>,
    {
        /*
         * pending notification drain처럼 여러 warning이 한 번에 들어오는 경로를 위한 bulk helper다.
         * 단건 record_warning과 같은 empty-filter 정책을 유지한다.
         */
        self.warnings.extend(
            warnings
                .into_iter()
                .filter(|warning| !warning.trim().is_empty()),
        );
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
            self.fatal_stderr.push(trimmed.to_string());
            /*
             * fatal context는 최신 원인 중심으로 유지한다. VecDeque까지 쓰지 않고 작은 Vec의
             * 앞 원소를 제거하는 편이 이 제한된 크기에서는 더 단순하다.
             */
            if self.fatal_stderr.len() > MAX_FATAL_STDERR_LINES {
                self.fatal_stderr.remove(0);
            }
        } else {
            self.warnings.push(trimmed.to_string());
        }
    }

    pub(super) fn take_warnings(&mut self) -> Vec<String> {
        /*
         * warning은 connection caller가 한 번 가져가 operator notice로 전파한다.
         * 반환 직전 정렬/dedup해 같은 stderr나 delayed notification이 화면을 반복해서 차지하지 않게 한다.
         */
        sort_and_dedup_warnings(&mut self.warnings);
        std::mem::take(&mut self.warnings)
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
            message.push_str(&self.fatal_stderr.join(" | "));
        }
        anyhow!(message)
    }
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

    use super::{ConnectionDiagnostics, PendingNotifications};
    use crate::adapter::outbound::app_server::protocol::AppServerNotification;

    #[test]
    fn pending_notifications_become_warnings_if_no_turn_stream_consumes_them() {
        /*
         * notification은 보통 stream consumer가 turn delta로 소비한다. 이 테스트는 consumer 없이
         * response가 끝난 예외 경로에서 queue가 silent drop 대신 warning text를 만드는지 고정한다.
         */
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
