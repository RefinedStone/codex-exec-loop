//! Gateway 의 `akra/context/*` 처리이다 (plan 22). 스레드에 나온 외부 문서를 장부에서 읽어 준다. 수집은
//! 블로킹(app-server 조회와 SQLite)이므로 요청마다 블로킹 작업으로 돌린다. 접속의 읽기 고리는 기다리지 않는다.
//! `refresh` 는 외부 서비스에 문서의 버전을 묻는다. 느릴 수 있으므로 목록 조회(`thread`)와 따로 둔다.
//! `attach` 와 `list` 는 스레드가 아닌 범위(저장소, 작업, 저장소 안 경로)를 쓴다 (A03).

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::recorder::{RecordDirection, SessionRecording};
use crate::application::port::inbound::work_context_port::{
    AttachContextRequest, ContextScope, ContextScopeRequest, WorkContextPort,
};

pub(super) const CONTEXT_METHOD_PREFIX: &str = "akra/context/";
/// params `{ threadId }` -> result `{ resources: [ContextResource] }`
const CONTEXT_THREAD_METHOD: &str = "akra/context/thread";
/// params `{ threadId }` -> result `{ resources: [ContextResource] }`. 문서가 바뀌었는지 외부 서비스에 확인한다.
const CONTEXT_REFRESH_METHOD: &str = "akra/context/refresh";
/// params `{ threadId, kind, key }` -> result `{ resources: [ContextResource] }`. 그 문서를 읽었다고 적는다.
const CONTEXT_MARK_READ_METHOD: &str = "akra/context/markRead";
/// params `{ cwd, scope, url, source? }` -> result `{ resources: [ContextResource] }`. 문서를 범위에 잇는다.
/// `scope` 는 `{ kind: "repository" }`, `{ kind: "task", taskId }`, `{ kind: "path", path }` 이다.
const CONTEXT_ATTACH_METHOD: &str = "akra/context/attach";
/// params `{ cwd, scope }` -> result `{ resources: [ContextResource] }`. 장부의 값만 읽는다.
const CONTEXT_LIST_METHOD: &str = "akra/context/list";
/// `attach` 에 `source` 가 없으면 사람이 이은 것으로 본다.
const DEFAULT_ATTACH_SOURCE: &str = "user";
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkReadParams {
    thread_id: String,
    kind: String,
    key: String,
}

#[derive(Deserialize)]
struct ScopeParams {
    cwd: String,
    scope: ContextScope,
}

#[derive(Deserialize)]
struct AttachParams {
    cwd: String,
    scope: ContextScope,
    url: String,
    source: Option<String>,
}

fn params<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let envelope = serde_json::from_str::<RequestEnvelope>(text).ok()?;
    serde_json::from_value(envelope.params).ok()
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
    let resources = match method {
        CONTEXT_THREAD_METHOD | CONTEXT_REFRESH_METHOD => {
            let Some(params) = params::<ThreadParams>(text) else {
                return error_response(id, JSON_RPC_INVALID_PARAMS, "`threadId` is required");
            };
            if method == CONTEXT_REFRESH_METHOD {
                context.refresh_thread_resources(&params.thread_id)
            } else {
                context.thread_resources(&params.thread_id)
            }
        }
        CONTEXT_MARK_READ_METHOD => {
            let Some(params) = params::<MarkReadParams>(text) else {
                return error_response(
                    id,
                    JSON_RPC_INVALID_PARAMS,
                    "`threadId`, `kind`, and `key` are required",
                );
            };
            context.mark_thread_resource_read(&params.thread_id, &params.kind, &params.key)
        }
        CONTEXT_ATTACH_METHOD => {
            let Some(params) = params::<AttachParams>(text) else {
                return error_response(
                    id,
                    JSON_RPC_INVALID_PARAMS,
                    "`cwd`, `scope`, and `url` are required",
                );
            };
            context.attach_scope_resource(&AttachContextRequest {
                scope: ContextScopeRequest {
                    cwd: params.cwd,
                    scope: params.scope,
                },
                url: params.url,
                source: params
                    .source
                    .unwrap_or_else(|| DEFAULT_ATTACH_SOURCE.to_string()),
            })
        }
        CONTEXT_LIST_METHOD => {
            let Some(params) = params::<ScopeParams>(text) else {
                return error_response(
                    id,
                    JSON_RPC_INVALID_PARAMS,
                    "`cwd` and `scope` are required",
                );
            };
            context.scope_resources(&ContextScopeRequest {
                cwd: params.cwd,
                scope: params.scope,
            })
        }
        _ => {
            return error_response(
                id,
                JSON_RPC_METHOD_NOT_FOUND,
                &format!("unknown gateway method `{method}`"),
            );
        }
    };
    match resources.and_then(|resources| Ok(serde_json::to_value(resources)?)) {
        Ok(resources) => {
            json!({ "jsonrpc": "2.0", "id": id, "result": { "resources": resources } }).to_string()
        }
        Err(error) => error_response(id, WORK_CONTEXT_ERROR, &format!("{error:#}")),
    }
}

