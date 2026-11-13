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
//! 범위(D1): 메시지는 양방향으로 바꾸지 않고 전달한다. 요청 변환(§2.4)·토큰 갱신·401 재연결은
//! 뒤 단계에서 더한다. 실계정 쓰기(turn/*)는 클라이언트가 보내지 않는 한 일어나지 않는다.

use std::path::PathBuf;
use std::sync::Once;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

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
    let inner =
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
            let mut inner = inner;
            let _ = inner.close(None).await;
            return;
        }
    };
    relay(inner, cloud).await;
}

/// 두 WebSocket 사이에서 Text·Binary·Close 를 그대로 나른다. ping/pong 은 각 소켓이 읽을 때 처리한다.
async fn relay<I, C>(
    inner: tokio_tungstenite::WebSocketStream<I>,
    cloud: tokio_tungstenite::WebSocketStream<C>,
) where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut inner_sink, mut inner_source) = inner.split();
    let (mut cloud_sink, mut cloud_source) = cloud.split();

    let client_to_cloud = async {
        while let Some(result) = inner_source.next().await {
            let message = match result {
                Ok(message) => message,
                Err(_) => break,
            };
            match message {
                Message::Text(_) | Message::Binary(_) => {
                    if cloud_sink.send(message).await.is_err() {
                        break;
                    }
                }
                Message::Close(frame) => {
                    let _ = cloud_sink.send(Message::Close(frame)).await;
                    break;
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    };
    let cloud_to_client = async {
        while let Some(result) = cloud_source.next().await {
            let message = match result {
                Ok(message) => message,
                Err(_) => break,
            };
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
    };

    tokio::select! {
        () = client_to_cloud => {}
        () = cloud_to_client => {}
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
}
