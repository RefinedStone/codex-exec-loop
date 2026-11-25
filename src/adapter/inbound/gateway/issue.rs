//! Gateway 의 `akra/issue/*` 처리이다 (#2428). 이슈 인계에 쓸 공유본을 준다. 원격의 원시 값은 답에 없다.
//! 조회는 블로킹이므로 요청마다 블로킹 작업으로 돌린다.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::recorder::{RecordDirection, SessionRecording};
use crate::application::port::inbound::issue_handoff_port::{
    IssueHandoffPort, IssueHandoffRequest,
};
use crate::domain::issue_work::IssueLocator;

pub(super) const ISSUE_METHOD_PREFIX: &str = "akra/issue/";
/// params `{ owner, repository, number, cwd? }` -> result `{ sharedCopy: { title, body } }`.
const ISSUE_HANDOFF_COPY_METHOD: &str = "akra/issue/handoffCopy";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const JSON_RPC_INVALID_PARAMS: i64 = -32602;
const ISSUE_ERROR: i64 = -32000;

#[derive(Deserialize)]
struct RequestEnvelope {
    params: HandoffCopyParams,
}

#[derive(Deserialize)]
struct HandoffCopyParams {
    #[serde(flatten)]
    locator: IssueLocator,
    #[serde(default)]
    cwd: Option<String>,
}

/// 요청 하나를 블로킹 작업으로 처리하고 답을 접속의 보내기 함으로 보낸다.
pub(super) fn spawn_request(
    handoff: Arc<dyn IssueHandoffPort>,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
    recording: Option<Arc<SessionRecording>>,
) {
    tokio::spawn(async move {
        let fallback_id = id.clone();
        let response =
            tokio::task::spawn_blocking(move || respond(handoff.as_ref(), id, &method, &text))
                .await
                .unwrap_or_else(|_| {
                    error_response(fallback_id, ISSUE_ERROR, "the issue request stopped")
                });
        if let Some(recording) = &recording {
            recording.record(RecordDirection::Gateway, &response);
        }
        let _ = outbox.send(Message::text(response)).await;
    });
}

fn respond(handoff: &dyn IssueHandoffPort, id: Value, method: &str, text: &str) -> String {
    if method != ISSUE_HANDOFF_COPY_METHOD {
        return error_response(
            id,
            JSON_RPC_METHOD_NOT_FOUND,
            &format!("unknown gateway method `{method}`"),
        );
    }
    let Ok(RequestEnvelope { params }) = serde_json::from_str(text) else {
        return error_response(
            id,
            JSON_RPC_INVALID_PARAMS,
            &format!("the params of `{method}` are not valid"),
        );
    };
    // The service reads the scoped publication journal. The client cannot
    // supply a hash and call it an approval.
    match handoff.handoff_copy(IssueHandoffRequest {
        locator: params.locator,
        workspace_dir: params.cwd,
        approved: None,
    }) {
        Ok(shared_copy) => {
            json!({ "jsonrpc": "2.0", "id": id, "result": { "sharedCopy": shared_copy } })
                .to_string()
        }
        Err(error) => error_response(id, ISSUE_ERROR, &format!("{error:#}")),
    }
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}
