//! Gateway가 구독한 스레드의 폴더와 턴 상태를 기억한다 (#2850).
//! 구독 응답은 요청 ID로 구분한다. thread/read 같은 일회 조회는 이후 알림을 보장하지 않는다.
use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;

use crate::application::port::inbound::turn_activity_port::TurnActivityObserverPort;

enum PendingThreadRequest {
    Subscribe { expected_thread: Option<String> },
    Unsubscribe { thread_id: String },
}

/// 클라이언트와 upstream 고리가 함께 쓴다. 접속 종료 때 observer가 이 접속의 기록만 지운다.
pub(super) struct TurnActivityTracker {
    observer: Box<dyn TurnActivityObserverPort>,
    pending: Mutex<HashMap<String, PendingThreadRequest>>,
}

impl TurnActivityTracker {
    pub(super) fn new(observer: Box<dyn TurnActivityObserverPort>) -> Self {
        Self {
            observer,
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn client_message(&self, text: &str) {
        if !text.contains("\"thread/") {
            return;
        }
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let Some(id) = message
            .get("id")
            .filter(|id| id.is_string() || id.is_number())
        else {
            return;
        };
        let thread_id = message["params"]["threadId"].as_str().map(str::to_owned);
        let request = match message["method"].as_str() {
            Some("thread/start" | "thread/fork") => PendingThreadRequest::Subscribe {
                expected_thread: None,
            },
            Some("thread/resume") if thread_id.is_some() => PendingThreadRequest::Subscribe {
                expected_thread: thread_id,
            },
            Some("thread/unsubscribe") => {
                let Some(thread_id) = thread_id else { return };
                PendingThreadRequest::Unsubscribe { thread_id }
            }
            _ => return,
        };
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.to_string(), request);
    }

    pub(super) fn upstream_message(&self, text: &str) {
        // 출력 delta는 풀지 않는다. 요청 응답, 턴 알림과 스레드 알림만 필요하다.
        if !["\"id\"", "\"turn/", "\"thread/"]
            .iter()
            .any(|marker| text.contains(marker))
        {
            return;
        }
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let method = message["method"].as_str();
        if method.is_none() {
            let Some(id) = message.get("id") else { return };
            let pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&id.to_string());
            if message.get("error").is_some() {
                return;
            }
            match pending {
                Some(PendingThreadRequest::Subscribe { expected_thread }) => {
                    let thread = &message["result"]["thread"];
                    if expected_thread
                        .as_deref()
                        .is_none_or(|expected| thread["id"].as_str() == Some(expected))
                    {
                        self.observe_thread(thread);
                    }
                }
                Some(PendingThreadRequest::Unsubscribe { thread_id }) => {
                    if matches!(
                        message["result"]["status"].as_str(),
                        Some("unsubscribed" | "notSubscribed" | "notLoaded")
                    ) {
                        self.observer.forget_thread(&thread_id);
                    }
                }
                None => {}
            }
            return;
        }
        let params = &message["params"];
        if method == Some("thread/started") {
            self.observe_thread(&params["thread"]);
        }
        let Some(thread_id) = params["threadId"].as_str() else {
            return;
        };
        match method {
            Some("turn/started") => self.observer.turn_running(thread_id, true),
            Some("turn/completed") => self.observer.turn_running(thread_id, false),
            Some("thread/status/changed") => {
                self.observer.turn_running(
                    thread_id,
                    params["status"]["type"].as_str() == Some("active"),
                );
            }
            Some("thread/closed" | "thread/archived" | "thread/deleted") => {
                self.observer.forget_thread(thread_id);
            }
            _ => {}
        }
    }

