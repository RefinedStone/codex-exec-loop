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
    IssueCreationConfirmRequest, IssueCreationIntentRequest, IssueCreationPreviewRequest,
    IssueCreationPublishRequest, IssueCreationRestartRequest, IssueCreationTarget,
};
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
const ISSUE_CREATE_OPEN_METHOD: &str = "akra/issue/create/open";
const ISSUE_CREATE_PREVIEW_METHOD: &str = "akra/issue/create/preview";
const ISSUE_CREATE_PUBLISH_METHOD: &str = "akra/issue/create/publish";
const ISSUE_CREATE_RECHECK_METHOD: &str = "akra/issue/create/recheck";
const ISSUE_CREATE_CONFIRM_METHOD: &str = "akra/issue/create/confirm";
const ISSUE_CREATE_RESTART_METHOD: &str = "akra/issue/create/restart";
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
        ISSUE_CREATE_OPEN_METHOD,
        ISSUE_CREATE_PREVIEW_METHOD,
        ISSUE_CREATE_PUBLISH_METHOD,
        ISSUE_CREATE_RECHECK_METHOD,
        ISSUE_CREATE_CONFIRM_METHOD,
        ISSUE_CREATE_RESTART_METHOD,
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
        ISSUE_CREATE_OPEN_METHOD => params::<IssueCreationTarget>(text).map(|target| {
            drafts
                .creation_open(target)
                .and_then(|opened| to_value(&opened))
        }),
        ISSUE_CREATE_PREVIEW_METHOD => params::<IssueCreationPreviewRequest>(text).map(|request| {
            drafts
                .creation_preview(request)
                .and_then(|preview| to_value(&preview))
        }),
        ISSUE_CREATE_PUBLISH_METHOD => params::<IssueCreationPublishRequest>(text).map(|request| {
            drafts
                .creation_publish(request)
                .and_then(|published| to_value(&published))
        }),
        ISSUE_CREATE_RECHECK_METHOD => params::<IssueCreationIntentRequest>(text).map(|request| {
            drafts
                .creation_recheck(request)
                .and_then(|intent| to_value(&intent))
        }),
        ISSUE_CREATE_CONFIRM_METHOD => params::<IssueCreationConfirmRequest>(text).map(|request| {
            drafts
                .creation_confirm(request)
                .and_then(|intent| to_value(&intent))
        }),
        ISSUE_CREATE_RESTART_METHOD => params::<IssueCreationRestartRequest>(text).map(|request| {
            drafts
                .creation_restart(request)
                .and_then(|opened| to_value(&opened))
        }),

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
        fn creation_open(
            &self,
            target: IssueCreationTarget,
        ) -> Result<crate::application::port::inbound::issue_draft_port::IssueCreationOpened>
        {
            Ok(creation_opened(&target.repository))
        }

        fn creation_preview(
            &self,
            request: IssueCreationPreviewRequest,
        ) -> Result<crate::application::port::inbound::issue_draft_port::IssueCreationPreview>
        {
            Ok(
                crate::application::port::inbound::issue_draft_port::IssueCreationPreview {
                    preview: IssueDraftPreview::NeedsConfirmation {
                        span_indexes: (0..request.text.spans.len()).collect(),
                        readiness: None,
                    },
                    approval: None,
                },
            )
        }

        fn creation_publish(
            &self,
            request: IssueCreationPublishRequest,
        ) -> Result<crate::application::port::inbound::issue_draft_port::IssueCreationPublished>
        {
            assert_eq!(request.approval.intent_id, request.intent_id);
            assert_eq!(request.approval.repository.repository_id, "R_widgets");
            assert_eq!(
                request.approval.repository.project_scope.workspace_dir,
                "/work/project"
            );
            Ok(
                crate::application::port::inbound::issue_draft_port::IssueCreationPublished::Held {
                    intent: creation_intent(
                        &request.intent_id,
                        if request.shared_copy_only {
                            crate::domain::issue_creation::IssueCreationIntentState::Recorded
                        } else {
                            crate::domain::issue_creation::IssueCreationIntentState::Held
                        },
                    ),
                },
            )
        }

        fn creation_recheck(
            &self,
            request: IssueCreationIntentRequest,
        ) -> Result<crate::application::port::inbound::issue_draft_port::IssueCreationIntent>
        {
            Ok(creation_intent(
                &request.intent_id,
                crate::domain::issue_creation::IssueCreationIntentState::NeedsConfirmation,
            ))
        }

        fn creation_confirm(
            &self,
            request: IssueCreationConfirmRequest,
        ) -> Result<crate::application::port::inbound::issue_draft_port::IssueCreationIntent>
        {
            assert_eq!(request.url, "https://github.com/sample/widgets/issues/12");
            Ok(creation_intent(
                &request.intent_id,
                crate::domain::issue_creation::IssueCreationIntentState::Confirmed,
            ))
        }

        fn creation_restart(
            &self,
            request: IssueCreationRestartRequest,
        ) -> Result<crate::application::port::inbound::issue_draft_port::IssueCreationOpened>
        {
            assert!(request.acknowledge_uncertain);
            let mut opened = creation_opened("next");
            opened.intent.predecessor_intent_id = Some(request.previous_intent_id);
            Ok(opened)
        }

        fn open(&self, _: IssueDraftTarget) -> Result<IssueDraftOpened> {
            anyhow::bail!("gh is not signed in")
        }

        fn preview(&self, request: IssueDraftPreviewRequest) -> Result<IssueDraftPreview> {
            Ok(IssueDraftPreview::NeedsConfirmation {
                span_indexes: (0..request.text.spans.len()).collect(),
                readiness: None,
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
    fn creation_intent(
        id: &str,
        state: crate::domain::issue_creation::IssueCreationIntentState,
    ) -> crate::application::port::inbound::issue_draft_port::IssueCreationIntent {
        crate::application::port::inbound::issue_draft_port::IssueCreationIntent {
            intent_id: id.to_string(),
            state,
            created: None,
            shared_copy: None,
            restore_link_required: true,
            restore_link_finalized: false,
            predecessor_intent_id: None,
        }
    }

    fn creation_opened(
        id: &str,
    ) -> crate::application::port::inbound::issue_draft_port::IssueCreationOpened {
        use crate::domain::issue_policy::IssueSensitiveSpanAction;
        crate::application::port::inbound::issue_draft_port::IssueCreationOpened {
            repository: crate::domain::issue_creation::IssueRepositoryReference {
                provider: crate::domain::issue_work::IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_widgets".to_string(),
                project_scope: crate::domain::issue_work::IssueProjectScope {
                    workspace_dir: "/work/project".to_string(),
                },
            },
            intent: creation_intent(
                id,
                crate::domain::issue_creation::IssueCreationIntentState::Prepared,
            ),
            title: String::new(),
            body: String::new(),
            template: crate::application::port::inbound::issue_draft_port::IssueDraftTemplate {
                status: "not_configured",
                path: None,
                reason: None,
                required_inputs: vec![],
                unsupported_inputs: vec![],
            },
            required_sections: vec![],
            span_actions:
                crate::application::port::inbound::issue_draft_port::IssueDraftSpanActions {
                    secret: IssueSensitiveSpanAction::Remove,
                    name: IssueSensitiveSpanAction::RestorableMask,
                    path: IssueSensitiveSpanAction::RestorableMask,
                    log: IssueSensitiveSpanAction::RestorableMask,
                },
        }
    }

    #[test]
    fn creation_routes_keep_repository_intent_approval_and_explicit_restart_consent() {
        let target = json!({ "cwd": "/work/project", "owner": "sample", "repository": "widgets" });
        let opened = answer(ISSUE_CREATE_OPEN_METHOD, target.clone());
        assert_eq!(opened["result"]["intent"]["intentId"], "widgets");
        assert_eq!(opened["result"]["intent"]["state"], "prepared");
        assert_eq!(opened["result"]["repository"]["repositoryId"], "R_widgets");
        let text = json!({ "title": "T", "body": "B", "spans": [{ "field": "body", "start": 0, "end": 1 }] });
        let preview = answer(
            ISSUE_CREATE_PREVIEW_METHOD,
            json!({ "target": target, "intentId": "create-1", "text": text }),
        );
        assert_eq!(preview["result"]["preview"]["status"], "needs_confirmation");
        assert_eq!(preview["result"]["approval"], Value::Null);
        for shared_only in [false, true] {
            let published = answer(
                ISSUE_CREATE_PUBLISH_METHOD,
                json!({
                    "target": target, "intentId": "create-1", "text": text, "sharedCopyOnly": shared_only,
                    "approval": {
                        "repository": opened["result"]["repository"], "intentId": "create-1",
                        "sharedCopyHash": { "version": 1, "sha256": "00" }, "policyVersion": "p1", "templateVersion": null,
                    },
                }),
            );
            assert_eq!(
                published["result"]["intent"]["state"],
                if shared_only { "recorded" } else { "held" }
            );
            assert_eq!(published["result"]["intent"]["restoreLinkRequired"], true);
        }
        let rechecked = answer(
            ISSUE_CREATE_RECHECK_METHOD,
            json!({ "target": target, "intentId": "create-1" }),
        );
        assert_eq!(rechecked["result"]["state"], "needs_confirmation");
        let confirmed = answer(
            ISSUE_CREATE_CONFIRM_METHOD,
            json!({
                "target": target, "intentId": "create-1", "url": "https://github.com/sample/widgets/issues/12",
            }),
        );
        assert_eq!(confirmed["result"]["state"], "confirmed");
        let restarted = answer(
            ISSUE_CREATE_RESTART_METHOD,
            json!({
                "target": target, "previousIntentId": "create-1", "acknowledgeUncertain": true,
            }),
        );
        assert_eq!(
            restarted["result"]["intent"]["predecessorIntentId"],
            "create-1"
        );
    }

    #[test]
    fn every_creation_route_uses_the_sensitive_draft_classification_and_sanitizes_bad_params() {
        for method in [
            ISSUE_CREATE_OPEN_METHOD,
            ISSUE_CREATE_PREVIEW_METHOD,
            ISSUE_CREATE_PUBLISH_METHOD,
            ISSUE_CREATE_RECHECK_METHOD,
            ISSUE_CREATE_CONFIRM_METHOD,
            ISSUE_CREATE_RESTART_METHOD,
        ] {
            assert!(is_issue_draft_method(method));
            let invalid = answer(
                method,
                json!({ "unexpected": "synthetic-raw-create-secret" }),
            );
            assert_eq!(invalid["error"]["code"], JSON_RPC_INVALID_PARAMS);
            assert!(!invalid.to_string().contains("synthetic-raw-create-secret"));
        }
    }
}
