//! Gateway 의 `akra/git/*` 처리이다 (plan 27 G1). 저장소의 커밋 그래프를 읽어 준다. git 실행은 블로킹이므로
//! 요청마다 블로킹 작업으로 돌린다. 접속의 읽기 고리는 기다리지 않는다. 저장소를 고치는 메서드는 없다.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::recorder::{RecordDirection, SessionRecording};
use crate::application::port::inbound::git_graph_port::{
    GitGraphCursor, GitGraphPageRequest, GitGraphPort,
};

pub(super) const GIT_METHOD_PREFIX: &str = "akra/git/";
/// params `{ cwd, limit?, cursor? }` -> result `GitGraphPage`. `cursor` 는 앞 쪽의 `next` 이다.
const GIT_GRAPH_METHOD: &str = "akra/git/graph";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const JSON_RPC_INVALID_PARAMS: i64 = -32602;
const GIT_ERROR: i64 = -32000;

#[derive(Deserialize)]
struct RequestEnvelope {
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphParams {
    cwd: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<GitGraphCursor>,
}

fn params<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let envelope = serde_json::from_str::<RequestEnvelope>(text).ok()?;
    serde_json::from_value(envelope.params).ok()
}

/// 요청 하나를 블로킹 작업으로 처리하고 답을 접속의 보내기 함으로 보낸다.
pub(super) fn spawn_request(
    graph: Arc<dyn GitGraphPort>,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
    recording: Option<Arc<SessionRecording>>,
) {
    tokio::spawn(async move {
        let fallback_id = id.clone();
        let response =
            tokio::task::spawn_blocking(move || respond(graph.as_ref(), id, &method, &text))
                .await
                .unwrap_or_else(|_| {
                    error_response(fallback_id, GIT_ERROR, "the git request stopped")
                });
        if let Some(recording) = &recording {
            recording.record(RecordDirection::Gateway, &response);
        }
        let _ = outbox.send(Message::text(response)).await;
    });
}

fn respond(graph: &dyn GitGraphPort, id: Value, method: &str, text: &str) -> String {
    if method != GIT_GRAPH_METHOD {
        return error_response(
            id,
            JSON_RPC_METHOD_NOT_FOUND,
            &format!("unknown gateway method `{method}`"),
        );
    }
    let Some(params) = params::<GraphParams>(text) else {
        return error_response(id, JSON_RPC_INVALID_PARAMS, "`cwd` is required");
    };
    let page = graph.graph_page(GitGraphPageRequest {
        repository_dir: params.cwd,
        limit: params.limit,
        cursor: params.cursor,
    });
    match page.and_then(|page| Ok(serde_json::to_value(page)?)) {
        Ok(page) => json!({ "jsonrpc": "2.0", "id": id, "result": page }).to_string(),
        Err(error) => error_response(id, GIT_ERROR, &format!("{error:#}")),
    }
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::application::port::inbound::git_graph_port::GitGraphPage;

    /// 받은 요청을 답에 되비춘다: `head` 는 폴더, `stale` 은 자리가 있었는지이다.
    struct EchoGraph;

    impl GitGraphPort for EchoGraph {
        fn graph_page(&self, request: GitGraphPageRequest) -> Result<GitGraphPage> {
            if request.repository_dir == "/broken" {
                anyhow::bail!("git did not run");
            }
            Ok(GitGraphPage {
                repository: true,
                stale: request.cursor.is_some(),
                head: Some(request.repository_dir),
                refs: Vec::new(),
                rows: Vec::new(),
                next: request.cursor,
            })
        }
    }

    fn answer(method: &str, params: Value) -> Value {
        let text = json!({ "id": 1, "method": method, "params": params }).to_string();
        serde_json::from_str(&respond(&EchoGraph, json!(1), method, &text)).unwrap()
    }

    #[test]
    fn graph_request_carries_the_folder_and_the_cursor_to_the_port() {
        let first = answer(GIT_GRAPH_METHOD, json!({ "cwd": "/repo" }));
        assert_eq!(first["result"]["head"], json!("/repo"));
        assert_eq!(first["result"]["stale"], json!(false));
        assert_eq!(first["result"]["next"], Value::Null);

        let cursor = json!({ "skip": 2, "lanes": ["abc", null], "refsDigest": "d" });
        let next = answer(
            GIT_GRAPH_METHOD,
            json!({ "cwd": "/repo", "limit": 5, "cursor": cursor }),
        );
        assert_eq!(next["result"]["stale"], json!(true));
        // 자리는 받은 모양 그대로 돌아간다. 클라이언트가 다음 요청에 다시 쓴다.
        assert_eq!(next["result"]["next"], cursor);
    }

    #[test]
    fn bad_params_unknown_methods_and_port_failures_are_json_rpc_errors() {
        assert_eq!(
            answer(GIT_GRAPH_METHOD, json!({}))["error"]["code"],
            json!(JSON_RPC_INVALID_PARAMS)
        );
        assert_eq!(
            answer("akra/git/reset", json!({ "cwd": "/repo" }))["error"]["code"],
            json!(JSON_RPC_METHOD_NOT_FOUND)
        );
        let failed = answer(GIT_GRAPH_METHOD, json!({ "cwd": "/broken" }));
        assert_eq!(failed["error"]["code"], json!(GIT_ERROR));
        assert_eq!(failed["error"]["message"], json!("git did not run"));
    }
}
