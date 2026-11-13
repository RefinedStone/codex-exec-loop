//! dots 클라우드 app-server upstream 이다 (D-19, D1). `/host/cloud` 접속을 OpenAI 클라우드
//! app-server WebSocket 로 잇는 다리이다.
//!
//! Gateway 는 upstream 바이트 스트림 위로 안쪽 WS client 핸드셰이크(`ws://localhost/rpc`)를 한다.
//! 이 다리는 그것을 server 로 받아, 실제 클라우드 WS(`wss://codex-cloud-backend.chatgpt.com/`)에
//! client 로 붙어 메시지를 그대로 중계한다. 즉 "안쪽 WS(gateway 용 server) ↔ 바깥 WS(cloud client)"
//! 이다. proxy 자식 프로세스가 없는 두 번째 호스트이다 (plan 25 G0, [[dots-integration]]).
//!
//! 인증은 로컬 `~/.codex/auth.json` 의 access_token 하나이다(공식 앱과 같은 토큰). 하위 프로토콜
//! `openai-bearer.<token>` 로 보낸다. 자격증명은 로그·스냅샷에 남기지 않는다.
//!
//! 범위(D1): 읽기 전용이다. 클라이언트의 요청은 [`READ_ONLY_METHODS`] 만 클라우드로 보내고 나머지는
//! 다리가 거절한다. 그래서 실계정 쓰기(turn/*, thread/resume 등)는 어떤 클라이언트가 붙어도 일어나지
//! 않는다 (D-19). 클라우드가 로그인을 거절하면(401/403) close 1008 로 닫아 클라이언트가 다시 시도하지
//! 않게 한다 (plan 25 §7). 요청 변환(§3)·토큰 갱신은 쓰기 단계(D2)에서 더한다.

use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};

use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamSnapshot, GatewayUpstreamStream,
};

/// 클라우드 app-server 의 고정 주소이다. relay 설정(`openai_base_url`)과 무관하다.
const CLOUD_URL: &str = "wss://codex-cloud-backend.chatgpt.com/";
/// 공식 데스크톱 앱의 신원이다. 서버가 검사할 수 있다.
const CLIENT_ORIGINATOR: &str = "Codex Desktop";
const PRODUCT_SKU: &str = "codex";
/// UA 에 쓰는 기본 클라이언트 버전이다. `AKRA_DOTS_USER_AGENT` 로 UA 전체를 바꿀 수 있다.
const DEFAULT_CLIENT_VERSION: &str = "26.1106.51428";
/// `thread/resume` 응답이 커서 세션과 같은 상한을 둔다.
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 256 << 20;
/// Gateway 세션과 다리 사이의 인메모리 버퍼이다.
const DUPLEX_BUFFER_BYTES: usize = 1 << 20;
/// 클라우드로 보내도 되는 읽기 method 이다. 여기 없는 요청은 다리가 거절한다. `thread/resume` 은 멈춘
/// 에이전트를 다시 깨울 수 있어 쓰기로 본다. 쓰기는 사용자 허락을 받는 단계(D2)에서 연다 (D-19).
const READ_ONLY_METHODS: &[&str] = &[
    "initialize",
    "initialized",
    "thread/list",
    "thread/read",
    "thread/turns/list",
    "thread/items/list",
    "model/list",
];
/// 거절 응답의 JSON-RPC 코드이다. 원격 접속의 `akra/kernel/*` 거절과 같다 (method not found).
const READ_ONLY_REFUSAL_CODE: i64 = -32601;
/// 클라우드가 로그인을 거절했을 때의 close 이유이다. Desktop 이 이 글자로 안내문을 고른다.
const LOGIN_REJECTED_REASON: &str = "cloud login rejected";
/// 공식 앱과 같은 keepalive 이다: 10초마다 ping, 60초 동안 클라우드가 조용하면 끊는다.
const KEEPALIVE: Keepalive = Keepalive {
    ping_every: Duration::from_secs(10),
    silence_limit: Duration::from_secs(60),
};

#[derive(Clone, Copy)]
struct Keepalive {
    ping_every: Duration,
    silence_limit: Duration,
}

/// rustls 0.23 은 암호화 provider 를 자동으로 고르지 않는다. 첫 클라우드 접속 전에 ring 을 한 번 설치한다.
fn ensure_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
}

/// 클라우드 호스트 upstream 이다. proxy 프로세스가 없으므로 다시 시작도 없다(세대는 늘 0).
pub(crate) struct CloudGatewayUpstream {
    auth_path: PathBuf,
    user_agent: String,
    started_at: String,
    generation: watch::Sender<u64>,
}

