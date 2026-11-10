//! Gateway 의 `akra/context/*` 처리이다 (plan 22). 스레드에 나온 외부 문서를 장부에서 읽어 준다. 수집은
//! 블로킹(app-server 조회와 SQLite)이므로 요청마다 블로킹 작업으로 돌린다. 접속의 읽기 고리는 기다리지 않는다.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::recorder::{RecordDirection, SessionRecording};
use crate::application::port::inbound::work_context_port::WorkContextPort;

pub(super) const CONTEXT_METHOD_PREFIX: &str = "akra/context/";
/// params `{ threadId }` -> result `{ resources: [ThreadResource] }`
const CONTEXT_THREAD_METHOD: &str = "akra/context/thread";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const JSON_RPC_INVALID_PARAMS: i64 = -32602;
const WORK_CONTEXT_ERROR: i64 = -32000;

#[derive(Deserialize)]
struct RequestEnvelope {
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadParams {
    thread_id: String,
}

/// 요청 하나를 블로킹 작업으로 처리하고 답을 접속의 보내기 함으로 보낸다.
pub(super) fn spawn_request(
    context: Arc<dyn WorkContextPort>,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
    recording: Option<Arc<SessionRecording>>,
) {
    tokio::spawn(async move {
        let fallback_id = id.clone();
        let response =
            tokio::task::spawn_blocking(move || respond(context.as_ref(), id, &method, &text))
                .await
                .unwrap_or_else(|_| {
                    error_response(
                        fallback_id,
                        WORK_CONTEXT_ERROR,
                        "the work context request stopped",
                    )
                });
        if let Some(recording) = &recording {
            recording.record(RecordDirection::Gateway, &response);
        }
        let _ = outbox.send(Message::text(response)).await;
    });
}

fn respond(context: &dyn WorkContextPort, id: Value, method: &str, text: &str) -> String {
    if method != CONTEXT_THREAD_METHOD {
        return error_response(
            id,
            JSON_RPC_METHOD_NOT_FOUND,
            &format!("unknown gateway method `{method}`"),
        );
    }
    let Some(params) = serde_json::from_str::<RequestEnvelope>(text)
        .ok()
        .and_then(|envelope| serde_json::from_value::<ThreadParams>(envelope.params).ok())
    else {
        return error_response(id, JSON_RPC_INVALID_PARAMS, "`threadId` is required");
    };
    match context
        .thread_resources(&params.thread_id)
        .and_then(|resources| Ok(serde_json::to_value(resources)?))
    {
        Ok(resources) => {
            json!({ "jsonrpc": "2.0", "id": id, "result": { "resources": resources } }).to_string()
        }
        Err(error) => error_response(id, WORK_CONTEXT_ERROR, &format!("{error:#}")),
    }
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::inbound::work_context_port::ThreadResource;
    use anyhow::Result;

    struct FakeContext;

    impl WorkContextPort for FakeContext {
        fn thread_resources(&self, thread_id: &str) -> Result<Vec<ThreadResource>> {
            if thread_id == "broken" {
                anyhow::bail!("upstream is not ready");
            }
            Ok(vec![ThreadResource {
                kind: "jira".into(),
                key: "acme.atlassian.net/ABC-1".into(),
                url: "https://acme.atlassian.net/browse/ABC-1".into(),
                title: "ABC-1".into(),
                source: "user".into(),
                first_seen_at: "2026-11-11T00:00:00Z".into(),
            }])
        }
    }

    fn answer(method: &str, params: Value) -> Value {
        let text =
            json!({ "jsonrpc": "2.0", "id": 3, "method": method, "params": params }).to_string();
        serde_json::from_str(&respond(&FakeContext, json!(3), method, &text)).unwrap()
    }

    #[test]
    fn thread_resources_are_returned_in_camel_case() {
        let response = answer(CONTEXT_THREAD_METHOD, json!({ "threadId": "t1" }));
        assert_eq!(response["id"], 3);
        let resource = &response["result"]["resources"][0];
        assert_eq!(resource["kind"], "jira");
        assert_eq!(resource["firstSeenAt"], "2026-11-11T00:00:00Z");
    }

    #[test]
    fn bad_requests_and_failures_are_errors() {
        assert_eq!(
            answer(CONTEXT_THREAD_METHOD, json!({}))["error"]["code"],
            JSON_RPC_INVALID_PARAMS
        );
        assert_eq!(
            answer("akra/context/other", json!({ "threadId": "t1" }))["error"]["code"],
            JSON_RPC_METHOD_NOT_FOUND
        );
        let failed = answer(CONTEXT_THREAD_METHOD, json!({ "threadId": "broken" }));
        assert_eq!(failed["error"]["code"], WORK_CONTEXT_ERROR);
        assert!(
            failed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not ready")
        );
    }
}
