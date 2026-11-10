//! Gateway 의 `akra/mcp/*` 처리이다 (plan 21 2단계). Gateway 는 hub 엔진(hub 전용 codex app-server)을 유지하고,
//! `akra mcp` 클라이언트의 조회와 호출을 그 엔진으로 처리한다. 엔진 호출은 블로킹이므로 요청마다 블로킹 작업으로
//! 돌린다. 접속의 읽기 고리는 기다리지 않는다. 엔진은 요청을 하나씩 처리한다.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::recorder::{RecordDirection, SessionRecording};
use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
use crate::domain::mcp_hub::{
    MCP_HUB_GATEWAY_CALL, MCP_HUB_GATEWAY_OTHER_HUB, MCP_HUB_GATEWAY_RELOAD,
    MCP_HUB_GATEWAY_STATUSES,
};

pub(super) use crate::domain::mcp_hub::MCP_HUB_GATEWAY_METHOD_PREFIX as MCP_METHOD_PREFIX;

const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const JSON_RPC_INVALID_PARAMS: i64 = -32602;
const MCP_HUB_ERROR: i64 = -32000;

/// Gateway 가 유지하는 hub 엔진과 그 hub 폴더이다. 다른 hub 폴더의 요청은 처리하지 않는다.
pub(crate) struct GatewayMcpHub {
    home: String,
    backend: Arc<dyn McpHubBackendPort>,
}

impl GatewayMcpHub {
    pub(crate) fn new(home: String, backend: Arc<dyn McpHubBackendPort>) -> Self {
        Self { home, backend }
    }

    /// Gateway 의 한가한 커널 정리 주기에 함께 부른다.
    pub(super) fn release_if_idle(&self) {
        self.backend.release_if_idle();
    }

    /// Gateway 가 멈출 때 엔진을 내린다.
    pub(super) fn shutdown(&self) {
        if let Err(error) = self.backend.reload() {
            tracing::warn!(error = %format!("{error:#}"), "MCP hub engine shutdown failed");
        }
    }
}

#[derive(Deserialize)]
struct RequestEnvelope {
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HubParams {
    hub_home: String,
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    arguments: Option<Value>,
}

/// 요청 하나를 블로킹 작업으로 처리하고 답을 접속의 보내기 함으로 보낸다.
pub(super) fn spawn_request(
    hub: Arc<GatewayMcpHub>,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
    recording: Option<Arc<SessionRecording>>,
) {
    tokio::spawn(async move {
        let fallback_id = id.clone();
        let response = tokio::task::spawn_blocking(move || respond(&hub, id, &method, &text))
            .await
            .unwrap_or_else(|_| {
                error_response(fallback_id, MCP_HUB_ERROR, "the MCP hub request stopped")
            });
        if let Some(recording) = &recording {
            recording.record(RecordDirection::Gateway, &response);
        }
        let _ = outbox.send(Message::text(response)).await;
    });
}

fn respond(hub: &GatewayMcpHub, id: Value, method: &str, text: &str) -> String {
    let params = match serde_json::from_str::<RequestEnvelope>(text)
        .ok()
        .and_then(|envelope| serde_json::from_value::<HubParams>(envelope.params).ok())
    {
        Some(params) => params,
        None => return error_response(id, JSON_RPC_INVALID_PARAMS, "`hubHome` is required"),
    };
    if params.hub_home != hub.home {
        return error_response(
            id,
            MCP_HUB_GATEWAY_OTHER_HUB,
            "this gateway serves a different MCP hub",
        );
    }
    let result = match method {
        MCP_HUB_GATEWAY_STATUSES => hub
            .backend
            .server_statuses(params.server.as_deref())
            .and_then(|statuses| Ok(json!({ "statuses": serde_json::to_value(statuses)? }))),
        MCP_HUB_GATEWAY_CALL => {
            let (Some(server), Some(tool)) = (params.server, params.tool) else {
                return error_response(
                    id,
                    JSON_RPC_INVALID_PARAMS,
                    "`server` and `tool` are required",
                );
            };
            let arguments = params.arguments.unwrap_or_else(|| json!({}));
            hub.backend.call_tool(&server, &tool, arguments)
        }
        MCP_HUB_GATEWAY_RELOAD => hub.backend.reload().map(|()| json!({})),
        _ => {
            return error_response(
                id,
                JSON_RPC_METHOD_NOT_FOUND,
                &format!("unknown gateway method `{method}`"),
            );
        }
    };
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        Err(error) => error_response(id, MCP_HUB_ERROR, &format!("{error:#}")),
    }
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mcp_hub::{McpHubServerSpec, McpHubServerStatus};
    use anyhow::Result;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeBackend {
        reloads: Mutex<usize>,
        calls: Mutex<Vec<(String, String, Value)>>,
    }