    fn observe_thread(&self, thread: &Value) {
        if let (Some(id), Some(cwd)) = (thread["id"].as_str(), thread["cwd"].as_str()) {
            let running = thread["status"]["type"]
                .as_str()
                .map(|status| status == "active");
            self.observer.thread_snapshot(id, cwd, running);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::application::port::inbound::turn_activity_port::TurnActivityPort;
    use crate::application::service::turn_activity::TurnActivityRegistry;

    #[test]
    fn upstream_turn_and_thread_messages_update_running_folders() {
        let registry = Arc::new(TurnActivityRegistry::default());
        let tracker = TurnActivityTracker::new(registry.clone().observe_connection());
        tracker.client_message(r#"{"id":2,"method":"thread/resume","params":{"threadId":"t1"}}"#);
        tracker.upstream_message(
            r#"{"id":2,"result":{"thread":{"id":"t1","cwd":"/repo/app","status":{"type":"idle"}}}}"#,
        );
        tracker.upstream_message(
            r#"{"method":"thread/started","params":{"thread":{"id":"t2","cwd":"/other"}}}"#,
        );
        tracker.upstream_message(
            r#"{"method":"turn/started","params":{"threadId":"t1","turn":{"id":"u1"}}}"#,
        );
        tracker.upstream_message(
            r#"{"method":"thread/status/changed","params":{"threadId":"t2","status":{"type":"active","activeFlags":[]}}}"#,
        );
        assert_eq!(registry.running_folders(), ["/other", "/repo/app"]);
        tracker.upstream_message(
            r#"{"method":"turn/completed","params":{"threadId":"t1","turn":{"id":"u1"}}}"#,
        );
        tracker.upstream_message(
            r#"{"method":"thread/status/changed","params":{"threadId":"t2","status":{"type":"idle"}}}"#,
        );
        tracker.upstream_message(
            r#"{"method":"item/agentMessage/delta","params":{"threadId":"t1","delta":"\"turn/started\""}}"#,
        );
        tracker.upstream_message("not json \"turn/");
        assert!(registry.running_folders().is_empty());
    }

    #[test]
    fn active_thread_snapshot_blocks_before_the_next_notification() {
        for method in ["thread/start", "thread/resume", "thread/fork"] {
            let registry = Arc::new(TurnActivityRegistry::default());
            let tracker = TurnActivityTracker::new(registry.clone().observe_connection());
            tracker.client_message(
                &serde_json::json!({"id":2,"method":method,"params":{"threadId":"active"}})
                    .to_string(),
            );
            tracker.upstream_message(
                r#"{"id":2,"result":{"thread":{"id":"active","cwd":"/repo/src","status":{"type":"active","activeFlags":["waitingOnApproval"]}}}}"#,
            );
            assert_eq!(registry.running_folders(), ["/repo/src"], "{method}");
            tracker.upstream_message(
                r#"{"method":"turn/completed","params":{"threadId":"active","turn":{"id":"turn-1"}}}"#,
            );
            assert!(registry.running_folders().is_empty());
        }
    }

    #[test]
    fn closed_thread_forgets_only_its_own_connection_record() {
        let registry = Arc::new(TurnActivityRegistry::default());
        let tracker = TurnActivityTracker::new(registry.clone().observe_connection());
        let other = registry.observer();
        for method in ["thread/closed", "thread/archived", "thread/deleted"] {
            tracker.observer.thread_snapshot("closed", "/repo", None);
            tracker.observer.turn_running("closed", true);
            other.thread_folder("closed", "/repo");
            other.turn_running("closed", true);
            other.thread_folder("other", "/other");
            other.turn_running("other", true);
            tracker.upstream_message(
                &serde_json::json!({"method":method,"params":{"threadId":"closed"}}).to_string(),
            );
            assert_eq!(registry.running_folders(), ["/other", "/repo"]);
            other.turn_running("closed", false);
            assert_eq!(registry.running_folders(), ["/other"], "{method}");
        }
    }

    #[test]
    fn only_successful_unsubscribe_responses_forget_the_running_thread() {
        for status in ["unsubscribed", "notSubscribed", "notLoaded"] {
            let registry = Arc::new(TurnActivityRegistry::default());
            let tracker = TurnActivityTracker::new(registry.clone().observe_connection());
            tracker.observer.thread_snapshot("t1", "/repo", None);
            tracker.observer.turn_running("t1", true);
            tracker.client_message(
                r#"{"id":1,"method":"thread/unsubscribe","params":{"threadId":"t1"}}"#,
            );
            assert_eq!(registry.running_folders(), ["/repo"]);
            tracker.upstream_message(
                r#"{"id":1,"error":{"code":-32603,"message":"not unsubscribed"}}"#,
            );
            assert_eq!(registry.running_folders(), ["/repo"]);
            tracker.client_message(
                r#"{"id":"1","method":"thread/unsubscribe","params":{"threadId":"t1"}}"#,
            );
            // JSON-RPC string and numeric IDs are distinct.
            tracker.upstream_message(
                &serde_json::json!({"id":1,"result":{"status":status}}).to_string(),
            );
            assert_eq!(registry.running_folders(), ["/repo"]);
            tracker.upstream_message(
                &serde_json::json!({"id":"1","result":{"status":status}}).to_string(),
            );
            assert!(registry.running_folders().is_empty(), "{status}");
        }
    }

    #[test]
    fn a_one_off_read_does_not_seed_unobserved_turns_or_override_a_subscription() {
        let registry = Arc::new(TurnActivityRegistry::default());
        let tracker = TurnActivityTracker::new(registry.clone().observe_connection());
        tracker.client_message(r#"{"id":1,"method":"thread/resume","params":{"threadId":"live"}}"#);
        tracker.client_message(
            r#"{"id":2,"method":"thread/read","params":{"threadId":"unobserved"}}"#,
        );
        tracker.upstream_message(
            r#"{"id":2,"result":{"thread":{"id":"unobserved","cwd":"/other","status":{"type":"active"}}}}"#,
        );
        assert!(registry.running_folders().is_empty());
        tracker.upstream_message(
            r#"{"id":1,"result":{"thread":{"id":"live","cwd":"/repo","status":{"type":"active"}}}}"#,
        );
        assert_eq!(registry.running_folders(), ["/repo"]);
        tracker.client_message(r#"{"id":3,"method":"thread/read","params":{"threadId":"live"}}"#);
        tracker.upstream_message(
            r#"{"id":3,"result":{"thread":{"id":"live","cwd":"/repo","status":{"type":"idle"}}}}"#,
        );
        assert_eq!(registry.running_folders(), ["/repo"]);
        drop(tracker);
        assert!(registry.running_folders().is_empty());
    }
}
