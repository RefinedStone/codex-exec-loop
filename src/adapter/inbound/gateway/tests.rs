//! 가짜 upstream 으로 세션 중계를 시험한다. upstream 은 받은 텍스트 메시지를 `echo:` 를 붙여 되돌리는
//! WebSocket 서버이다. 실제 codex 는 실행하지 않는다.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio::sync::watch;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::GatewayCommand;
use super::hosts::GatewayHosts;
use super::parse_args;
use super::session::{run_session, websocket_config};
use crate::application::port::inbound::issue_draft_port as creation;
use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamSnapshot, GatewayUpstreamStream,
};

const DUPLEX_BUFFER_BYTES: usize = 64 * 1024;
const TEST_TIMEOUT: Duration = Duration::from_secs(10);
mod chat_routes;
mod conversation_workspace;
// 이슈 게시, 복원, 외부 변경 흐름의 검증이다 (#2429). Vault 파일 저장소는 unix 에서만 동작한다.
mod human_confirmation;
mod issue_decision;
#[cfg(unix)]
mod issue_flow;
mod issue_intervention;
#[cfg(any(unix, windows))]
mod issue_shared_copy_flow;
mod provider_credentials;
mod repository_extensions;
mod resource_versions;

struct EchoUpstream {
    generation: watch::Sender<u64>,
    available: bool,
    opened: Mutex<u32>,
}

impl EchoUpstream {
    fn new(available: bool) -> Arc<Self> {
        Arc::new(Self {
            generation: watch::channel(0).0,
            available,
            opened: Mutex::new(0),
        })
    }

    fn restart(&self) {
        self.generation.send_modify(|generation| *generation += 1);
    }
}

impl GatewayUpstreamPort for EchoUpstream {
    fn open_session(&self) -> Result<Box<dyn GatewayUpstreamStream>> {
        if !self.available {
            bail!("upstream down")
        }
        *self.opened.lock().unwrap() += 1;
        let (gateway_side, upstream_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
        tokio::spawn(async move {
            let Ok(mut websocket) = tokio_tungstenite::accept_async_with_config(
                upstream_side,
                Some(websocket_config()),
            )
            .await
            else {
                return;
            };
            while let Some(Ok(message)) = websocket.next().await {
                match message {
                    Message::Text(text) => {
                        let reply = format!("echo:{}", text.as_str());
                        if websocket.send(Message::text(reply)).await.is_err() {
                            return;
                        }
                    }
                    Message::Close(_) => return,
                    _ => {}
                }
            }
        });
        Ok(Box::new(gateway_side))
    }

    fn snapshot(&self) -> GatewayUpstreamSnapshot {
        GatewayUpstreamSnapshot {
            codex_version: Some("codex-cli test".to_string()),
            started_at: "2026-11-05T00:00:00+00:00".to_string(),
            restarts: 0,
        }
    }

    fn generation(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    fn shutdown(&self) {}
}

async fn connect_client(upstream: Arc<EchoUpstream>) -> WebSocketStream<DuplexStream> {
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    tokio::spawn(async move {
        let _ = run_session(
            gateway_side,
            GatewayHosts::new(upstream, Vec::new()),
            Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
            super::session::GatewayServices::default(),
            None,
        )
        .await;
    });
    let (websocket, _) = tokio_tungstenite::client_async_with_config(
        "ws://localhost/",
        client_side,
        Some(websocket_config()),
    )
    .await
    .expect("client handshake through the gateway session should succeed");
    websocket
}

async fn next_text(websocket: &mut WebSocketStream<DuplexStream>) -> String {
    loop {
        let message = tokio::time::timeout(TEST_TIMEOUT, websocket.next())
            .await
            .expect("message should arrive in time")
            .expect("stream should stay open")
            .expect("message should be valid");
        if let Message::Text(text) = message {
            return text.as_str().to_string();
        }
    }
}

async fn next_close_code(websocket: &mut WebSocketStream<DuplexStream>) -> Option<CloseCode> {
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, websocket.next())
            .await
            .expect("close should arrive in time")
        {
            Some(Ok(Message::Close(frame))) => return frame.map(|frame| frame.code),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return None,
        }
    }
}

#[tokio::test]
async fn app_server_messages_are_relayed_in_order() {
    let upstream = EchoUpstream::new(true);
    let mut client = connect_client(upstream.clone()).await;

    for index in 0..50 {
        let request = json!({"jsonrpc": "2.0", "id": index, "method": "thread/list"}).to_string();
        client.send(Message::text(request)).await.unwrap();
    }
    for index in 0..50 {
        let reply = next_text(&mut client).await;
        let body: Value = serde_json::from_str(reply.strip_prefix("echo:").unwrap()).unwrap();
        assert_eq!(body["id"], json!(index), "replies must keep request order");
    }
}

#[tokio::test]
async fn messages_larger_than_the_tungstenite_default_frame_are_relayed() {
    // A long thread's thread/resume response exceeded tungstenite's 16 MiB frame / 64 MiB message defaults
    // and silently closed the session (found by opening real threads in Akra Desktop).
    let upstream = EchoUpstream::new(true);
    let mut client = connect_client(upstream).await;
    let large = "x".repeat(20 << 20);
    client.send(Message::text(large.clone())).await.unwrap();
    let reply = next_text(&mut client).await;
    assert_eq!(reply.len(), "echo:".len() + large.len());
}

#[tokio::test]
async fn gateway_info_is_answered_locally_and_never_reaches_upstream() {
    let upstream = EchoUpstream::new(true);
    let mut client = connect_client(upstream.clone()).await;

    client
        .send(Message::text(
            r#"{"jsonrpc":"2.0","id":"info","method":"akra/gateway/info"}"#,
        ))
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
    assert_eq!(reply["id"], json!("info"));
    assert_eq!(reply["result"]["codexVersion"], json!("codex-cli test"));
}