impl CloudGatewayUpstream {
    /// `auth_path` 는 codex 의 `auth.json` 이다(보통 `~/.codex/auth.json`).
    pub(crate) fn new(auth_path: PathBuf) -> Self {
        Self {
            auth_path,
            user_agent: default_user_agent(),
            started_at: chrono::Utc::now().to_rfc3339(),
            generation: watch::channel(0).0,
        }
    }
}

impl GatewayUpstreamPort for CloudGatewayUpstream {
    fn open_session(&self) -> Result<Box<dyn GatewayUpstreamStream>> {
        ensure_crypto_provider();
        let credentials = load_credentials(&self.auth_path)?;
        let request = build_cloud_request(&credentials, &self.user_agent)?;
        let (gateway_side, bridge_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
        tokio::spawn(run_bridge(bridge_side, request));
        Ok(Box::new(gateway_side))
    }

    fn snapshot(&self) -> GatewayUpstreamSnapshot {
        GatewayUpstreamSnapshot {
            codex_version: None,
            started_at: self.started_at.clone(),
            restarts: 0,
        }
    }

    fn generation(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    fn shutdown(&self) {
        // 세션은 클라이언트가 닫거나 프로세스가 끝나면 끝난다. 감시할 자식 프로세스가 없다.
    }
}

/// Gateway 의 안쪽 WS 를 server 로 받고, 클라우드 WS 에 client 로 붙어 양방향으로 중계한다.
async fn run_bridge(bridge_side: tokio::io::DuplexStream, request: Request) {
    let mut inner =
        match tokio_tungstenite::accept_async_with_config(bridge_side, Some(websocket_config()))
            .await
        {
            Ok(inner) => inner,
            Err(error) => {
                tracing::warn!(error = %error, "dots cloud bridge inner handshake failed");
                return;
            }
        };
    let cloud = match tokio_tungstenite::connect_async_with_config(
        request,
        Some(websocket_config()),
        false,
    )
    .await
    {
        Ok((cloud, _response)) => cloud,
        Err(error) => {
            // 토큰·신원은 로그에 넣지 않는다. 오류 자체(상태 코드 등)만 남긴다.
            tracing::warn!(error = %error, "dots cloud upstream connect failed");
            let _ = inner.close(connect_failure_close(&error)).await;
            return;
        }
    };
    relay(inner, cloud, KEEPALIVE).await;
}

/// 로그인 거절(401/403)은 1008 로 닫는다. 클라이언트가 같은 토큰으로 클라우드를 다시 두드리지 않는다.
/// 그 밖의 실패는 이유 없이 닫고, 클라이언트가 평소처럼 다시 붙는다.
fn connect_failure_close(error: &WsError) -> Option<CloseFrame> {
    let WsError::Http(response) = error else {
        return None;
    };
    matches!(response.status().as_u16(), 401 | 403).then(|| CloseFrame {
        code: CloseCode::Policy,
        reason: LOGIN_REJECTED_REASON.into(),
    })
}

/// 클라이언트가 보낸 글자 프레임을 어떻게 다룰지이다.
#[derive(Debug, PartialEq, Eq)]
enum ClientFrameRoute {
    /// 클라우드로 그대로 보낸다.
    Forward,
    /// 클라우드로 보내지 않고 이 JSON-RPC 오류로 답한다.
    Refuse(String),
    /// 답할 id 가 없어 버린다.
    Drop,
}

/// 읽기 요청과 오류 응답만 클라우드로 보낸다. `result` 응답은 클라우드의 요청(예: 승인)을 수락하는
/// 쓰기이므로 버린다.
fn route_client_frame(text: &str) -> ClientFrameRoute {
    let Ok(serde_json::Value::Object(message)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return ClientFrameRoute::Drop;
    };
    let Some(method) = message.get("method").and_then(|method| method.as_str()) else {
        return if message.contains_key("error") {
            ClientFrameRoute::Forward
        } else {
            ClientFrameRoute::Drop
        };
    };
    if READ_ONLY_METHODS.contains(&method) {
        return ClientFrameRoute::Forward;
    }
    match message.get("id") {
        Some(id) => ClientFrameRoute::Refuse(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": READ_ONLY_REFUSAL_CODE,
                    "message": format!("the cloud host is read-only: {method} is not sent"),
                },
            })
            .to_string(),
        ),
        None => ClientFrameRoute::Drop,
    }
}

