//! 클라이언트 메시지의 `method` 만 읽고 Gateway 가 직접 답할지(`akra/*`) upstream 으로 그대로 보낼지 정한다.
//! 메시지 본문은 바꾸지 않는다 (D-11). JSON 으로 읽을 수 없는 메시지도 그대로 전달한다.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::port::outbound::gateway_upstream_port::GatewayUpstreamSnapshot;

const AKRA_METHOD_PREFIX: &str = "akra/";
const GATEWAY_INFO_METHOD: &str = "akra/gateway/info";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, PartialEq)]
pub(super) enum ClientTextRoute {
    /// upstream 으로 바이트 그대로 보낸다.
    Forward,
    /// Gateway 가 직접 응답한다.
    AkraRequest { id: Value, method: String },
    /// 응답할 id 가 없는 `akra/*` 알림이다. upstream 은 모르는 메서드이므로 보내지 않는다.
    AkraNotification { method: String },
}

#[derive(Deserialize)]
struct MethodEnvelope {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    method: Option<String>,
}

pub(super) fn route_client_text(text: &str) -> ClientTextRoute {
    // 대부분의 메시지는 `akra/` 를 포함하지 않으므로 JSON 을 해석하지 않고 바로 전달한다.
    if !text.contains(AKRA_METHOD_PREFIX) {
        return ClientTextRoute::Forward;
    }
    let Ok(envelope) = serde_json::from_str::<MethodEnvelope>(text) else {
        return ClientTextRoute::Forward;
    };
    match (envelope.method, envelope.id) {
        (Some(method), Some(id)) if method.starts_with(AKRA_METHOD_PREFIX) => {
            ClientTextRoute::AkraRequest { id, method }
        }
        (Some(method), None) if method.starts_with(AKRA_METHOD_PREFIX) => {
            ClientTextRoute::AkraNotification { method }
        }
        _ => ClientTextRoute::Forward,
    }
}

/// `akra/*` 요청에 대한 JSON-RPC 응답 문자열이다.
pub(super) fn respond_to_akra_request(
    id: Value,
    method: &str,
    snapshot: &GatewayUpstreamSnapshot,
) -> String {
    let response = if method == GATEWAY_INFO_METHOD {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "gatewayVersion": env!("CARGO_PKG_VERSION"),
                "codexVersion": snapshot.codex_version,
                "startedAt": snapshot.started_at,
                "upstreamRestarts": snapshot.restarts,
            }
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": JSON_RPC_METHOD_NOT_FOUND,
                "message": format!("unknown gateway method `{method}`"),
            }
        })
    };
    response.to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{ClientTextRoute, respond_to_akra_request, route_client_text};
    use crate::application::port::outbound::gateway_upstream_port::GatewayUpstreamSnapshot;

    #[test]
    fn app_server_messages_are_forwarded_without_parsing() {
        for text in [
            r#"{"jsonrpc":"2.0","id":1,"method":"thread/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{"decision":"accept"}}"#,
            "not json at all",
        ] {
            assert_eq!(route_client_text(text), ClientTextRoute::Forward, "{text}");
        }
    }

    #[test]
    fn akra_text_inside_params_does_not_capture_an_app_server_request() {
        let text = r#"{"jsonrpc":"2.0","id":2,"method":"turn/start","params":{"input":"akra/gateway/info"}}"#;
        assert_eq!(route_client_text(text), ClientTextRoute::Forward);
    }

    #[test]
    fn akra_requests_and_notifications_are_answered_by_the_gateway() {
        assert_eq!(
            route_client_text(r#"{"jsonrpc":"2.0","id":"a","method":"akra/gateway/info"}"#),
            ClientTextRoute::AkraRequest {
                id: json!("a"),
                method: "akra/gateway/info".to_string(),
            }
        );
        assert_eq!(
            route_client_text(r#"{"jsonrpc":"2.0","method":"akra/ping"}"#),
            ClientTextRoute::AkraNotification {
                method: "akra/ping".to_string(),
            }
        );
    }

    #[test]
    fn gateway_info_reports_versions_and_unknown_methods_get_method_not_found() {
        let snapshot = GatewayUpstreamSnapshot {
            codex_version: Some("codex-cli 0.166.0".to_string()),
            started_at: "2026-11-05T00:00:00+00:00".to_string(),
            restarts: 2,
        };
        let info: Value = serde_json::from_str(&respond_to_akra_request(
            json!(1),
            "akra/gateway/info",
            &snapshot,
        ))
        .unwrap();
        assert_eq!(info["id"], json!(1));
        assert_eq!(info["result"]["codexVersion"], json!("codex-cli 0.166.0"));
        assert_eq!(info["result"]["upstreamRestarts"], json!(2));
        assert_eq!(
            info["result"]["gatewayVersion"],
            json!(env!("CARGO_PKG_VERSION"))
        );

        let unknown: Value =
            serde_json::from_str(&respond_to_akra_request(json!(2), "akra/nope", &snapshot))
                .unwrap();
        assert_eq!(unknown["error"]["code"], json!(-32601));
    }
}