#[tokio::test]
async fn each_client_gets_its_own_upstream_connection() {
    let upstream = EchoUpstream::new(true);
    let mut first = connect_client(upstream.clone()).await;
    let mut second = connect_client(upstream.clone()).await;

    // Both clients reuse request id 1, as the app-server allows per connection.
    first
        .send(Message::text(r#"{"id":1,"method":"a"}"#))
        .await
        .unwrap();
    second
        .send(Message::text(r#"{"id":1,"method":"b"}"#))
        .await
        .unwrap();
    assert_eq!(next_text(&mut first).await, r#"echo:{"id":1,"method":"a"}"#);
    assert_eq!(
        next_text(&mut second).await,
        r#"echo:{"id":1,"method":"b"}"#
    );
    assert_eq!(*upstream.opened.lock().unwrap(), 2);
}

#[tokio::test]
async fn upstream_restart_closes_clients_with_service_restart_code() {
    let upstream = EchoUpstream::new(true);
    let mut client = connect_client(upstream.clone()).await;
    client.send(Message::text(r#"{"id":1}"#)).await.unwrap();
    next_text(&mut client).await;

    upstream.restart();
    assert_eq!(next_close_code(&mut client).await, Some(CloseCode::Restart));
}

#[tokio::test]
async fn unavailable_upstream_closes_the_client_with_try_again_later() {
    let upstream = EchoUpstream::new(false);
    let mut client = connect_client(upstream).await;
    assert_eq!(next_close_code(&mut client).await, Some(CloseCode::Again));
}

#[test]
fn arguments_accept_help_and_desktop_browser_bridge() {
    assert_eq!(parse_args(&[]).unwrap(), GatewayCommand::Run);
    assert_eq!(
        parse_args(&["--help".to_string()]).unwrap(),
        GatewayCommand::Help
    );
    assert_eq!(
        parse_args(&["desktop-browser-bridge".to_string()]).unwrap(),
        GatewayCommand::DesktopBrowserBridge
    );
    assert!(parse_args(&["desktop-browser-bridge".to_string(), "extra".to_string()]).is_err());
    let error = parse_args(&["--port".to_string()]).unwrap_err().to_string();
    assert!(error.contains("unsupported argument: --port"), "{error}");
}

/// 실제 저장소 커널(자기 app-server, 시작 점검 없음)에 붙는 접속이다.
async fn connect_kernel_client(
    kernels: Arc<crate::composition::repository_kernel::RepositoryKernelRegistry>,
) -> WebSocketStream<DuplexStream> {
    connect_kernel_client_with_services(kernels, super::session::GatewayServices::default()).await
}

async fn connect_kernel_client_with_services(
    kernels: Arc<crate::composition::repository_kernel::RepositoryKernelRegistry>,
    services: super::session::GatewayServices,
) -> WebSocketStream<DuplexStream> {
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let upstream = EchoUpstream::new(true);
    tokio::spawn(async move {
        let _ = run_session(
            gateway_side,
            GatewayHosts::new(upstream, Vec::new()),
            kernels,
            services,
            None,
        )
        .await;
    });
    let (websocket, _) = tokio_tungstenite::client_async_with_config(
        "ws://localhost/",
        client_side,
        Some(websocket_config()),
    )
    .await
    .expect("client handshake through the gateway session should succeed");
    websocket
}

#[tokio::test(flavor = "multi_thread")]
async fn delegation_selected_chat_crosses_gateway_with_account_and_bounded_recovery() {
    let folder = KernelTestFolder::new("selected-chat");
    let auth_path = folder.path.join("fixture-auth.json");
    const ACCOUNT: &str = "fixture-account";
    const URL: &str = "https://chatgpt.com/c/12345678-1234-1234-1234-123456789abc";
    std::fs::write(
        &auth_path,
        json!({"tokens":{"access_token":"fixture", "account_id":ACCOUNT}}).to_string(),
    )
    .unwrap();
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut delegation = super::delegation::Delegations::default();
    delegation.set_test_account_reader(move || {
        crate::adapter::outbound::app_server::cloud_upstream::chat_account_id(&auth_path)
    });
    let services = super::session::GatewayServices {
        delegations: Arc::new(delegation),
        ..Default::default()
    };
    let mut client = connect_kernel_client_with_services(kernels.clone(), services.clone()).await;
    let mut browser = connect_kernel_client_with_services(kernels, services).await;
    attach_kernel(&mut client, "attach", &folder.workspace()).await;
    let registration = kernel_request(
        &mut browser,
        "register",
        "akra/delegationDriver/register",
        json!({}),
    )
    .await;
    assert!(registration.get("result").is_some(), "{registration}");
    let callback_root = std::fs::canonicalize(folder.workspace()).unwrap();
    let driver = tokio::spawn(async move {
        let mut submits = 0;
        let mut observations = 0;
        loop {
            let request =
                next_json_matching(&mut browser, |message| message["method"].is_string()).await;
            let result = match request["method"].as_str().unwrap() {
                "akra/delegationDriver/status" => {
                    if !request["params"]["expectedAccountId"].is_null() {
                        assert_eq!(request["params"]["expectedAccountId"], ACCOUNT);
                    }
                    json!({"state":"ready","accountMatch":"match",
                        "tunnel":{"host":true,"readiness":"ready","install":"ready","connection":"unknown"}})
                }
                "akra/delegationDriver/submit" => {
                    submits += 1;
                    assert_eq!(
                        request["params"]["session"],
                        json!({"mode":"continue","chatUrl":URL})
                    );
                    assert_eq!(request["params"]["expectedAccountId"], ACCOUNT);
                    assert_eq!(
                        std::fs::canonicalize(
                            request["params"]["callback"]["workspace"].as_str().unwrap()
                        )
                        .unwrap(),
                        callback_root
                    );
                    assert!(
                        request["params"]["maxResumes"]
                            .as_u64()
                            .is_some_and(|n| n <= 2)
                    );
                    assert!(
                        request["params"]["callback"]["token"]
                            .as_str()
                            .is_some_and(|s| s.len() == 64)
                    );
                    json!({"delivery":"accepted","phase":"working","conversationUrl":URL,"recovery":"none","resumes":0})
                }
                "akra/delegationDriver/observe" => {
                    observations += 1;
                    if observations == 1 {
                        json!({"delivery":"observed","phase":"disconnected","recovery":"in_progress","resumes":1})
                    } else {
                        json!({"delivery":"observed","phase":"disconnected","recovery":"exhausted","resumes":2})
                    }
                }
                method => panic!("unexpected delegated fixture method {method}"),
            };
            assert!(submits <= 1, "observation must not resend the job");
            browser
                .send(Message::text(
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string(),
                ))
                .await
                .unwrap();
        }
    });

    let status = kernel_request(
        &mut client,
        "backends",
        "akra/kernel/query",
        json!({"query":{"type":"delegation_backends"}}),
    )
    .await;
    assert_eq!(
        status["result"]["chatgpt_chat"]["account_match"], "match",
        "{status}"
    );
    assert_eq!(
        status["result"]["chatgpt_chat"]["tunnel"]["connection"],
        "unknown"
    );
    assert_eq!(status["result"]["chatgpt_chat"]["circuit"], "closed");
    let submit = |url| {
        json!({"command":{
            "type":"submit_delegation","instruction":"Fixture request","idempotency_key":"selected-chat",
            "session":{"mode":"continue","chat_url":url}
        }})
    };
    let invalid = kernel_request(
        &mut client,
        "invalid",
        "akra/kernel/command",
        submit("https://example.test/c/abc"),
    )
    .await;
    assert_eq!(invalid["error"]["code"], -32602, "{invalid}");
    let accepted = kernel_request(&mut client, "submit", "akra/kernel/command", submit(URL)).await;
    let job = &accepted["result"]["result"]["delegation"];
    assert_eq!(job["session"]["chat_url"], URL, "{accepted}");
    let id = job["id"].as_str().unwrap();
    let changed = kernel_request(
        &mut client,
        "changed",
        "akra/kernel/command",
        submit("https://chatgpt.com/c/abcdefab-1234-1234-1234-123456789abc"),
    )
    .await;
    assert!(changed["result"]["failure"].is_string(), "{changed}");
    tokio::time::timeout(Duration::from_secs(12), async {
        let mut observed_recovery = false;
        loop {
            let detail = kernel_request(
                &mut client,
                "detail",
                "akra/kernel/query",
                json!({"query":{"type":"delegation","id":id}}),
            )
            .await;
            let job = &detail["result"]["delegation"];
            assert_eq!(job["session"]["chat_url"], URL, "{detail}");
            assert_eq!(job["can_cancel"], true, "{detail}");
            assert_eq!(job["can_retry"], false, "{detail}");
            assert!(!detail.to_string().contains(ACCOUNT));
            if job["attempts"][0]["recovery"] == "in_progress" {
                assert_eq!(job["current_backend"], "chatgpt_chat");
                assert_eq!(job["attempts"][0]["resumes"], 1);
                assert!(job["fallbacks"].is_null());
                observed_recovery = true;
            }
            if job["state"] == "awaiting_human" && job["attempts"][0]["recovery"] == "exhausted" {
                assert!(observed_recovery);
                assert_eq!(job["attempts"][0]["resumes"], 2);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("driver recovery should reach My Turn");
    let attention = kernel_request(
        &mut client,
        "my-turn",
        "akra/kernel/query",
        json!({"query":{"type":"delegation_my_turn"}}),
    )
    .await;
    assert_eq!(
        attention["result"]["items"],
        json!([{"id":id,"reason":"chat_recovery_exhausted"}]),
        "{attention}"
    );
    let store = crate::composition::production::delegation_store(
        crate::composition::production::delegation_runtime(&folder.workspace()).unwrap(),
        true,
    )
    .unwrap();
    let persisted = store
        .active()
        .unwrap()
        .into_iter()
        .find(|job| job.view.id == id)
        .unwrap();
    assert_eq!(persisted.view.session.selected_url(), Some(URL));
    assert_eq!(
        persisted.account_digest.as_deref(),
        Some(super::delegation::account_fingerprint(ACCOUNT).as_slice())
    );
    driver.abort();
}

async fn next_json_matching(
    websocket: &mut WebSocketStream<DuplexStream>,
    matches: impl Fn(&Value) -> bool,
) -> Value {
    loop {
        let value: Value = serde_json::from_str(&next_text(websocket).await).unwrap();
        if matches(&value) {
            return value;
        }
    }
}

async fn kernel_request(
    websocket: &mut WebSocketStream<DuplexStream>,
    id: &str,
    method: &str,
    params: Value,
) -> Value {
    websocket
        .send(Message::text(
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        ))
        .await
        .unwrap();
    next_json_matching(websocket, |value| value["id"] == json!(id)).await
}

#[tokio::test(flavor = "multi_thread")]
async fn delegation_commands_scope_callbacks_and_replay_retry_keys() {
    use crate::adapter::inbound::desktop_confirmation_bridge::storage::random_id;
    use crate::composition::production::{delegation_runtime, delegation_store};
    use crate::domain::delegation::State;

    let folder = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-gateway-delegation-{}",
        random_id(12).unwrap()
    ));
    std::fs::create_dir(&folder).unwrap();
    let workspace = folder.to_str().unwrap();
    let store = delegation_store(delegation_runtime(workspace).unwrap(), false).unwrap();
    let seed = |key| {
        let job = store
            .submit(key, "fixture instruction", "gateway-test")
            .unwrap();
        let mut dispatching = job.clone();
        dispatching.state = State::Dispatching;
        assert!(store.update(&job, &mut dispatching, None, false).unwrap());
        let token = store.issue_callback_capability(&job.id).unwrap().unwrap();
        (dispatching, token)
    };
    let retryable = store
        .submit("retryable", "fixture instruction", "gateway-test")
        .unwrap();
    store.cancel(&retryable.id).unwrap().unwrap();
    let (active, capability) = seed("active");
    let (expired, expired_capability) = seed("expired");
    let mut past_deadline = expired.clone();
    past_deadline.deadline = Some(0);
    assert!(
        store
            .update(&expired, &mut past_deadline, None, false)
            .unwrap()
    );

    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels).await;
    let attach = kernel_request(
        &mut client,
        "attach",
        "akra/kernel/attach",
        json!({"workspace": workspace, "client_name": "gateway-test", "transient": true}),
    )
    .await;
    assert!(attach["result"]["kernel"].is_string(), "{attach}");

    for (id, token) in [
        (&active.id, "wrong"),
        (&active.id, expired_capability.as_str()),
        (&expired.id, expired_capability.as_str()),
    ] {
        let rejected = kernel_request(
            &mut client,
            "callback",
            "akra/kernel/command",
            json!({"command": {"type": "report_delegation_result", "id": id, "token": token, "result": "fixture"}}),
        )
        .await;
        assert_eq!(
            rejected["result"]["failure"],
            "delegation callback rejected"
        );
        assert!(!rejected.to_string().contains(token));
    }
    for (kind, field, expected) in [
        (
            "checkpoint_delegation",
            "message",
            "delegation_checkpointed",
        ),
        (
            "report_delegation_result",
            "result",
            "delegation_result_reported",
        ),
    ] {
        let mut command = json!({"type": kind, "id": active.id, "token": capability});
        command[field] = json!("fixture");
        let response = kernel_request(
            &mut client,
            "callback",
            "akra/kernel/command",
            json!({"command": command}),
        )
        .await;
        assert_eq!(response["result"]["result"]["type"], expected, "{response}");
        assert!(!response.to_string().contains(&capability));
    }
    assert_eq!(
        store.result(&active.id).unwrap().as_deref(),
        Some("fixture")
    );

    let params = json!({
        "command": {"type": "retry_delegation", "id": retryable.id},
        "idempotency_key": "same-retry",
    });
    let mut new_id = None;
    for _ in 0..2 {
        let response =
            kernel_request(&mut client, "retry", "akra/kernel/command", params.clone()).await;
        let result = &response["result"]["result"];
        assert_eq!(result["type"], "delegation_retried", "{response}");
        let id = result["delegation"]["id"].as_str().unwrap();
        assert_ne!(id, retryable.id);
        assert_eq!(id, new_id.get_or_insert_with(|| id.to_owned()).as_str());
    }
    client.close(None).await.unwrap();
    drop(client);
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_attaches_commands_and_receives_kernel_events() {
    // plan 18 K3-5: akra/kernel/* reaches the repository kernel through the Gateway session.
    let folder = crate::test_utils::platform_safe_temp_dir()
        .join(format!("akra-gateway-kernel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let workspace = folder.display().to_string();
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;

    let refused = kernel_request(
        &mut client,
        "early",
        "akra/kernel/query",
        json!({"query": {"type": "auto_follow"}}),
    )
    .await;
    assert_eq!(refused["error"]["code"], json!(-32011), "{refused}");

    let attached = kernel_request(
        &mut client,
        "attach",
        "akra/kernel/attach",
        json!({"workspace": workspace, "client_name": "gateway-test"}),
    )
    .await;
    assert_eq!(
        attached["result"]["protocol_version"],
        json!(crate::adapter::inbound::kernel_protocol::PROTOCOL_VERSION),
        "{attached}"
    );
    assert!(
        attached["result"]["kernel"]
            .as_str()
            .is_some_and(|kernel| !kernel.is_empty())
    );

    // The event and the command response travel on different paths, so either may come first.
    client
        .send(Message::text(
            json!({
                "jsonrpc": "2.0",
                "id": "command",
                "method": "akra/kernel/command",
                "params": {"command": {"type": "set_auto_follow_max_turns", "value": 3}},
            })
            .to_string(),
        ))
        .await
        .unwrap();
    let (mut commanded, mut event) = (None, None);
    while commanded.is_none() || event.is_none() {
        let value: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
        if value["id"] == json!("command") {
            commanded = Some(value);
        } else if value["params"]["event"]["type"] == json!("auto_follow") {
            event = Some(value);
        }
    }
    // plan 19 Q3-6: a recorded command answers with its receipt. The operation id is a log position.
    let commanded = commanded.unwrap()["result"].clone();
    assert_eq!(commanded["processed"], json!(true), "{commanded}");
    assert_eq!(
        commanded["operation"]["state"],
        json!("accepted"),
        "{commanded}"
    );
    assert!(
        commanded["operation"]["op_id"]
            .as_i64()
            .is_some_and(|op_id| op_id > 0)
    );
    assert_eq!(
        commanded["operation"]["next"],
        json!({"query": "history", "after": commanded["operation"]["op_id"]})
    );
    assert_eq!(event.unwrap()["params"]["event"]["max_turns"], json!(3));

    // plan 18 K4-4: the operation query answers with the state and the records of that command. This command
    // ends when the kernel accepts it, so it is done. The query sees the record before the recorder wrote it.
    let operation = kernel_request(
        &mut client,
        "operation",
        "akra/kernel/query",
        json!({"query": {"type": "operation", "op_id": commanded["operation"]["op_id"]}}),
    )
    .await;
    let operation = &operation["result"];
    assert_eq!(operation["type"], json!("operation"), "{operation}");
    assert_eq!(operation["state"], json!("done"), "{operation}");
    assert_eq!(
        operation["records"][0]["event"]["kind"],
        json!("command_received"),
        "{operation}"
    );
    assert_eq!(
        operation["records"][0]["sequence"], commanded["operation"]["op_id"],
        "{operation}"
    );
    // plan 18 K4-5: the record names the connection that sent the command.
    assert_eq!(
        operation["records"][0]["event"]["caller"],
        json!("gateway-test"),
        "{operation}"
    );
    let unknown = kernel_request(
        &mut client,
        "unknown-operation",
        "akra/kernel/query",
        json!({"query": {"type": "operation", "op_id": 999_999}}),
    )
    .await;
    assert_eq!(unknown["result"]["state"], json!("unknown"), "{unknown}");

    // A resend with the same key does not run again and gets the first receipt.
    let keyed = |id: &'static str, value: u64| {
        (
            id,
            json!({
                "command": {"type": "set_auto_follow_max_turns", "value": value},
                "idempotency_key": "key-1",
            }),
        )
    };
    let (id, params) = keyed("keyed", 4);
    let first = kernel_request(&mut client, id, "akra/kernel/command", params).await;
    let (id, params) = keyed("resent", 4);
    let resent = kernel_request(&mut client, id, "akra/kernel/command", params).await;
    assert_eq!(
        first["result"]["operation"]["replayed"],
        Value::Null,
        "{first}"
    );
    assert_eq!(
        resent["result"]["operation"]["replayed"],
        json!(true),
        "{resent}"
    );
    assert_eq!(
        resent["result"]["operation"]["op_id"], first["result"]["operation"]["op_id"],
        "{resent}"
    );
    let (id, params) = keyed("reused", 5);
    let reused = kernel_request(&mut client, id, "akra/kernel/command", params).await;
    assert_eq!(reused["error"]["code"], json!(-32014), "{reused}");

    // A command that the kernel refuses at once answers `rejected`, and a resend gets the same refusal.
    let unknown_thread = |id: &'static str| {
        (
            id,
            json!({
                "command": {"type": "submit_prompt", "thread_id": "no-such-thread", "prompt": "hello"},
                "idempotency_key": "key-2",
            }),
        )
    };
    for (id, replayed) in [("refused", Value::Null), ("refused-again", json!(true))] {
        let (id, params) = unknown_thread(id);
        let refused = kernel_request(&mut client, id, "akra/kernel/command", params).await;
        let result = &refused["result"];
        assert_eq!(
            result["rejection"]["reason"],
            json!("unknown_thread"),
            "{refused}"
        );
        assert_eq!(result["operation"]["state"], json!("rejected"), "{refused}");
        assert_eq!(result["operation"]["replayed"], replayed, "{refused}");
    }

    let queried = kernel_request(
        &mut client,
        "query",
        "akra/kernel/query",
        json!({"query": {"type": "auto_follow"}}),
    )
    .await;
    assert_eq!(queried["result"]["type"], json!("auto_follow"), "{queried}");
    // The keyed command (4) ran once. The command that reused its key (5) did not run.
    assert_eq!(queried["result"]["max_turns"], json!(4), "{queried}");

    // plan 18 K3-6: the planning queue and the parallel state travel through the same session.
    let refreshed = kernel_request(
        &mut client,
        "refresh",
        "akra/kernel/command",
        json!({"command": {"type": "refresh_planning"}}),
    )
    .await;
    assert_eq!(
        refreshed["result"],
        json!({"processed": true}),
        "{refreshed}"
    );
    let planning = kernel_request(
        &mut client,
        "planning",
        "akra/kernel/query",
        json!({"query": {"type": "planning"}}),
    )
    .await;
    assert_eq!(planning["result"]["type"], json!("planning"), "{planning}");
    let parallel = kernel_request(
        &mut client,
        "parallel",
        "akra/kernel/query",
        json!({"query": {"type": "parallel"}}),
    )
    .await;
    assert_eq!(parallel["result"]["type"], json!("parallel"), "{parallel}");
    // plan 19 Q3-3: the history query goes through the host thread; this kernel has no log.
    let history = kernel_request(
        &mut client,
        "history",
        "akra/kernel/query",
        json!({"query": {"type": "history", "after": 7}}),
    )
    .await;
    assert_eq!(history["result"]["type"], json!("history"), "{history}");
    assert_eq!(history["result"]["records"], json!([]), "{history}");
    assert_eq!(history["result"]["next_after"], json!(7), "{history}");
    assert_eq!(parallel["result"]["enabled"], json!(false), "{parallel}");

    let described =
        kernel_request(&mut client, "describe", "akra/kernel/describe", json!({})).await;
    // plan 18 K4-1: `describe` answers with the whole spec. A client needs no other document.
    let spec = &described["result"];
    assert_eq!(spec["protocol_version"], json!(1), "{described}");
    let named = |list: &str, name: &str| {
        spec[list]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["name"] == name))
    };
    assert!(named("methods", "akra/kernel/describe"));
    assert!(named("commands", "submit_prompt"));
    assert!(named("queries", "history"));
    assert!(named("events", "recorded"));
    assert!(named("records", "command_received"));
    assert!(named("reasons", "busy"));
    assert!(named("errors", "idempotency_key_reused"));
    assert!(spec["$defs"]["CommandResponse"].is_object());

    let detached = kernel_request(&mut client, "detach", "akra/kernel/detach", json!({})).await;
    assert_eq!(detached["result"], json!({}));
    let refused = kernel_request(
        &mut client,
        "late",
        "akra/kernel/command",
        json!({"command": {"type": "pause_auto_follow"}}),
    )
    .await;
    assert_eq!(refused["error"]["code"], json!(-32011), "{refused}");

    drop(client);
    drop(kernels);
    let _ = std::fs::remove_dir_all(&folder);
}

/// 커널 시험 폴더이다. 끝나면 지운다.
struct KernelTestFolder {
    path: std::path::PathBuf,
}

impl KernelTestFolder {
    fn new(label: &str) -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-gateway-kernel-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn workspace(&self) -> String {
        self.path.display().to_string()
    }
}

impl Drop for KernelTestFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

async fn attach_kernel(
    client: &mut WebSocketStream<DuplexStream>,
    id: &str,
    workspace: &str,
) -> Value {
    let attached = kernel_request(
        client,
        id,
        "akra/kernel/attach",
        json!({"workspace": workspace, "client_name": "gateway-test"}),
    )
    .await;
    assert!(attached["result"]["kernel"].is_string(), "{attached}");
    attached
}

fn set_max_turns_command(id: &str, value: u32) -> Message {
    Message::text(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "akra/kernel/command",
            "params": {"command": {"type": "set_auto_follow_max_turns", "value": value}},
        })
        .to_string(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn transient_detach_and_socket_close_wake_idle_unload() {
    for close_socket in [false, true] {
        let folder = KernelTestFolder::new(if close_socket {
            "transient-close"
        } else {
            "transient-detach"
        });
        let kernels =
            Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
        let idle_notification = super::register_idle_notification(&kernels);
        let mut client = connect_kernel_client(kernels.clone()).await;
        let attached = kernel_request(
            &mut client,
            "attach",
            "akra/kernel/attach",
            json!({"workspace": folder.workspace(), "client_name": "command-test", "transient": true}),
        ).await;
        let key = attached["result"]["kernel"]
            .as_str()
            .expect("attached")
            .to_string();
        let idle = idle_notification.0.notified();
        tokio::pin!(idle);
        idle.as_mut().enable();
        if close_socket {
            client.close(None).await.expect("close");
        } else {
            let detached =
                kernel_request(&mut client, "detach", "akra/kernel/detach", json!({})).await;
            assert_eq!(detached["result"], json!({}));
        }
        tokio::time::timeout(Duration::from_secs(10), idle)
            .await
            .expect("disconnect wakes the idle reaper");
        assert_eq!(
            kernels.unload_idle_kernels(std::time::Instant::now()),
            vec![key]
        );
        assert!(
            crate::composition::production::acquire_repository_kernel_lock(&folder.workspace())
                .is_ok()
        );
    }
}

/// 조용한 시간이 올 때까지 받은 텍스트 메시지를 모은다.
async fn texts_until_quiet(
    websocket: &mut WebSocketStream<DuplexStream>,
    quiet: Duration,
) -> Vec<Value> {
    let mut texts = Vec::new();
    while let Ok(Some(Ok(message))) = tokio::time::timeout(quiet, websocket.next()).await {
        if let Message::Text(text) = message {
            texts.push(serde_json::from_str(text.as_str()).unwrap());
        }
    }
    texts
}

const QUIET_WINDOW: Duration = Duration::from_millis(700);

#[tokio::test(flavor = "multi_thread")]
async fn a_client_close_ends_the_session_without_waiting_for_the_flush_timeout() {
    // K3-5 review: the kernel session kept an outbox sender, so the writer waited 2 s on every close.
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let session = tokio::spawn(run_session(
        gateway_side,
        GatewayHosts::new(EchoUpstream::new(true), Vec::new()),
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
        super::session::GatewayServices::default(),
        None,
    ));
    let (mut client, _) = tokio_tungstenite::client_async_with_config(
        "ws://localhost/",
        client_side,
        Some(websocket_config()),
    )
    .await
    .unwrap();

    client.close(None).await.unwrap();

    tokio::time::timeout(Duration::from_secs(1), session)
        .await
        .expect("the session should end before the flush timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_kernel_attach_does_not_block_app_server_forwarding() {
    // K3-5 review: the read loop awaited the kernel call, so app-server messages stopped behind it.
    let folder = KernelTestFolder::new("waiting-attach");
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let release = crate::composition::repository_kernel::tests::hold_repository_for_test(
        &kernels,
        &folder.workspace(),
    );
    let mut client = connect_kernel_client(kernels.clone()).await;

    client
        .send(Message::text(
            json!({
                "jsonrpc": "2.0",
                "id": "attach",
                "method": "akra/kernel/attach",
                "params": {"workspace": folder.workspace(), "client_name": "gateway-test"},
            })
            .to_string(),
        ))
        .await
        .unwrap();
    client
        .send(Message::text(r#"{"id":1,"method":"thread/list"}"#))
        .await
        .unwrap();
    let forwarded = tokio::time::timeout(Duration::from_secs(2), next_text(&mut client))
        .await
        .expect("app-server forwarding must not wait for the kernel attach");
    assert_eq!(forwarded, r#"echo:{"id":1,"method":"thread/list"}"#);

    drop(release);
    let attached = next_json_matching(&mut client, |value| value["id"] == json!("attach")).await;
    assert!(attached["result"]["kernel"].is_string(), "{attached}");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_kernel_event_reaches_a_client_after_its_detach_response() {
    let folder = KernelTestFolder::new("detach-events");
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut watcher = connect_kernel_client(kernels.clone()).await;
    let mut driver = connect_kernel_client(kernels.clone()).await;
    attach_kernel(&mut watcher, "attach", &folder.workspace()).await;
    attach_kernel(&mut driver, "attach", &folder.workspace()).await;

    let detached = kernel_request(&mut watcher, "detach", "akra/kernel/detach", json!({})).await;
    assert_eq!(detached["result"], json!({}));
    driver
        .send(set_max_turns_command("command", 3))
        .await
        .unwrap();
    next_json_matching(&mut driver, |value| value["id"] == json!("command")).await;

    let late = texts_until_quiet(&mut watcher, QUIET_WINDOW).await;
    assert!(
        late.is_empty(),
        "events after the detach response: {late:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attaching_again_does_not_duplicate_kernel_events() {
    let folder = KernelTestFolder::new("reattach-events");
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;
    attach_kernel(&mut client, "first", &folder.workspace()).await;
    attach_kernel(&mut client, "second", &folder.workspace()).await;

    client
        .send(set_max_turns_command("command", 4))
        .await
        .unwrap();

    let received = texts_until_quiet(&mut client, QUIET_WINDOW).await;
    let events = received
        .iter()
        .filter(|value| value["params"]["event"]["type"] == json!("auto_follow"))
        .count();
    assert_eq!(events, 1, "{received:?}");
}

/// upstream 이 멈출 때 그 저장소의 커널 잠금이 풀려 있었는지 적는다.
struct LockProbeUpstream {
    inner: Arc<EchoUpstream>,
    workspace: String,
    kernel_lock_free_at_shutdown: Mutex<Option<bool>>,
}

impl GatewayUpstreamPort for LockProbeUpstream {
    fn open_session(&self) -> Result<Box<dyn GatewayUpstreamStream>> {
        self.inner.open_session()
    }

    fn snapshot(&self) -> GatewayUpstreamSnapshot {
        self.inner.snapshot()
    }

    fn generation(&self) -> watch::Receiver<u64> {
        self.inner.generation()
    }

    fn shutdown(&self) {
        let free =
            crate::composition::production::acquire_repository_kernel_lock(&self.workspace).is_ok();
        *self.kernel_lock_free_at_shutdown.lock().unwrap() = Some(free);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_stop_stops_kernels_before_the_upstream_app_server() {
    // K3-5 review: a connected session kept the registry alive, so the upstream stopped first.
    let folder = KernelTestFolder::new("stop-order");
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;
    attach_kernel(&mut client, "attach", &folder.workspace()).await;
    let upstream = Arc::new(LockProbeUpstream {
        inner: EchoUpstream::new(true),
        workspace: folder.workspace(),
        kernel_lock_free_at_shutdown: Mutex::new(None),
    });

    super::stop_backend(
        GatewayHosts::new(upstream.clone(), Vec::new()),
        kernels,
        empty_mcp_hub(),
    )
    .await;

    assert_eq!(
        *upstream.kernel_lock_free_at_shutdown.lock().unwrap(),
        Some(true),
        "the kernel stops and releases its lock before the upstream stops"
    );
}

/// MCP hub 가 필요 없는 테스트용이다. 등록된 서버가 없고 엔진도 없다.
fn empty_mcp_hub() -> Arc<super::GatewayMcpHub> {
    Arc::new(empty_hub())
}

fn empty_hub() -> super::GatewayMcpHub {
    struct EmptyHub;
    impl crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort for EmptyHub {
        fn registered_servers(
            &self,
        ) -> anyhow::Result<Vec<crate::domain::mcp_hub::McpHubServerSpec>> {
            Ok(Vec::new())
        }
        fn register_server(
            &self,
            _spec: &crate::domain::mcp_hub::McpHubServerSpec,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn unregister_server(&self, _name: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        fn server_statuses(
            &self,
            _only: Option<&str>,
        ) -> anyhow::Result<Vec<crate::domain::mcp_hub::McpHubServerStatus>> {
            Ok(Vec::new())
        }
        fn call_tool(
            &self,
            _server: &str,
            _tool: &str,
            _arguments: serde_json::Value,
        ) -> anyhow::Result<serde_json::Value> {
            anyhow::bail!("no MCP hub in this test")
        }
        fn login(&self, _server: &str, _on_url: &mut dyn FnMut(&str)) -> anyhow::Result<()> {
            Ok(())
        }
    }
    super::GatewayMcpHub::new(String::new(), Arc::new(EmptyHub))
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_connection_that_declares_itself_receives_mcp_write_approvals() {
    // #2830 review: the `akra mcp` connection that calls a write tool must not count as an approver.
    use crate::application::port::outbound::mcp_write_approval_port::{
        McpWriteApproval, McpWriteApprovalPort, McpWriteApprovalRequest,
    };
    let broker = Arc::new(
        crate::application::service::mcp_write_approval::McpWriteApprovalBroker::new(
            Duration::from_secs(5),
        ),
    );
    let services = super::session::GatewayServices {
        mcp_hub: Some(Arc::new(empty_hub().with_write_approvals(broker.clone()))),
        ..Default::default()
    };
    let ask =
        |broker: Arc<crate::application::service::mcp_write_approval::McpWriteApprovalBroker>| {
            tokio::task::spawn_blocking(move || {
                broker.request_approval(&McpWriteApprovalRequest {
                    server: "notion".into(),
                    tool: "update".into(),
                    arguments: json!({}),
                })
            })
        };

    let mut cli = connect_with_services("/", services.clone()).await;
    let statuses = kernel_request(&mut cli, "s", "akra/mcp/statuses", json!({})).await;
    assert!(statuses["result"].is_object(), "the CLI session is up");
    assert_eq!(
        ask(broker.clone()).await.unwrap(),
        McpWriteApproval::NoApprover
    );

    let mut remote = connect_with_services("/host/cloud", services.clone()).await;
    let refused = kernel_request(
        &mut remote,
        "r",
        "akra/mcp/writeApproval/subscribe",
        json!({}),
    )
    .await;
    assert_eq!(refused["result"]["subscribed"], json!(false));

    let mut desktop = connect_with_services("/", services).await;
    let subscribed = kernel_request(
        &mut desktop,
        "d",
        "akra/mcp/writeApproval/subscribe",
        json!({}),
    )
    .await;
    assert_eq!(subscribed["result"]["subscribed"], json!(true));
    let pending = ask(broker.clone());
    let requested = next_json_matching(&mut desktop, |value| {
        value["method"] == json!("akra/mcp/writeApproval/requested")
    })
    .await;
    let answer = kernel_request(
        &mut desktop,
        "a",
        "akra/mcp/writeApproval/answer",
        json!({ "id": requested["params"]["id"], "approve": true }),
    )
    .await;
    assert_eq!(answer["result"]["accepted"], json!(true));
    assert_eq!(pending.await.unwrap(), McpWriteApproval::Approved);
}

/// plan 25 S1a: `local` 과 가짜 두 번째 호스트 `cloud` 가 있는 Gateway 에 `path` 로 붙는다.
async fn connect_to_host(
    path: &str,
    local: Arc<EchoUpstream>,
    cloud: Arc<EchoUpstream>,
    recorder: Option<Arc<super::recorder::GatewayRecorder>>,
) -> WebSocketStream<DuplexStream> {
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let hosts = GatewayHosts::new(local, vec![("cloud".to_string(), cloud as _)]);
    tokio::spawn(async move {
        let _ = run_session(
            gateway_side,
            hosts,
            Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
            super::session::GatewayServices::default(),
            recorder,
        )
        .await;
    });
    let (websocket, _) = tokio_tungstenite::client_async_with_config(
        format!("ws://localhost{path}"),
        client_side,
        Some(websocket_config()),
    )
    .await
    .expect("client handshake through the gateway session should succeed");
    websocket
}

#[tokio::test]
async fn the_request_path_selects_the_host_and_an_unknown_host_is_closed() {
    let local = EchoUpstream::new(true);
    let cloud = EchoUpstream::new(true);

    let mut remote = connect_to_host("/host/cloud", local.clone(), cloud.clone(), None).await;
    remote.send(Message::text(r#"{"id":1}"#)).await.unwrap();
    assert_eq!(next_text(&mut remote).await, r#"echo:{"id":1}"#);
    assert_eq!(*cloud.opened.lock().unwrap(), 1);
    assert_eq!(*local.opened.lock().unwrap(), 0);

    let info = kernel_request(&mut remote, "info", "akra/gateway/info", json!({})).await;
    assert_eq!(info["result"]["host"], json!("cloud"));
    let ids: Vec<&Value> = info["result"]["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|host| &host["id"])
        .collect();
    assert_eq!(ids, [&json!("local"), &json!("cloud")]);

    let mut unknown = connect_to_host("/host/nope", local.clone(), cloud.clone(), None).await;
    assert_eq!(next_close_code(&mut unknown).await, Some(CloseCode::Policy));
    assert_eq!(*local.opened.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_remote_host_refuses_kernel_methods_and_answers_context_without_the_ledger() {
    // plan 25 §3.1 and Q-C3. No work context service is set, so an answer proves the ledger was not used.
    let mut remote = connect_to_host(
        "/host/cloud",
        EchoUpstream::new(true),
        EchoUpstream::new(true),
        None,
    )
    .await;

    let refused = kernel_request(&mut remote, "k", "akra/kernel/describe", json!({})).await;
    assert_eq!(refused["error"]["code"], json!(-32601));

    let context = kernel_request(
        &mut remote,
        "c",
        "akra/context/thread",
        json!({ "threadId": "remote-thread" }),
    )
    .await;
    assert_eq!(context["result"]["resources"], json!([]));
}

#[tokio::test]
async fn a_remote_host_session_is_not_recorded() {
    let directory = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-gateway-host-recording-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let recorder = Arc::new(super::recorder::GatewayRecorder::create(&directory).unwrap());
    let sessions = || std::fs::read_dir(&directory).unwrap().count();
    let before = sessions();

    let mut remote = connect_to_host(
        "/host/cloud",
        EchoUpstream::new(true),
        EchoUpstream::new(true),
        Some(recorder.clone()),
    )
    .await;
    remote.send(Message::text(r#"{"id":1}"#)).await.unwrap();
    next_text(&mut remote).await;
    assert_eq!(
        sessions(),
        before,
        "a remote session must not open a recording"
    );

    let mut local = connect_to_host(
        "/",
        EchoUpstream::new(true),
        EchoUpstream::new(true),
        Some(recorder),
    )
    .await;
    local.send(Message::text(r#"{"id":1}"#)).await.unwrap();
    next_text(&mut local).await;
    assert!(sessions() > before, "the local session is still recorded");
    let _ = std::fs::remove_dir_all(&directory);
}

/// 폴더 이름을 `head` 로 돌려주는 가짜 그래프이다.
struct FolderEchoGraph;

impl crate::application::port::inbound::git_graph_port::GitGraphPort for FolderEchoGraph {
    fn graph_page(
        &self,
        request: crate::application::port::inbound::git_graph_port::GitGraphPageRequest,
    ) -> anyhow::Result<crate::application::port::inbound::git_graph_port::GitGraphPage> {
        Ok(
            crate::application::port::inbound::git_graph_port::GitGraphPage {
                repository: true,
                stale: false,
                head: Some(request.repository_dir),
                refs_digest: None,
                refs: Vec::new(),
                worktrees: Vec::new(),
                rows: Vec::new(),
                next: None,
            },
        )
    }

    fn refs_state(
        &self,
        _repository_dir: &str,
    ) -> anyhow::Result<crate::application::port::inbound::git_graph_port::GitGraphRefsState> {
        Ok(
            crate::application::port::inbound::git_graph_port::GitGraphRefsState {
                repository: true,
                refs_digest: None,
            },
        )
    }

    fn commit_detail(
        &self,
        _repository_dir: &str,
        _commit: &str,
    ) -> anyhow::Result<Option<crate::domain::git_graph::GitCommitDetail>> {
        Ok(None)
    }
}

async fn connect_with_services(
    path: &str,
    services: super::session::GatewayServices,
) -> WebSocketStream<DuplexStream> {
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let hosts = GatewayHosts::new(
        EchoUpstream::new(true),
        vec![("cloud".to_string(), EchoUpstream::new(true) as _)],
    );
    tokio::spawn(async move {
        let _ = run_session(
            gateway_side,
            hosts,
            Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
            services,
            None,
        )
        .await;
    });
    let (websocket, _) = tokio_tungstenite::client_async_with_config(
        format!("ws://localhost{path}"),
        client_side,
        Some(websocket_config()),
    )
    .await
    .expect("client handshake through the gateway session should succeed");
    websocket
}

#[tokio::test]
async fn the_local_host_answers_the_git_graph_and_a_remote_host_does_not() {
    // plan 27 G1. The graph reads repositories of this machine, so a remote host has no such method.
    let services = super::session::GatewayServices {
        git_graph: Some(Arc::new(FolderEchoGraph)),
        ..Default::default()
    };
    let mut local = connect_with_services("/", services.clone()).await;
    let page = kernel_request(&mut local, "g", "akra/git/graph", json!({ "cwd": "/repo" })).await;
    assert_eq!(page["result"]["repository"], json!(true));
    assert_eq!(page["result"]["head"], json!("/repo"));

    let mut remote = connect_with_services("/host/cloud", services).await;
    let refused = kernel_request(
        &mut remote,
        "g",
        "akra/git/graph",
        json!({ "cwd": "/repo" }),
    )
    .await;
    assert_eq!(refused["error"]["code"], json!(-32601));
}

/// 원격 이슈에 합성 비밀값이 들어온 경우이다. 시험용 값이고 실제 비밀이 아니다.
struct EditedIssueSnapshot;

const SYNTHETIC_ISSUE_TOKEN: &str = "ghp_SyntheticCanary0000";

impl crate::application::port::outbound::issue_snapshot_port::IssueSnapshotPort
    for EditedIssueSnapshot
{
    fn resolve(
        &self,
        request: crate::application::port::outbound::issue_snapshot_port::IssueSnapshotResolveRequest,
    ) -> anyhow::Result<
        crate::application::port::outbound::issue_snapshot_port::IssueSnapshotResponse,
    > {
        use crate::application::port::outbound::issue_snapshot_port::{
            IssueRawFields, IssueSnapshotResponse,
        };
        use crate::domain::issue_work::{IssueComparisonBasis, IssueProvider, IssueReference};
        let raw_fields = IssueRawFields::new(
            format!("Crash #{}", request.locator.number),
            format!("token={SYNTHETIC_ISSUE_TOKEN} 로 실패"),
        );
        Ok(IssueSnapshotResponse {
            issue: IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_1".to_string(),
                issue_id: "I_1".to_string(),
                project_scope: request.project_scope,
            },
            comparison_basis: IssueComparisonBasis {
                field_hash: raw_fields.field_hash(),
                provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
            },
            raw_fields,
        })
    }

    fn read(
        &self,
        _request: crate::application::port::outbound::issue_snapshot_port::IssueSnapshotReadRequest,
    ) -> anyhow::Result<
        crate::application::port::outbound::issue_snapshot_port::IssueSnapshotResponse,
    > {
        anyhow::bail!("not used")
    }
}

struct NoIssuePolicyFiles;

impl crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort
    for NoIssuePolicyFiles
{
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }

    fn load_project_policy_toml(&self, _workspace_dir: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

#[tokio::test]
async fn the_issue_handoff_answer_is_the_shared_copy_without_the_secret() {
    // #2428. The remote text has a synthetic secret from an external edit. The answer must not have it.
    let services = super::session::GatewayServices {
        issue_handoff: Some(Arc::new(
            crate::application::service::issue_handoff::IssueHandoffService::new(
                Arc::new(EditedIssueSnapshot),
                Arc::new(NoIssuePolicyFiles),
            ),
        )),
        ..Default::default()
    };
    let params = json!({ "owner": "sample", "repository": "widgets", "number": 42 });
    let mut local = connect_with_services("/", services.clone()).await;
    let answer = kernel_request(&mut local, "i", "akra/issue/handoffCopy", params.clone()).await;
    assert_eq!(
        answer["result"],
        json!({ "sharedCopy": { "title": "Crash #42", "body": "token=[removed] 로 실패" } })
    );
    assert!(!answer.to_string().contains(SYNTHETIC_ISSUE_TOKEN));

    let invalid = kernel_request(&mut local, "j", "akra/issue/handoffCopy", json!({})).await;
    assert_eq!(invalid["error"]["code"], json!(-32602));

    let mut remote = connect_with_services("/host/cloud", services).await;
    let refused = kernel_request(&mut remote, "i", "akra/issue/handoffCopy", params).await;
    assert_eq!(refused["error"]["code"], json!(-32601));
}

/// 미리보기의 답에 로컬 글을 그대로 넣는 가짜이다. 녹화가 요청과 답을 둘 다 빼는지 본다.
struct EchoIssueDrafts;

impl crate::application::port::inbound::issue_draft_port::IssueDraftPort for EchoIssueDrafts {
    fn creation_open(
        &self,
        request: creation::IssueCreationTarget,
    ) -> Result<creation::IssueCreationOpened> {
        bail!("echo {}", request.owner)
    }

    fn creation_preview(
        &self,
        request: creation::IssueCreationPreviewRequest,
    ) -> Result<creation::IssueCreationPreview> {
        bail!("echo {}", request.text.title)
    }

    fn creation_publish(
        &self,
        request: creation::IssueCreationPublishRequest,
    ) -> Result<creation::IssueCreationPublished> {
        bail!("echo {}", request.text.title)
    }

    fn creation_recheck(
        &self,
        request: creation::IssueCreationIntentRequest,
    ) -> Result<creation::IssueCreationIntent> {
        bail!("echo {}", request.intent_id)
    }

    fn creation_confirm(
        &self,
        request: creation::IssueCreationConfirmRequest,
    ) -> Result<creation::IssueCreationIntent> {
        bail!("echo {}", request.url)
    }

    fn creation_restart(
        &self,
        request: creation::IssueCreationRestartRequest,
    ) -> Result<creation::IssueCreationOpened> {
        bail!("echo {}", request.previous_intent_id)
    }

    fn open(
        &self,
        _: crate::application::port::inbound::issue_draft_port::IssueDraftTarget,
    ) -> anyhow::Result<crate::application::port::inbound::issue_draft_port::IssueDraftOpened> {
        anyhow::bail!("not used")
    }

    fn preview(
        &self,
        request: crate::application::port::inbound::issue_draft_port::IssueDraftPreviewRequest,
    ) -> anyhow::Result<crate::application::port::inbound::issue_draft_port::IssueDraftPreview>
    {
        anyhow::bail!("echo {}", request.text.title)
    }

    fn publish(
        &self,
        _: crate::application::port::inbound::issue_draft_port::IssueDraftPublishRequest,
    ) -> anyhow::Result<crate::application::port::inbound::issue_draft_port::IssueDraftPublished>
    {
        anyhow::bail!("not used")
    }

    fn recheck(
        &self,
        _: crate::application::port::inbound::issue_draft_port::IssueDraftTarget,
    ) -> anyhow::Result<Option<crate::application::port::inbound::issue_draft_port::IssueDraftIntent>>
    {
        anyhow::bail!("not used")
    }

    fn abandon(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<crate::application::port::inbound::issue_draft_port::IssueDraftIntent> {
        anyhow::bail!("not used")
    }
}

#[tokio::test]
async fn issue_draft_requests_are_answered_on_the_local_host_and_stay_out_of_the_recording() {
    // #2426. The request carries the local text before masking, so neither it nor its answer is recorded.
    const LOCAL_TEXT: &str = "synthetic-secret-0000";
    let directory = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-gateway-issue-recording-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let recorder = Arc::new(super::recorder::GatewayRecorder::create(&directory).unwrap());
    let connect = |path: &'static str| {
        let recorder = recorder.clone();
        async move {
            let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
            let hosts = GatewayHosts::new(
                EchoUpstream::new(true),
                vec![("cloud".to_string(), EchoUpstream::new(true) as _)],
            );
            let services = super::session::GatewayServices {
                issue_drafts: Some(Arc::new(EchoIssueDrafts)),
                ..Default::default()
            };
            tokio::spawn(async move {
                let _ = run_session(
                    gateway_side,
                    hosts,
                    Arc::new(
                        crate::composition::repository_kernel::RepositoryKernelRegistry::default(),
                    ),
                    services,
                    Some(recorder),
                )
                .await;
            });
            let (websocket, _) = tokio_tungstenite::client_async_with_config(
                format!("ws://localhost{path}"),
                client_side,
                Some(websocket_config()),
            )
            .await
            .expect("client handshake through the gateway session should succeed");
            websocket
        }
    };
    let params = json!({
        "target": { "cwd": "/repo", "owner": "sample", "repository": "widgets", "number": 7 },
        "text": { "title": LOCAL_TEXT, "body": "" },
    });

    let mut local = connect("/").await;
    let answer = kernel_request(&mut local, "i", "akra/issue/preview", params.clone()).await;
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains(LOCAL_TEXT),
        "the fake port answered: {answer}"
    );

    // #2436. Every creation method carries local data and uses the same private local-only boundary.
    let creation_target = json!({ "cwd": "/repo", "owner": LOCAL_TEXT, "repository": "widgets" });
    let creation_text = json!({ "title": LOCAL_TEXT, "body": "", "spans": [] });
    let creation_approval = json!({
        "repository": { "provider": "github", "serverId": "github.com", "repositoryId": "R_1",
            "projectScope": { "workspaceDir": "/repo" } },
        "intentId": LOCAL_TEXT, "sharedCopyHash": { "version": 1, "sha256": "aa" },
        "policyVersion": "p1", "templateVersion": null,
    });
    let creation_requests = [
        ("akra/issue/create/open", creation_target.clone()),
        (
            "akra/issue/create/preview",
            json!({ "target": creation_target, "intentId": LOCAL_TEXT, "text": creation_text }),
        ),
        (
            "akra/issue/create/publish",
            json!({ "target": creation_target, "intentId": LOCAL_TEXT, "text": creation_text, "approval": creation_approval }),
        ),
        (
            "akra/issue/create/recheck",
            json!({ "target": creation_target, "intentId": LOCAL_TEXT }),
        ),
        (
            "akra/issue/create/confirm",
            json!({ "target": creation_target, "intentId": LOCAL_TEXT, "url": LOCAL_TEXT }),
        ),
        (
            "akra/issue/create/restart",
            json!({ "target": creation_target, "previousIntentId": LOCAL_TEXT, "acknowledgeUncertain": true }),
        ),
    ];
    for (method, params) in &creation_requests {
        let answer = kernel_request(&mut local, method, method, params.clone()).await;
        assert_eq!(
            answer["error"]["code"],
            json!(-32000),
            "local route: {method}"
        );
        assert!(
            answer["error"]["message"]
                .as_str()
                .unwrap()
                .contains(LOCAL_TEXT),
            "the creation fake answered {method}: {answer}"
        );
    }
    // A recorded request after the issue request proves the recording is on and has caught up.
    kernel_request(&mut local, "info", "akra/gateway/info", json!({})).await;
    let recorded: String = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    assert!(recorded.contains("akra/gateway/info"));
    assert!(!recorded.contains(LOCAL_TEXT));

    let mut remote = connect("/host/cloud").await;
    let refused = kernel_request(&mut remote, "i", "akra/issue/preview", params).await;
    assert_eq!(refused["error"]["code"], json!(-32601));
    for (method, params) in creation_requests {
        let refused = kernel_request(&mut remote, method, method, params).await;
        assert_eq!(
            refused["error"]["code"],
            json!(-32601),
            "remote route: {method}"
        );
        assert!(!refused.to_string().contains(LOCAL_TEXT));
    }
    kernel_request(&mut remote, "remote-info", "akra/gateway/info", json!({})).await;
    let recorded: String = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    assert!(
        !recorded.contains(LOCAL_TEXT),
        "remote creation requests are private too"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

mod kernel_extensions;
