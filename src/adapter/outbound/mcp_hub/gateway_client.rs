//! `akra mcp` 가 Gateway 의 상주 hub 엔진을 쓰는 길이다.
//!
//! 조회와 호출은 Gateway 로 먼저 보낸다. Gateway 가 없거나, 오래된 Gateway 이거나, 다른 hub 폴더를 쓰면 직접
//! 실행한다. 이 판단은 요청을 보내기 전이나 Gateway 가 요청을 처리하지 않았다고 확실히 답했을 때만 한다. 그래서
//! 쓰기 도구가 두 번 실행되지 않는다. 등록부와 로그인은 직접 바꾸고, 그 뒤 Gateway 에 상주 엔진을 내리라고 알린다.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
use crate::domain::mcp_hub::{
    MCP_HUB_GATEWAY_CALL, MCP_HUB_GATEWAY_OTHER_HUB, MCP_HUB_GATEWAY_RELOAD,
    MCP_HUB_GATEWAY_STATUSES, McpHubServerSpec, McpHubServerStatus, McpHubSharedSkill,
    McpHubSharedSkillAction,
};

const GATEWAY_WEBSOCKET_URL: &str = "ws://localhost/rpc";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
// 상주 엔진의 첫 요청은 app-server 시작과 원격 MCP handshake 를 기다린다. 도구 호출도 오래 걸릴 수 있다.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const RELOAD_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq)]
pub(super) enum GatewayReply {
    /// Gateway 가 요청을 처리하지 않았다. 직접 실행해도 된다.
    Unavailable,
    Done(Value),
}

pub(super) trait GatewayHubTransport: Send + Sync {
    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<GatewayReply>;
}

pub(super) struct GatewayHubClient {
    hub_home: String,
}

impl GatewayHubClient {
    pub(super) fn new(hub_home: String) -> Self {
        Self { hub_home }
    }
}

impl GatewayHubTransport for GatewayHubClient {
    fn request(&self, method: &str, mut params: Value, timeout: Duration) -> Result<GatewayReply> {
        let Ok(endpoint) = crate::gateway_endpoint::client_endpoint() else {
            return Ok(GatewayReply::Unavailable);
        };
        params["hubHome"] = json!(self.hub_home);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to start the MCP hub gateway client")?;
        runtime.block_on(request_over_gateway(&endpoint, method, params, timeout))
    }
}

async fn request_over_gateway(
    endpoint: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<GatewayReply> {
    let connected = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let stream = connect(endpoint).await.ok()?;
        tokio_tungstenite::client_async(GATEWAY_WEBSOCKET_URL, stream)
            .await
            .ok()
    })
    .await;
    let Ok(Some((mut websocket, _response))) = connected else {
        return Ok(GatewayReply::Unavailable);
    };
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    if websocket
        .send(Message::text(request.to_string()))
        .await
        .is_err()
    {
        // 보내지 못했으면 Gateway 는 이 요청을 모른다.
        return Ok(GatewayReply::Unavailable);
    }
    let reply = tokio::time::timeout(timeout, async {
        while let Some(message) = websocket.next().await {
            match message.context("the gateway connection failed")? {
                Message::Text(text) => {
                    let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    if value.get("id") == Some(&json!(1)) && value.get("method").is_none() {
                        return reply_from_response(&value);
                    }
                }
                Message::Close(frame) => return reply_from_close(frame.map(|frame| frame.code)),
                _ => {}
            }
        }
        bail!("the gateway closed the connection before it answered `{method}`")
    })
    .await
    .with_context(|| format!("the gateway did not answer `{method}` in time"))?;
    let _ = websocket.close(None).await;
    reply
}

#[cfg(unix)]
async fn connect(endpoint: &str) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(endpoint).await
}

#[cfg(windows)]
async fn connect(
    endpoint: &str,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)
}

/// Gateway 의 JSON-RPC 응답을 해석한다. 모르는 메서드(오래된 Gateway)와 다른 hub 폴더는 처리하지 않았다는 뜻이다.
fn reply_from_response(response: &Value) -> Result<GatewayReply> {
    if let Some(error) = response.get("error") {
        let code = error.get("code").and_then(Value::as_i64);
        if matches!(
            code,
            Some(JSON_RPC_METHOD_NOT_FOUND | MCP_HUB_GATEWAY_OTHER_HUB)
        ) {
            return Ok(GatewayReply::Unavailable);
        }
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown gateway error");
        bail!("{message}")
    }
    Ok(GatewayReply::Done(
        response.get("result").cloned().unwrap_or(Value::Null),
    ))
}

