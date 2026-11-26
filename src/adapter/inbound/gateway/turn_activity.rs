//! upstream 이 클라이언트에 보내는 메시지에서 스레드 폴더와 턴의 시작·끝만 읽는다 (#2850). 메시지는 바꾸지 않고
//! 그대로 넘긴다. 이 정보를 담지 않는 메시지는 JSON 으로 풀지 않는다.
use serde_json::Value;

use crate::application::service::turn_activity::TurnActivityObserver;

/// 이 낱말이 없는 메시지는 풀지 않는다. 턴 알림, 스레드 상태 알림, 스레드를 담은 응답과 알림이 해당한다.
const MARKERS: [&str; 3] = ["\"turn/", "\"thread/status/changed\"", "\"thread\""];

pub(super) fn observe_upstream(observer: &TurnActivityObserver, text: &str) {
    if !MARKERS.iter().any(|marker| text.contains(marker)) {
        return;
    }
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let params = &message["params"];
    // thread/start, thread/resume, thread/fork 의 응답과 thread/started 알림은 스레드와 그 폴더를 담는다.
    for thread in [&message["result"]["thread"], &params["thread"]] {
        if let (Some(id), Some(cwd)) = (thread["id"].as_str(), thread["cwd"].as_str()) {
            observer.thread_folder(id, cwd);
        }
    }
    let Some(thread_id) = params["threadId"].as_str() else {
        return;
    };
    match message["method"].as_str() {
        Some("turn/started") => observer.turn_running(thread_id, true),
        Some("turn/completed") => observer.turn_running(thread_id, false),
        Some("thread/status/changed") => {
            observer.turn_running(
                thread_id,
                params["status"]["type"].as_str() == Some("active"),
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::application::service::turn_activity::TurnActivityRegistry;

    #[test]
    fn upstream_turn_and_thread_messages_update_running_folders() {
        let registry = Arc::new(TurnActivityRegistry::default());
        let observer = registry.observer();
        observe_upstream(
            &observer,
            r#"{"id":2,"result":{"thread":{"id":"t1","cwd":"/repo/app","status":{"type":"idle"}}}}"#,
        );
        observe_upstream(
            &observer,
            r#"{"method":"thread/started","params":{"thread":{"id":"t2","cwd":"/other"}}}"#,
        );
        observe_upstream(
            &observer,
            r#"{"method":"turn/started","params":{"threadId":"t1","turn":{"id":"u1"}}}"#,
        );
        observe_upstream(
            &observer,
            r#"{"method":"thread/status/changed","params":{"threadId":"t2","status":{"type":"active","activeFlags":[]}}}"#,
        );
        assert_eq!(registry.running_folders(), ["/other", "/repo/app"]);

        observe_upstream(
            &observer,
            r#"{"method":"turn/completed","params":{"threadId":"t1","turn":{"id":"u1"}}}"#,
        );
        observe_upstream(
            &observer,
            r#"{"method":"thread/status/changed","params":{"threadId":"t2","status":{"type":"idle"}}}"#,
        );
        // Text that only mentions a marker inside a string is parsed but changes nothing.
        observe_upstream(
            &observer,
            r#"{"method":"item/agentMessage/delta","params":{"threadId":"t1","delta":"\"turn/started\""}}"#,
        );
        observe_upstream(&observer, "not json \"turn/");
        assert!(registry.running_folders().is_empty());
    }
}