    impl McpHubBackendPort for FakeBackend {
        fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>> {
            Ok(vec![])
        }
        fn register_server(&self, _spec: &McpHubServerSpec) -> Result<()> {
            Ok(())
        }
        fn unregister_server(&self, _name: &str) -> Result<bool> {
            Ok(false)
        }
        fn server_statuses(&self, only: Option<&str>) -> Result<Vec<McpHubServerStatus>> {
            Ok(vec![McpHubServerStatus {
                name: only.unwrap_or("all").to_string(),
                auth: "oAuth".into(),
                connection: "connected".into(),
                tools: vec![],
                error: None,
            }])
        }
        fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value> {
            if tool == "explode" {
                anyhow::bail!("tool exploded");
            }
            self.calls
                .lock()
                .unwrap()
                .push((server.into(), tool.into(), arguments));
            Ok(json!({ "content": [] }))
        }
        fn login(&self, _server: &str, _on_url: &mut dyn FnMut(&str)) -> Result<()> {
            Ok(())
        }
        fn reload(&self) -> Result<()> {
            *self.reloads.lock().unwrap() += 1;
            Ok(())
        }
    }

    fn hub() -> (GatewayMcpHub, Arc<FakeBackend>) {
        let backend = Arc::new(FakeBackend::default());
        (
            GatewayMcpHub::new("/home/.akra/mcp-hub".into(), backend.clone()),
            backend,
        )
    }

    fn request(method: &str, params: Value) -> String {
        json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }).to_string()
    }

    fn answer(hub: &GatewayMcpHub, method: &str, params: Value) -> Value {
        serde_json::from_str(&respond(hub, json!(7), method, &request(method, params))).unwrap()
    }

    #[test]
    fn statuses_and_calls_run_on_the_resident_backend() {
        let (hub, backend) = hub();
        let statuses = answer(
            &hub,
            MCP_HUB_GATEWAY_STATUSES,
            json!({ "hubHome": "/home/.akra/mcp-hub", "server": "notion" }),
        );
        assert_eq!(statuses["id"], 7);
        assert_eq!(statuses["result"]["statuses"][0]["name"], "notion");

        let call = answer(
            &hub,
            MCP_HUB_GATEWAY_CALL,
            json!({ "hubHome": "/home/.akra/mcp-hub", "server": "notion", "tool": "search", "arguments": { "query": "x" } }),
        );
        assert_eq!(call["result"], json!({ "content": [] }));
        assert_eq!(backend.calls.lock().unwrap()[0].2["query"], "x");

        let reload = answer(
            &hub,
            MCP_HUB_GATEWAY_RELOAD,
            json!({ "hubHome": "/home/.akra/mcp-hub" }),
        );
        assert_eq!(reload["result"], json!({}));
        assert_eq!(*backend.reloads.lock().unwrap(), 1);
    }

    #[test]
    fn other_hubs_missing_params_and_unknown_methods_are_rejected() {
        let (hub, backend) = hub();
        let other = answer(
            &hub,
            MCP_HUB_GATEWAY_CALL,
            json!({ "hubHome": "/other", "server": "a", "tool": "b" }),
        );
        assert_eq!(other["error"]["code"], MCP_HUB_GATEWAY_OTHER_HUB);
        let missing = answer(&hub, MCP_HUB_GATEWAY_CALL, json!({ "server": "a" }));
        assert_eq!(missing["error"]["code"], JSON_RPC_INVALID_PARAMS);
        let no_tool = answer(
            &hub,
            MCP_HUB_GATEWAY_CALL,
            json!({ "hubHome": "/home/.akra/mcp-hub", "server": "a" }),
        );
        assert_eq!(no_tool["error"]["code"], JSON_RPC_INVALID_PARAMS);
        let unknown = answer(
            &hub,
            "akra/mcp/other",
            json!({ "hubHome": "/home/.akra/mcp-hub" }),
        );
        assert_eq!(unknown["error"]["code"], JSON_RPC_METHOD_NOT_FOUND);
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn backend_errors_become_hub_errors() {
        let (hub, _) = hub();
        let failed = answer(
            &hub,
            MCP_HUB_GATEWAY_CALL,
            json!({ "hubHome": "/home/.akra/mcp-hub", "server": "a", "tool": "explode" }),
        );
        assert_eq!(failed["error"]["code"], MCP_HUB_ERROR);
        assert!(
            failed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("tool exploded")
        );
    }
}