/// Gateway 는 upstream 을 열지 못하면 클라이언트 메시지를 읽기 전에 1013 으로 닫는다. 이때만 다시 실행해도 된다.
fn reply_from_close(code: Option<CloseCode>) -> Result<GatewayReply> {
    if code == Some(CloseCode::Again) {
        return Ok(GatewayReply::Unavailable);
    }
    bail!("the gateway closed the connection before it answered")
}

/// CLI 가 쓰는 backend 이다. 조회와 호출은 Gateway 를 먼저 쓰고, 나머지는 직접 실행한다.
pub(crate) struct GatewayRoutedMcpHubBackend {
    local: Arc<dyn McpHubBackendPort>,
    gateway: Box<dyn GatewayHubTransport>,
}

impl GatewayRoutedMcpHubBackend {
    pub(crate) fn new(local: Arc<dyn McpHubBackendPort>, hub_home: String) -> Self {
        Self::with_transport(local, Box::new(GatewayHubClient::new(hub_home)))
    }

    pub(super) fn with_transport(
        local: Arc<dyn McpHubBackendPort>,
        gateway: Box<dyn GatewayHubTransport>,
    ) -> Self {
        Self { local, gateway }
    }

    /// 상주 엔진이 바뀐 등록부와 로그인을 다시 읽게 한다. Gateway 가 없으면 할 일이 없다.
    fn reload_gateway(&self) {
        if let Err(error) = self
            .gateway
            .request(MCP_HUB_GATEWAY_RELOAD, json!({}), RELOAD_TIMEOUT)
        {
            tracing::warn!(error = %format!("{error:#}"), "MCP hub gateway reload failed");
        }
    }
}

impl McpHubBackendPort for GatewayRoutedMcpHubBackend {
    fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>> {
        self.local.registered_servers()
    }

    fn register_server(&self, spec: &McpHubServerSpec) -> Result<()> {
        self.local.register_server(spec)?;
        self.reload_gateway();
        Ok(())
    }

    fn unregister_server(&self, name: &str) -> Result<bool> {
        let removed = self.local.unregister_server(name)?;
        if removed {
            self.reload_gateway();
        }
        Ok(removed)
    }

    fn server_statuses(&self, only: Option<&str>) -> Result<Vec<McpHubServerStatus>> {
        match self.gateway.request(
            MCP_HUB_GATEWAY_STATUSES,
            json!({ "server": only }),
            REQUEST_TIMEOUT,
        )? {
            GatewayReply::Done(result) => {
                serde_json::from_value(result.get("statuses").cloned().unwrap_or(Value::Null))
                    .context("the gateway returned malformed MCP server statuses")
            }
            GatewayReply::Unavailable => self.local.server_statuses(only),
        }
    }

    fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value> {
        let params = json!({ "server": server, "tool": tool, "arguments": arguments });
        match self
            .gateway
            .request(MCP_HUB_GATEWAY_CALL, params, REQUEST_TIMEOUT)?
        {
            GatewayReply::Done(result) => Ok(result),
            GatewayReply::Unavailable => self.local.call_tool(server, tool, arguments),
        }
    }

    fn login(&self, server: &str, on_authorization_url: &mut dyn FnMut(&str)) -> Result<()> {
        let result = self.local.login(server, on_authorization_url);
        self.reload_gateway();
        result
    }

