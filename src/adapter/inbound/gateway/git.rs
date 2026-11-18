//! Gateway 의 `akra/git/*` 처리이다 (plan 27 G1, G2). 저장소의 커밋 그래프, ref 들의 지문, 커밋 하나의 상세를 읽어
//! 준다. git 실행은 블로킹이므로 요청마다 블로킹 작업으로 돌린다. 접속의 읽기 고리는 기다리지 않는다. 저장소를
//! 고치는 메서드는 없다.

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
/// params `{ cwd }` -> result `GitGraphRefsState`. 화면이 저장소가 바뀌었는지 보려고 주기적으로 부른다.
const GIT_REFS_METHOD: &str = "akra/git/refs";
/// params `{ cwd, commit }` -> result `{ commit: GitCommitDetail | null }`. 그 커밋이 없으면 `null` 이다.
const GIT_COMMIT_METHOD: &str = "akra/git/commit";
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

#[derive(Deserialize)]
struct RefsParams {
    cwd: String,
}

#[derive(Deserialize)]
struct CommitParams {
    cwd: String,
    commit: String,
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
    let result = match method {
        GIT_GRAPH_METHOD => graph_result(graph, text),
        GIT_REFS_METHOD => refs_result(graph, text),
        GIT_COMMIT_METHOD => commit_result(graph, text),
        _ => {
            return error_response(
                id,
                JSON_RPC_METHOD_NOT_FOUND,
                &format!("unknown gateway method `{method}`"),
            );
        }
    };
    match result {
        Some(Ok(result)) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        Some(Err(error)) => error_response(id, GIT_ERROR, &format!("{error:#}")),
        None => error_response(
            id,
            JSON_RPC_INVALID_PARAMS,
            &format!("the params of `{method}` are not valid"),
        ),
    }
}

/// 답이다. params 를 읽지 못하면 `None` 이다.
type MethodResult = Option<anyhow::Result<Value>>;

fn graph_result(graph: &dyn GitGraphPort, text: &str) -> MethodResult {
    let params = params::<GraphParams>(text)?;
    let page = graph.graph_page(GitGraphPageRequest {
        repository_dir: params.cwd,
        limit: params.limit,
        cursor: params.cursor,
    });
    Some(page.and_then(|page| Ok(serde_json::to_value(page)?)))
}

fn refs_result(graph: &dyn GitGraphPort, text: &str) -> MethodResult {
    let params = params::<RefsParams>(text)?;
    let state = graph.refs_state(&params.cwd);
    Some(state.and_then(|state| Ok(serde_json::to_value(state)?)))
}

fn commit_result(graph: &dyn GitGraphPort, text: &str) -> MethodResult {
    let params = params::<CommitParams>(text)?;
    let detail = graph.commit_detail(&params.cwd, &params.commit);
    Some(detail.and_then(|detail| Ok(json!({ "commit": serde_json::to_value(detail)? }))))
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::application::port::inbound::git_graph_port::{GitGraphPage, GitGraphRefsState};
    use crate::domain::git_graph::GitCommitDetail;

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
                refs_digest: None,
                refs: Vec::new(),
                worktrees: Vec::new(),
                rows: Vec::new(),
                next: request.cursor,
            })
        }

        fn refs_state(&self, repository_dir: &str) -> Result<GitGraphRefsState> {
            Ok(GitGraphRefsState {
                repository: true,
                refs_digest: Some(repository_dir.to_string()),
            })
        }

        fn commit_detail(
            &self,
            repository_dir: &str,
            commit: &str,
        ) -> Result<Option<GitCommitDetail>> {
            if commit == "missing" {
                return Ok(None);
            }
            Ok(Some(GitCommitDetail {
                id: commit.to_string(),
                parents: Vec::new(),
                author_name: repository_dir.to_string(),
                author_email: String::new(),
                author_time: 1,
                committer_name: String::new(),
                committer_time: 2,
                message: "subject".to_string(),
                diff: String::new(),
                diff_too_large: true,
            }))
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
    fn refs_and_commit_requests_reach_the_port() {
        let refs = answer(GIT_REFS_METHOD, json!({ "cwd": "/repo" }));
        assert_eq!(
            refs["result"],
            json!({ "repository": true, "refsDigest": "/repo" })
        );

        let found = answer(
            GIT_COMMIT_METHOD,
            json!({ "cwd": "/repo", "commit": "abc1234" }),
        );
        let commit = &found["result"]["commit"];
        assert_eq!(commit["id"], json!("abc1234"));
        assert_eq!(commit["authorName"], json!("/repo"));
        assert_eq!(commit["committerTime"], json!(2));
        assert_eq!(commit["diffTooLarge"], json!(true));

        let missing = answer(
            GIT_COMMIT_METHOD,
            json!({ "cwd": "/repo", "commit": "missing" }),
        );
        assert_eq!(missing["result"]["commit"], Value::Null);
        assert_eq!(
            answer(GIT_COMMIT_METHOD, json!({ "cwd": "/repo" }))["error"]["code"],
            json!(JSON_RPC_INVALID_PARAMS)
        );
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
