//! Gateway 의 이슈 초안 메서드 처리이다 (#2426): `akra/issue/open`, `preview`, `publish`, `recheck`, `abandon`.
//! 인계의 `akra/issue/handoffCopy` 는 `issue` 모듈이 맡는다 (#2428). Desktop 의 이슈 초안 화면이 쓴다. 검증, 마스킹, 승인 검사, 게시 의도
//! 기록은 `IssueDraftPort` 뒤의 서비스가 한다. 여기서는 요청을 읽고 답을 쓴다. 원격 조회는 블로킹이므로 요청마다
//! 블로킹 작업으로 돌린다.
//!
//! 이 경로는 민감 입력 경로이다. 요청에는 가리기 전의 로컬 글이 있고 `open` 의 답에는 원격 글이 있다. 그래서 요청과
//! 답을 세션 녹화에 남기지 않고, 오류 글에 요청의 값을 넣지 않는다.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::application::port::inbound::issue_draft_port::{
    IssueDraftPort, IssueDraftPreviewRequest, IssueDraftPublishRequest, IssueDraftTarget,
};

/// params `IssueDraftTarget` -> result `IssueDraftOpened`. 쓰지 않는다.
const ISSUE_OPEN_METHOD: &str = "akra/issue/open";
/// params `{ target, text }` -> result `IssueDraftPreview`. 쓰지 않는다.
const ISSUE_PREVIEW_METHOD: &str = "akra/issue/preview";
/// params `{ target, text, approval, editBasis, sharedCopyOnly? }` -> result `IssueDraftPublished`.
const ISSUE_PUBLISH_METHOD: &str = "akra/issue/publish";
/// params `IssueDraftTarget` -> result `{ intent: IssueDraftIntent | null }`. 읽기만 한다.
const ISSUE_RECHECK_METHOD: &str = "akra/issue/recheck";
/// params `{ cwd, intentId }` -> result `{ intent: IssueDraftIntent }`.
const ISSUE_ABANDON_METHOD: &str = "akra/issue/abandon";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const JSON_RPC_INVALID_PARAMS: i64 = -32602;
const ISSUE_ERROR: i64 = -32000;

#[derive(Deserialize)]
struct RequestEnvelope {
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbandonParams {
    cwd: String,
    intent_id: String,
}

/// 이 모듈이 답하는 메서드인지 본다. 이 메서드들의 요청과 답은 녹화하지 않는다.
pub(super) fn is_issue_draft_method(method: &str) -> bool {
    [
        ISSUE_OPEN_METHOD,
        ISSUE_PREVIEW_METHOD,
        ISSUE_PUBLISH_METHOD,
        ISSUE_RECHECK_METHOD,
        ISSUE_ABANDON_METHOD,
    ]
    .contains(&method)
}

fn params<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let envelope = serde_json::from_str::<RequestEnvelope>(text).ok()?;
    serde_json::from_value(envelope.params).ok()
}

/// 요청 하나를 블로킹 작업으로 처리하고 답을 접속의 보내기 함으로 보낸다. 녹화하지 않는다.
pub(super) fn spawn_request(
    drafts: Arc<dyn IssueDraftPort>,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
) {
    tokio::spawn(async move {
        let fallback_id = id.clone();
        let response =
            tokio::task::spawn_blocking(move || respond(drafts.as_ref(), id, &method, &text))
                .await
                .unwrap_or_else(|_| {
                    error_response(fallback_id, ISSUE_ERROR, "the issue request stopped")
                });
        let _ = outbox.send(Message::text(response)).await;
    });
}

fn respond(drafts: &dyn IssueDraftPort, id: Value, method: &str, text: &str) -> String {
    let result = match method {
        ISSUE_OPEN_METHOD => params::<IssueDraftTarget>(text)
            .map(|target| drafts.open(target).and_then(|opened| to_value(&opened))),
        ISSUE_PREVIEW_METHOD => params::<IssueDraftPreviewRequest>(text).map(|request| {
            drafts
                .preview(request)
                .and_then(|preview| to_value(&preview))
        }),
        ISSUE_PUBLISH_METHOD => params::<IssueDraftPublishRequest>(text).map(|request| {
            drafts
                .publish(request)
                .and_then(|published| to_value(&published))
        }),
        ISSUE_RECHECK_METHOD => params::<IssueDraftTarget>(text).map(|target| {
            drafts
                .recheck(target)
                .and_then(|intent| Ok(json!({ "intent": to_value(&intent)? })))
        }),
        ISSUE_ABANDON_METHOD => params::<AbandonParams>(text).map(|params| {
            drafts
                .abandon(&params.cwd, &params.intent_id)
                .and_then(|intent| Ok(json!({ "intent": to_value(&intent)? })))
        }),
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
        Some(Err(error)) => error_response(id, ISSUE_ERROR, &format!("{error:#}")),
        None => error_response(
            id,
            JSON_RPC_INVALID_PARAMS,
            &format!("the params of `{method}` are not valid"),
        ),
    }
}