/// 두 WebSocket 사이를 중계한다. 클라이언트 쪽은 [`route_client_frame`] 을 거치고, 클라우드 쪽은 그대로
/// 나른다. 클라우드에는 주기적으로 ping 을 보내고, 오래 조용하면 끊는다.
async fn relay<I, C>(
    inner: tokio_tungstenite::WebSocketStream<I>,
    cloud: tokio_tungstenite::WebSocketStream<C>,
    keepalive: Keepalive,
) where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut inner_sink, mut inner_source) = inner.split();
    let (mut cloud_sink, mut cloud_source) = cloud.split();
    let mut ping = tokio::time::interval(keepalive.ping_every);
    let mut cloud_heard_at = tokio::time::Instant::now();

    loop {
        tokio::select! {
            message = inner_source.next() => {
                let Some(Ok(message)) = message else { break };
                match message {
                    Message::Text(text) => match route_client_frame(text.as_str()) {
                        ClientFrameRoute::Forward => {
                            if cloud_sink.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        }
                        ClientFrameRoute::Refuse(response) => {
                            if inner_sink.send(Message::text(response)).await.is_err() {
                                break;
                            }
                        }
                        ClientFrameRoute::Drop => {}
                    },
                    Message::Close(frame) => {
                        let _ = cloud_sink.send(Message::Close(frame)).await;
                        break;
                    }
                    // 압축을 협상하지 않으므로 클라이언트의 binary 는 없다. 있으면 검사할 수 없어 버린다.
                    Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
            message = cloud_source.next() => {
                let Some(Ok(message)) = message else { break };
                cloud_heard_at = tokio::time::Instant::now();
                match message {
                    Message::Text(_) | Message::Binary(_) => {
                        if inner_sink.send(message).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(frame) => {
                        let _ = inner_sink.send(Message::Close(frame)).await;
                        break;
                    }
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
            _ = ping.tick() => {
                if cloud_heard_at.elapsed() > keepalive.silence_limit {
                    tracing::warn!("dots cloud upstream is silent; closing the session");
                    break;
                }
                if cloud_sink.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
        }
    }
}

/// 클라우드 접속에 쓰는 토큰과 계정 ID 이다. 둘 다 로그에 남기지 않는다.
struct CloudCredentials {
    access_token: String,
    account_id: String,
}

fn load_credentials(auth_path: &std::path::Path) -> Result<CloudCredentials> {
    let data = std::fs::read_to_string(auth_path)
        .with_context(|| format!("failed to read codex auth file `{}`", auth_path.display()))?;
    let document: serde_json::Value =
        serde_json::from_str(&data).context("codex auth file is not valid JSON")?;
    let tokens = document.get("tokens").unwrap_or(&serde_json::Value::Null);
    let access_token = tokens
        .get("access_token")
        .and_then(|value| value.as_str())
        .filter(|token| !token.is_empty())
        .context("codex auth file has no ChatGPT access token (log in with codex first)")?
        .to_string();
    let account_id = account_id_from_token(&access_token)
        .or_else(|| {
            tokens
                .get("account_id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .context("codex auth token has no ChatGPT account id")?;
    Ok(CloudCredentials {
        access_token,
        account_id,
    })
}

/// access_token(JWT) 의 claim 에서 ChatGPT 계정 ID 를 꺼낸다. 토큰 값 자체는 돌려주지 않는다.
fn account_id_from_token(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    let auth = claims.get("https://api.openai.com/auth");
    auth.and_then(|auth| auth.get("chatgpt_account_id"))
        .or_else(|| claims.get("chatgpt_account_id"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn build_cloud_request(credentials: &CloudCredentials, user_agent: &str) -> Result<Request> {
    let mut request = CLOUD_URL
        .into_client_request()
        .context("failed to build the cloud WebSocket request")?;
    let headers = request.headers_mut();
    let protocols = format!(
        "codex-app-server, codex-client.desktop, openai-bearer.{}",
        credentials.access_token
    );
    headers.insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&protocols)
            .map_err(|_| anyhow!("the access token is not a valid header value"))?,
    );
    headers.insert(
        "X-OpenAI-Product-Sku",
        HeaderValue::from_static(PRODUCT_SKU),
    );
    headers.insert(
        "ChatGPT-Account-Id",
        HeaderValue::from_str(&credentials.account_id)
            .map_err(|_| anyhow!("the account id is not a valid header value"))?,
    );
    headers.insert("originator", HeaderValue::from_static(CLIENT_ORIGINATOR));
    headers.insert(
        "User-Agent",
        HeaderValue::from_str(user_agent)
            .map_err(|_| anyhow!("the user agent is not a valid header value"))?,
    );
    Ok(request)
}

fn default_user_agent() -> String {
    if let Ok(agent) = std::env::var("AKRA_DOTS_USER_AGENT")
        && !agent.is_empty()
    {
        return agent;
    }
    let os = if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Mac OS"
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    format!("{CLIENT_ORIGINATOR}/{DEFAULT_CLIENT_VERSION} ({os}; {arch})")
}

/// `~/.codex/auth.json` 의 경로이다. `CODEX_HOME` 이 있으면 그 아래를 쓴다.
pub(crate) fn codex_auth_path() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|home| !home.is_empty()) {
        return Ok(PathBuf::from(home).join("auth.json"));
    }
    #[cfg(windows)]
    {
        if let Some(home) = std::env::var_os("USERPROFILE").filter(|home| !home.is_empty()) {
            return Ok(PathBuf::from(home).join(".codex").join("auth.json"));
        }
    }
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("auth.json"))
        .ok_or_else(|| anyhow!("CODEX_HOME is unset and the user home directory is unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실계정 읽기 확인이다 (D-13 조회만, 모델 턴 없음). CI 는 네트워크·토큰이 없으므로 `#[ignore]` 이다.
    /// 수동 실행: `cargo test --lib cloud_upstream::tests::live_cloud_reads -- --ignored --nocapture`.
    /// `~/.codex/auth.json` 의 토큰으로 클라우드 WS 에 붙어 initialize, thread/list, 첫 스레드의 thread/read 와
    /// thread/turns/list 를 보낸다. 토큰·스레드 내용은 찍지 않고 개수만 센다.
    #[tokio::test]
    #[ignore = "real OpenAI cloud account and network; run manually"]
    async fn live_cloud_reads() {
        ensure_crypto_provider();
        let credentials = load_credentials(&codex_auth_path().expect("auth path")).expect("creds");
        let request = build_cloud_request(&credentials, &default_user_agent()).expect("request");
        let (mut cloud, _response) =
            tokio_tungstenite::connect_async_with_config(request, Some(websocket_config()), false)
                .await
                .expect("the cloud handshake must succeed");

        let init = live_rpc(
            &mut cloud,
            1,
            "initialize",
            serde_json::json!({
                "clientInfo": { "name": "Codex Desktop", "title": "Codex Desktop", "version": DEFAULT_CLIENT_VERSION },
                "experimentalApi": true,
                "requestAttestation": true,
            }),
        )
        .await;
        assert!(init.get("result").is_some(), "initialize failed: {init}");
        cloud
            .send(Message::text(r#"{"jsonrpc":"2.0","method":"initialized"}"#))
            .await
            .expect("initialized");

        let list = live_rpc(
            &mut cloud,
            2,
            "thread/list",
            serde_json::json!({ "limit": 20 }),
        )
        .await;
        let threads = list["result"]["data"]
            .as_array()
            .expect("thread/list returns data");
        eprintln!("live: thread/list returned {} threads", threads.len());
        assert!(list.get("result").is_some(), "thread/list failed: {list}");

        if let Some(first) = threads.first() {
            let thread_id = first["id"].as_str().expect("thread id");
            let read = live_rpc(
                &mut cloud,
                3,
                "thread/read",
                serde_json::json!({ "threadId": thread_id, "includeTurns": false }),
            )
            .await;
            eprintln!(
                "live: thread/read {} -> result={}",
                thread_id,
                read.get("result").is_some()
            );
            let turns = live_rpc(
                &mut cloud,
                4,
                "thread/turns/list",
                serde_json::json!({ "threadId": thread_id, "limit": 5, "sortDirection": "desc", "itemsView": "full" }),
            )
            .await;
            eprintln!(
                "live: thread/turns/list -> result={}, error={}",
                turns.get("result").is_some(),
                turns
                    .get("error")
                    .map(|error| error.to_string())
                    .unwrap_or_default()
            );
            assert!(read.get("result").is_some(), "thread/read failed: {read}");
            assert!(
                turns.get("result").is_some(),
                "thread/turns/list failed: {turns}"
            );
        }
        let _ = cloud.close(None).await;
    }

    /// 요청 하나를 보내고 그 `id` 의 응답이 올 때까지 다른 프레임을 건너뛴다. 알림(`method` 있는 프레임)은 버린다.
    async fn live_rpc<S>(
        cloud: &mut tokio_tungstenite::WebSocketStream<S>,
        id: i64,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let request =
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        cloud
            .send(Message::text(request.to_string()))
            .await
            .expect("send request");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let frame = tokio::time::timeout_at(deadline, cloud.next())
                .await
                .expect("the cloud must answer within 30s")
                .expect("the stream stays open")
                .expect("a readable frame");
            if let Message::Text(text) = frame {
                let value: serde_json::Value = serde_json::from_str(text.as_str()).expect("json");
                if value.get("id").and_then(|value| value.as_i64()) == Some(id) {
                    return value;
                }
            }
        }
    }

    fn token_with_claims(claims: &serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap());
        format!("{header}.{payload}.")
    }

    /// 저장소 관례대로 고유 임시 경로를 만든다(외부 crate 없이). 테스트 끝에 상위 폴더를 지운다.
    struct TempAuthFile {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TempAuthFile {
        fn write(contents: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "akra-dots-cloud-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let directory = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&directory).expect("temp dir");
            let path = directory.join("auth.json");
            std::fs::write(&path, contents).expect("write auth file");
            Self { directory, path }
        }
    }

    impl Drop for TempAuthFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn the_account_id_comes_from_the_openai_auth_claim_first() {
        let token = token_with_claims(&serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-nested" },
            "chatgpt_account_id": "acc-top",
        }));
        assert_eq!(account_id_from_token(&token).as_deref(), Some("acc-nested"));
    }

    #[test]
    fn the_account_id_falls_back_to_the_top_level_claim() {
        let token = token_with_claims(&serde_json::json!({ "chatgpt_account_id": "acc-top" }));
        assert_eq!(account_id_from_token(&token).as_deref(), Some("acc-top"));
    }

    #[test]
    fn a_token_without_an_account_claim_has_no_account_id() {
        let token = token_with_claims(&serde_json::json!({ "sub": "user" }));
        assert_eq!(account_id_from_token(&token), None);
    }

    #[test]
    fn the_request_carries_the_desktop_identity_and_the_bearer_subprotocol() {
        let credentials = CloudCredentials {
            access_token: "header.payload.signature".to_string(),
            account_id: "acc-1".to_string(),
        };
        let request = build_cloud_request(&credentials, "Codex Desktop/1.2.3 (Mac OS; arm64)")
            .expect("request builds");
        let headers = request.headers();
        assert_eq!(
            headers.get("Sec-WebSocket-Protocol").unwrap(),
            "codex-app-server, codex-client.desktop, openai-bearer.header.payload.signature"
        );
        assert_eq!(headers.get("X-OpenAI-Product-Sku").unwrap(), "codex");
        assert_eq!(headers.get("ChatGPT-Account-Id").unwrap(), "acc-1");
        assert_eq!(headers.get("originator").unwrap(), "Codex Desktop");
        assert_eq!(
            headers.get("User-Agent").unwrap(),
            "Codex Desktop/1.2.3 (Mac OS; arm64)"
        );
    }

    #[test]
    fn loading_credentials_needs_an_access_token() {
        let auth = TempAuthFile::write(r#"{"tokens":{"account_id":"acc-1"}}"#);
        // CloudCredentials 는 토큰을 담으므로 Debug 를 붙이지 않는다. expect_err 대신 match 로 본다.
        let error = match load_credentials(&auth.path) {
            Err(error) => error,
            Ok(_) => panic!("missing token must be an error"),
        };
        assert!(format!("{error:#}").contains("access token"), "{error:#}");
    }

    #[test]
    fn loading_credentials_reads_the_token_and_the_account_id() {
        let token = token_with_claims(&serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-nested" }
        }));
        let contents =
            serde_json::to_string(&serde_json::json!({ "tokens": { "access_token": token } }))
                .unwrap();
        let auth = TempAuthFile::write(&contents);
        let credentials = load_credentials(&auth.path).expect("credentials load");
        assert_eq!(credentials.account_id, "acc-nested");
    }

    #[test]
    fn read_requests_and_error_responses_go_to_the_cloud() {
        for text in [
            r#"{"jsonrpc":"2.0","id":1,"method":"thread/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","method":"initialized"}"#,
            r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32601,"message":"no"}}"#,
        ] {
            assert_eq!(
                route_client_frame(text),
                ClientFrameRoute::Forward,
                "{text}"
            );
        }
    }

    #[test]
    fn a_write_request_is_refused_with_its_id() {
        for method in [
            "turn/start",
            "thread/resume",
            "thread/start",
            "getAuthStatus",
        ] {
            let text = format!(r#"{{"jsonrpc":"2.0","id":"a-1","method":"{method}"}}"#);
            let ClientFrameRoute::Refuse(response) = route_client_frame(&text) else {
                panic!("{method} must be refused");
            };
            let response: serde_json::Value = serde_json::from_str(&response).unwrap();
            assert_eq!(response["id"], "a-1");
            assert_eq!(response["error"]["code"], READ_ONLY_REFUSAL_CODE);
            assert!(
                response["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(method)
            );
        }
    }

    #[test]
    fn frames_that_cannot_be_refused_are_dropped() {
        for text in [
            // 쓰기 알림, 클라우드 요청을 수락하는 응답, JSON 이 아닌 글자.
            r#"{"jsonrpc":"2.0","method":"turn/interrupt"}"#,
            r#"{"jsonrpc":"2.0","id":3,"result":{"decision":"accept"}}"#,
            "not json",
        ] {
            assert_eq!(route_client_frame(text), ClientFrameRoute::Drop, "{text}");
        }
    }

    #[test]
    fn only_a_login_rejection_closes_with_policy() {
        use tokio_tungstenite::tungstenite::http::Response;
        let http = |status: u16| {
            WsError::Http(Box::new(
                Response::builder().status(status).body(None).unwrap(),
            ))
        };
        for status in [401, 403] {
            let frame = connect_failure_close(&http(status)).expect("login rejection");
            assert_eq!(frame.code, CloseCode::Policy);
            assert_eq!(frame.reason.as_str(), LOGIN_REJECTED_REASON);
        }
        assert!(connect_failure_close(&http(503)).is_none());
        assert!(connect_failure_close(&WsError::ConnectionClosed).is_none());
    }

    type DuplexSocket = tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>;

    /// 인메모리 WebSocket 한 쌍이다: (client, server).
    async fn socket_pair() -> (DuplexSocket, DuplexSocket) {
        let (client_io, server_io) = tokio::io::duplex(1 << 16);
        let (client, server) = tokio::join!(
            tokio_tungstenite::client_async("ws://localhost/", client_io),
            tokio_tungstenite::accept_async(server_io),
        );
        (
            client.expect("client handshake").0,
            server.expect("server handshake"),
        )
    }

    async fn next_json(socket: &mut DuplexSocket) -> serde_json::Value {
        loop {
            if let Message::Text(text) = socket.next().await.expect("frame").expect("read") {
                return serde_json::from_str(text.as_str()).expect("json");
            }
        }
    }

    #[tokio::test]
    async fn the_relay_keeps_writes_away_from_the_cloud() {
        let (mut client, inner) = socket_pair().await;
        let (cloud, mut fake_cloud) = socket_pair().await;
        let relay_task = tokio::spawn(relay(inner, cloud, KEEPALIVE));

        let write = r#"{"jsonrpc":"2.0","id":1,"method":"turn/start","params":{}}"#;
        let read = r#"{"jsonrpc":"2.0","id":2,"method":"thread/list","params":{}}"#;
        client.send(Message::text(write)).await.unwrap();
        client.send(Message::text(read)).await.unwrap();

        // 클라우드가 처음 받는 글자는 읽기 요청이다. 쓰기 요청은 오지 않았다.
        assert_eq!(next_json(&mut fake_cloud).await["method"], "thread/list");
        fake_cloud
            .send(Message::text(
                r#"{"jsonrpc":"2.0","id":2,"result":{"data":[]}}"#,
            ))
            .await
            .unwrap();

        let refusal = next_json(&mut client).await;
        assert_eq!(refusal["id"], 1);
        assert_eq!(refusal["error"]["code"], READ_ONLY_REFUSAL_CODE);
        assert_eq!(next_json(&mut client).await["id"], 2);

        client.close(None).await.unwrap();
        relay_task.await.unwrap();
    }

    #[tokio::test]
    async fn the_relay_ends_when_the_cloud_stays_silent() {
        let (_client, inner) = socket_pair().await;
        // 가짜 클라우드는 읽지 않으므로 pong 도 보내지 않는다.
        let (cloud, _fake_cloud) = socket_pair().await;
        let keepalive = Keepalive {
            ping_every: Duration::from_millis(10),
            silence_limit: Duration::from_millis(50),
        };
        tokio::time::timeout(Duration::from_secs(5), relay(inner, cloud, keepalive))
            .await
            .expect("the relay must end by itself");
    }
}
