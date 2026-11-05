// Gateway upstream port 는 akra-gateway 가 클라이언트 세션마다 codex app-server 로 가는 연결을 여는
// 능력이다. app-server 를 어떻게 실행하고 어떤 통로(proxy 자식 프로세스)로 붙는지는 outbound adapter 가
// 소유한다. Gateway 는 이 연결 위에서 WebSocket 을 말하고 메시지를 그대로 중계한다 (D-11, P-03, P-09).
use anyhow::Result;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

/// upstream 과 이어진 양방향 바이트 스트림이다. drop 하면 연결과 그 자식 프로세스가 함께 정리된다.
pub trait GatewayUpstreamStream: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T> GatewayUpstreamStream for T where T: AsyncRead + AsyncWrite + Send + Unpin {}

/// `akra/gateway/info` 에 쓰는 upstream 상태이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayUpstreamSnapshot {
    /// upstream 으로 실행한 codex 의 버전 문자열이다. 확인하지 못했으면 `None` 이다.
    pub codex_version: Option<String>,
    /// upstream supervisor 가 시작한 시각 (RFC 3339) 이다.
    pub started_at: String,
    /// upstream app-server 를 다시 시작한 횟수이다.
    pub restarts: u64,
}

pub trait GatewayUpstreamPort: Send + Sync {
    /// 새 세션용 upstream 연결을 연다. upstream 이 준비되지 않았으면 오류를 돌려준다.
    fn open_session(&self) -> Result<Box<dyn GatewayUpstreamStream>>;

    /// 현재 upstream 상태이다.
    fn snapshot(&self) -> GatewayUpstreamSnapshot;

    /// upstream 세대 번호이다. app-server 가 다시 시작될 때마다 1 씩 늘어난다. 세션은 값이 바뀌면 닫는다.
    fn generation(&self) -> watch::Receiver<u64>;

    /// upstream app-server 와 남은 자식 프로세스를 정리한다.
    fn shutdown(&self);
}