fn to_value(value: &impl serde::Serialize) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(value)?)
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::application::port::inbound::issue_draft_port::{
        IssueDraftIntent, IssueDraftOpened, IssueDraftPreview, IssueDraftPublished,
    };
    use crate::domain::issue_publish_intent::IssuePublishIntentState;

    /// 받은 값을 답에 비추는 가짜이다.
    struct EchoDrafts;

    fn intent(intent_id: &str, state: IssuePublishIntentState) -> IssueDraftIntent {
        IssueDraftIntent {
            intent_id: intent_id.to_string(),
            state,
        }
    }

    impl IssueDraftPort for EchoDrafts {
        fn open(&self, _: IssueDraftTarget) -> Result<IssueDraftOpened> {
            anyhow::bail!("gh is not signed in")
        }

        fn preview(&self, request: IssueDraftPreviewRequest) -> Result<IssueDraftPreview> {
            Ok(IssueDraftPreview::NeedsConfirmation {
                span_indexes: (0..request.text.spans.len()).collect(),
            })
        }

        fn publish(&self, request: IssueDraftPublishRequest) -> Result<IssueDraftPublished> {
            Ok(if request.shared_copy_only {
                IssueDraftPublished::Conflict
            } else {
                IssueDraftPublished::Refused {
                    reason: "policy_version",
                }
            })
        }

        fn recheck(&self, _: IssueDraftTarget) -> Result<Option<IssueDraftIntent>> {
            Ok(Some(intent(
                "a1",
                IssuePublishIntentState::NeedsConfirmation,
            )))
        }

        fn abandon(&self, _: &str, intent_id: &str) -> Result<IssueDraftIntent> {
            Ok(intent(intent_id, IssuePublishIntentState::Abandoned))
        }
    }

    fn target() -> Value {
        json!({ "cwd": "/work/project", "owner": "sample", "repository": "widgets", "number": 7 })
    }

    fn answer(method: &str, params: Value) -> Value {
        let text =
            json!({ "jsonrpc": "2.0", "id": 5, "method": method, "params": params }).to_string();
        serde_json::from_str(&respond(&EchoDrafts, json!(5), method, &text)).unwrap()
    }

    #[test]
    fn preview_and_publish_read_camel_case_params_and_answer_with_a_status() {
        let text = json!({
            "title": "T",
            "body": "B",
            "spans": [{ "field": "body", "start": 0, "end": 1, "class": "secret", "decision": null }],
        });
        let preview = answer(
            ISSUE_PREVIEW_METHOD,
            json!({ "target": target(), "text": text }),
        );
        assert_eq!(preview["result"]["status"], "needs_confirmation");
        assert_eq!(preview["result"]["spanIndexes"], json!([0]));

        let publish = |shared_copy_only: bool| {
            answer(
                ISSUE_PUBLISH_METHOD,
                json!({
                    "target": target(),
                    "text": text,
                    "approval": {
                        "sharedCopyHash": { "version": 1, "sha256": "00" },
                        "policyVersion": "p1",
                        "templateVersion": null,
                    },
                    "editBasis": { "version": 1, "sha256": "11" },
                    "sharedCopyOnly": shared_copy_only,
                }),
            )
        };
        assert_eq!(publish(true)["result"]["status"], "conflict");
        assert_eq!(publish(false)["result"]["status"], "refused");
        assert_eq!(publish(false)["result"]["reason"], "policy_version");
    }

    #[test]
    fn recheck_and_abandon_answer_with_the_intent_state() {
        let rechecked = answer(ISSUE_RECHECK_METHOD, target());
        assert_eq!(rechecked["result"]["intent"]["intentId"], "a1");
        assert_eq!(rechecked["result"]["intent"]["state"], "needs_confirmation");

        let abandoned = answer(
            ISSUE_ABANDON_METHOD,
            json!({ "cwd": "/work/project", "intentId": "a1" }),
        );
        assert_eq!(abandoned["result"]["intent"]["state"], "abandoned");
    }

    #[test]
    fn bad_params_unknown_methods_and_failures_are_errors_without_request_values() {
        let bad = answer(
            ISSUE_PREVIEW_METHOD,
            json!({ "target": target(), "text": { "title": "synthetic-secret-0000" } }),
        );
        assert_eq!(bad["error"]["code"], JSON_RPC_INVALID_PARAMS);
        assert!(!bad.to_string().contains("synthetic-secret-0000"));
        assert_eq!(
            answer("akra/issue/other", target())["error"]["code"],
            JSON_RPC_METHOD_NOT_FOUND
        );
        let failed = answer(ISSUE_OPEN_METHOD, target());
        assert_eq!(failed["error"]["code"], ISSUE_ERROR);
        assert!(
            failed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not signed in")
        );
    }
}