    fn shared_skill(&self, action: McpHubSharedSkillAction) -> Result<McpHubSharedSkill> {
        self.local.shared_skill(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn responses_fall_back_only_when_the_gateway_did_not_handle_them() {
        let not_found = json!({ "id": 1, "error": { "code": -32601, "message": "unknown" } });
        let other_hub =
            json!({ "id": 1, "error": { "code": MCP_HUB_GATEWAY_OTHER_HUB, "message": "x" } });
        let failed = json!({ "id": 1, "error": { "code": -32000, "message": "tool exploded" } });
        let done = json!({ "id": 1, "result": { "content": [] } });

        assert_eq!(
            reply_from_response(&not_found).unwrap(),
            GatewayReply::Unavailable
        );
        assert_eq!(
            reply_from_response(&other_hub).unwrap(),
            GatewayReply::Unavailable
        );
        assert_eq!(
            reply_from_response(&failed).unwrap_err().to_string(),
            "tool exploded"
        );
        assert_eq!(
            reply_from_response(&done).unwrap(),
            GatewayReply::Done(json!({ "content": [] }))
        );
        assert_eq!(
            reply_from_close(Some(CloseCode::Again)).unwrap(),
            GatewayReply::Unavailable
        );
        assert!(reply_from_close(Some(CloseCode::Restart)).is_err());
        assert!(reply_from_close(None).is_err());
    }

    struct ScriptedGateway {
        reply: Mutex<Option<Result<GatewayReply>>>,
        methods: Arc<Mutex<Vec<String>>>,
    }

    impl GatewayHubTransport for ScriptedGateway {
        fn request(
            &self,
            method: &str,
            _params: Value,
            _timeout: Duration,
        ) -> Result<GatewayReply> {
            self.methods.lock().unwrap().push(method.to_string());
            if method == MCP_HUB_GATEWAY_RELOAD {
                return Ok(GatewayReply::Done(json!({})));
            }
            self.reply
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(GatewayReply::Unavailable))
        }
    }

    #[derive(Default)]
    struct LocalBackend {
        calls: Mutex<usize>,
    }

    impl McpHubBackendPort for LocalBackend {
        fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>> {
            Ok(vec![])
        }
        fn register_server(&self, _spec: &McpHubServerSpec) -> Result<()> {
            Ok(())
        }
        fn unregister_server(&self, _name: &str) -> Result<bool> {
            Ok(true)
        }
        fn server_statuses(&self, _only: Option<&str>) -> Result<Vec<McpHubServerStatus>> {
            *self.calls.lock().unwrap() += 1;
            Ok(vec![])
        }
        fn call_tool(&self, _server: &str, _tool: &str, _arguments: Value) -> Result<Value> {
            *self.calls.lock().unwrap() += 1;
            Ok(json!({ "local": true }))
        }
        fn login(&self, _server: &str, _on_url: &mut dyn FnMut(&str)) -> Result<()> {
            Ok(())
        }
    }

    fn routed(
        reply: Result<GatewayReply>,
    ) -> (
        GatewayRoutedMcpHubBackend,
        Arc<LocalBackend>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let local = Arc::new(LocalBackend::default());
        let methods = Arc::new(Mutex::new(Vec::new()));
        let gateway = ScriptedGateway {
            reply: Mutex::new(Some(reply)),
            methods: methods.clone(),
        };
        (
            GatewayRoutedMcpHubBackend::with_transport(local.clone(), Box::new(gateway)),
            local,
            methods,
        )
    }

    #[test]
    fn calls_use_the_gateway_result_without_running_locally() {
        let (backend, local, _) = routed(Ok(GatewayReply::Done(json!({ "gateway": true }))));
        let result = backend.call_tool("notion", "search", json!({})).unwrap();
        assert_eq!(result["gateway"], true);
        assert_eq!(*local.calls.lock().unwrap(), 0);
    }

    #[test]
    fn calls_run_locally_only_when_the_gateway_is_unavailable() {
        let (backend, local, _) = routed(Ok(GatewayReply::Unavailable));
        assert_eq!(
            backend.call_tool("notion", "search", json!({})).unwrap()["local"],
            true
        );
        assert_eq!(*local.calls.lock().unwrap(), 1);
    }

    #[test]
    fn gateway_failures_after_sending_are_errors_not_retries() {
        let (backend, local, _) = routed(Err(anyhow::anyhow!("connection dropped")));
        assert!(backend.call_tool("notion", "search", json!({})).is_err());
        assert_eq!(*local.calls.lock().unwrap(), 0);
    }

    #[test]
    fn statuses_decode_the_gateway_payload() {
        let status = json!({
            "statuses": [{
                "name": "notion", "auth": "oAuth", "connection": "connected",
                "tools": [{ "name": "search", "description": "Find", "input_schema": {} }],
                "error": null
            }]
        });
        let (backend, local, _) = routed(Ok(GatewayReply::Done(status)));
        let statuses = backend.server_statuses(None).unwrap();
        assert_eq!(statuses[0].tools[0].name, "search");
        assert_eq!(*local.calls.lock().unwrap(), 0);
    }

    #[test]
    fn registry_changes_and_login_tell_the_gateway_to_reload() {
        let (backend, _, methods) = routed(Ok(GatewayReply::Unavailable));
        backend
            .register_server(
                &McpHubServerSpec::http("notion", "https://mcp.notion.com/mcp").unwrap(),
            )
            .unwrap();
        backend.unregister_server("notion").unwrap();
        backend.login("notion", &mut |_| {}).unwrap();
        assert_eq!(*methods.lock().unwrap(), vec![MCP_HUB_GATEWAY_RELOAD; 3]);
    }
}