/// `local` 이 아닌 접속의 `akra/context/*` 답이다 (plan 25 Q-C3). 답은 하지만 장부를 읽지도 쓰지도 않는다. 장부의
/// 키는 thread id 뿐이라서 원격 thread 가 들어가면 같은 id 의 로컬 thread 와 섞인다. 저장소 범위의 `cwd` 도 원격
/// 호스트의 폴더라서 로컬에서 찾지 않는다.
pub(super) fn remote_host_response(id: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "resources": [] } }).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::inbound::work_context_port::ContextResource;
    use anyhow::Result;

    struct FakeContext;

    fn resource(changed: bool) -> ContextResource {
        ContextResource {
            kind: "jira".into(),
            key: "acme.atlassian.net/ABC-1".into(),
            url: "https://acme.atlassian.net/browse/ABC-1".into(),
            title: "ABC-1".into(),
            source: "user".into(),
            first_seen_at: "2026-11-11T00:00:00Z".into(),
            changed,
            version_checked_at: changed.then(|| "2026-11-11T00:05:00Z".to_string()),
            confirmed_version: Some("v1".to_string()),
            current_version: Some(if changed { "v2" } else { "v1" }.to_string()),
        }
    }

    impl WorkContextPort for FakeContext {
        fn thread_resources(&self, thread_id: &str) -> Result<Vec<ContextResource>> {
            if thread_id == "broken" {
                anyhow::bail!("upstream is not ready");
            }
            Ok(vec![resource(false)])
        }

        fn refresh_thread_resources(&self, _thread_id: &str) -> Result<Vec<ContextResource>> {
            Ok(vec![resource(true)])
        }

        fn mark_thread_resource_read(
            &self,
            _thread_id: &str,
            resource_kind: &str,
            resource_key: &str,
        ) -> Result<Vec<ContextResource>> {
            if (resource_kind, resource_key) != ("jira", "acme.atlassian.net/ABC-1") {
                anyhow::bail!("the thread has no such document");
            }
            Ok(vec![resource(false)])
        }

        fn attach_scope_resource(
            &self,
            request: &AttachContextRequest,
        ) -> Result<Vec<ContextResource>> {
            let mut attached = resource(false);
            attached.source = request.source.clone();
            attached.url = request.url.clone();
            attached.title = format!("{}:{:?}", request.scope.cwd, request.scope.scope);
            Ok(vec![attached])
        }

        fn scope_resources(&self, request: &ContextScopeRequest) -> Result<Vec<ContextResource>> {
            let mut listed = resource(true);
            listed.title = format!("{}:{:?}", request.cwd, request.scope);
            Ok(vec![listed])
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
        assert_eq!(resource["changed"], false);
        assert_eq!(resource["versionCheckedAt"], Value::Null);
    }

    #[test]
    fn refresh_reports_changes_and_mark_read_clears_them() {
        // plan 22 stage 2
        let refreshed = answer(CONTEXT_REFRESH_METHOD, json!({ "threadId": "t1" }));
        let resource = &refreshed["result"]["resources"][0];
        assert_eq!(resource["changed"], true);
        assert_eq!(resource["versionCheckedAt"], "2026-11-11T00:05:00Z");

        let read = answer(
            CONTEXT_MARK_READ_METHOD,
            json!({ "threadId": "t1", "kind": "jira", "key": "acme.atlassian.net/ABC-1" }),
        );
        assert_eq!(read["result"]["resources"][0]["changed"], false);
        assert_eq!(
            answer(CONTEXT_MARK_READ_METHOD, json!({ "threadId": "t1" }))["error"]["code"],
            JSON_RPC_INVALID_PARAMS
        );
        let missing = answer(
            CONTEXT_MARK_READ_METHOD,
            json!({ "threadId": "t1", "kind": "jira", "key": "other" }),
        );
        assert_eq!(missing["error"]["code"], WORK_CONTEXT_ERROR);
    }

    #[test]
    fn attach_and_list_take_a_repository_scope() {
        // A03: one method family; the scope is a tagged object.
        let attached = answer(
            CONTEXT_ATTACH_METHOD,
            json!({
                "cwd": "/repo",
                "scope": { "kind": "task", "taskId": "T-1" },
                "url": "https://github.com/a/b/pull/2"
            }),
        );
        let resource = &attached["result"]["resources"][0];
        assert_eq!(resource["source"], "user");
        assert_eq!(resource["url"], "https://github.com/a/b/pull/2");
        assert_eq!(resource["title"], "/repo:Task { task_id: \"T-1\" }");
        assert_eq!(resource["confirmedVersion"], "v1");

        let listed = answer(
            CONTEXT_LIST_METHOD,
            json!({ "cwd": "/repo", "scope": { "kind": "path", "path": "src/a.rs" } }),
        );
        let resource = &listed["result"]["resources"][0];
        assert_eq!(resource["title"], "/repo:Path { path: \"src/a.rs\" }");
        assert_eq!(resource["changed"], true);
        assert_eq!(resource["currentVersion"], "v2");
        assert_eq!(
            answer(
                CONTEXT_LIST_METHOD,
                json!({ "cwd": "/repo", "scope": { "kind": "repository" } })
            )["result"]["resources"][0]["title"],
            "/repo:Repository"
        );

        for (method, params) in [
            (CONTEXT_LIST_METHOD, json!({ "cwd": "/repo" })),
            (
                CONTEXT_LIST_METHOD,
                json!({ "cwd": "/repo", "scope": { "kind": "branch" } }),
            ),
            (
                CONTEXT_LIST_METHOD,
                json!({ "cwd": "/repo", "scope": { "kind": "task" } }),
            ),
            (
                CONTEXT_ATTACH_METHOD,
                json!({ "cwd": "/repo", "scope": { "kind": "repository" } }),
            ),
        ] {
            assert_eq!(
                answer(method, params)["error"]["code"],
                JSON_RPC_INVALID_PARAMS
            );
        }
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
