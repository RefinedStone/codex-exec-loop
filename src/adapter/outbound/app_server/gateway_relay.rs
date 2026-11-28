//! TUI 의 app-server 연결이 Gateway 를 지나는 길이다 (plan 18 K6-1).
//!
//! TUI 는 Desktop 처럼 Gateway 의 app-server 중계로 대화를 연다. 그래서 같은 사용자의 클라이언트가 app-server
//! 하나를 함께 쓰고, Gateway 가 TUI 의 턴도 본다 (#2850 의 git 쓰기 거절). 커널 프로토콜에는 대화 스트림을
//! 넣지 않는다.
//!
//! 연결은 자식 프로세스 하나를 둔다: `akra __gateway-app-server-relay <endpoint>`. 자식은 stdio 와 Gateway
//! endpoint 사이에서 바이트만 나른다. Gateway endpoint 는 WebSocket 을 말하므로, 연결은 `codex app-server proxy`
//! 와 같은 WebSocket 전송(`proxy_transport`)을 쓴다. 자식이 끝나면 연결의 기존 재연결 규칙이 그대로 돈다.

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

/// `akra` 가 relay 자식으로 돌 때의 첫 인자이다. 사용자 명령이 아니다.
pub(crate) const GATEWAY_RELAY_COMMAND: &str = "__gateway-app-server-relay";

/// Gateway 를 찾는 시간의 상한이다. Gateway 가 없으면 바로 실패하므로 보통은 이보다 훨씬 짧다.
#[cfg(test)]
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECT_ATTEMPTS: u32 = 10;
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// 이 사용자의 Gateway 가 접속을 받으면 그 중계를 돌려준다. 없으면 TUI 는 지금처럼 자기 app-server 를 띄운다.
#[cfg(test)]
pub(crate) fn reachable_gateway_relay() -> Option<super::SharedAppServer> {
    let endpoint = reachable_gateway_endpoint()?;
    let relay_executable = std::env::current_exe().ok()?;
    Some(super::SharedAppServer::Relay {
        relay_executable,
        endpoint,
    })
}

#[cfg(test)]
fn reachable_gateway_endpoint() -> Option<String> {
    let endpoint = crate::gateway_endpoint::client_endpoint().ok()?;
    // 호출자가 async runtime 안에 있을 수 있으므로 자기 runtime 을 가진 스레드에서 본다.
    let probed = endpoint.clone();
    let reachable = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .is_ok_and(|runtime| runtime.block_on(endpoint_accepts(&probed)))
    })
    .join()
    .unwrap_or(false);
    reachable.then_some(endpoint)
}

#[cfg(test)]
async fn endpoint_accepts(endpoint: &str) -> bool {
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, connect_with_retry(endpoint)).await,
        Ok(Ok(_))
    )
}

/// Windows 의 Gateway 는 접속 하나를 받은 뒤에 다음 pipe 를 연다. 그 사이의 접속은 pipe busy 로 실패하므로 잠깐
/// 다시 해 본다. 다른 오류(Gateway 가 없음)는 바로 돌려준다. 그래서 Gateway 가 없을 때 TUI 시작이 늦지 않다.
pub(crate) async fn connect_with_retry(
    endpoint: &str,
) -> std::io::Result<impl AsyncRead + AsyncWrite + Unpin + use<>> {
    let mut attempts_left = CONNECT_ATTEMPTS;
    loop {
        match super::super::gateway_kernel_client::connect(endpoint).await {
            Ok(stream) => return Ok(stream),
            Err(error) if attempts_left > 1 && is_pipe_busy(&error) => {
                attempts_left -= 1;
                tokio::time::sleep(CONNECT_RETRY_INTERVAL).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_pipe_busy(error: &std::io::Error) -> bool {
    // ERROR_PIPE_BUSY. unix 소켓에는 이 상태가 없다.
    cfg!(windows) && error.raw_os_error() == Some(231)
}

/// relay 자식의 본체이다. 연결하지 못하면 이유를 stderr 에 적고 1 로 끝난다. 어느 한쪽이 닫히면 0 으로 끝난다.
pub(crate) fn run_relay(endpoint: &str) -> Result<i32> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to start the Gateway relay runtime")?;
    let result = runtime.block_on(async {
        let gateway = connect_with_retry(endpoint)
            .await
            .with_context(|| format!("cannot reach the Akra Gateway at {endpoint}"))?;
        relay(gateway, tokio::io::stdin(), tokio::io::stdout()).await;
        anyhow::Ok(())
    });
    // stdin 읽기는 blocking 스레드에 남을 수 있다. 기다리지 않고 끝낸다.
    runtime.shutdown_background();
    match result {
        Ok(()) => Ok(0),
        Err(error) => {
            eprintln!("{error:#}");
            Ok(1)
        }
    }
}

/// 두 방향의 바이트를 나른다. 어느 한쪽이 끝나면 멈춘다: Gateway 가 닫으면 연결이 끝을 보고, 연결이 stdin 을
/// 닫으면 Gateway 접속을 닫는다.
async fn relay<G, I, O>(gateway: G, mut input: I, mut output: O)
where
    G: AsyncRead + AsyncWrite + Unpin,
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    let (mut from_gateway, mut to_gateway) = tokio::io::split(gateway);
    tokio::select! {
        _ = tokio::io::copy(&mut input, &mut to_gateway) => {
            let _ = to_gateway.shutdown().await;
        }
        _ = tokio::io::copy(&mut from_gateway, &mut output) => {}
    }
    let _ = output.flush().await;
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::private_ipc::PrivateIpcListener;

    fn test_endpoint(name: &str) -> String {
        #[cfg(unix)]
        {
            crate::test_utils::platform_safe_temp_dir()
                .join(format!("akra-relay-{name}-{}.sock", rand::random::<u32>()))
                .display()
                .to_string()
        }
        #[cfg(windows)]
        {
            format!(r"\\.\pipe\akra-relay-test-{name}-{}", rand::random::<u64>())
        }
    }

    #[tokio::test]
    async fn relay_carries_bytes_both_ways_and_ends_when_the_gateway_closes() {
        let endpoint = test_endpoint("both-ways");
        let mut listener = PrivateIpcListener::bind(&endpoint).expect("fake Gateway binds");
        let gateway = tokio::spawn(async move {
            let mut client = listener.accept().await.expect("relay connects");
            let mut request = [0_u8; 5];
            client.read_exact(&mut request).await.expect("request");
            assert_eq!(&request, b"hello");
            client.write_all(b"world").await.expect("reply");
            // Gateway 가 접속을 닫는다 (upstream 재시작의 close 뒤와 같다).
        });
        let (mut client_input, relay_input) = tokio::io::duplex(64);
        let (relay_output, mut client_output) = tokio::io::duplex(64);
        let connected = crate::adapter::outbound::gateway_kernel_client::connect(&endpoint)
            .await
            .expect("endpoint accepts");
        client_input.write_all(b"hello").await.expect("write");
        tokio::time::timeout(
            Duration::from_secs(10),
            relay(connected, relay_input, relay_output),
        )
        .await
        .expect("relay ends after the Gateway closes");
        let mut reply = Vec::new();
        client_output.read_to_end(&mut reply).await.expect("read");
        assert_eq!(reply, b"world");
        gateway.await.expect("fake Gateway finishes");
    }

    #[tokio::test]
    async fn closed_input_closes_the_gateway_connection() {
        let endpoint = test_endpoint("input-closed");
        let mut listener = PrivateIpcListener::bind(&endpoint).expect("fake Gateway binds");
        let gateway = tokio::spawn(async move {
            let mut client = listener.accept().await.expect("relay connects");
            let mut rest = Vec::new();
            client
                .read_to_end(&mut rest)
                .await
                .expect("read until close");
            rest
        });
        let (client_input, relay_input) = tokio::io::duplex(64);
        let (relay_output, _client_output) = tokio::io::duplex(64);
        let connected = crate::adapter::outbound::gateway_kernel_client::connect(&endpoint)
            .await
            .expect("endpoint accepts");
        let mut client_input = client_input;
        client_input.write_all(b"bye").await.expect("write");
        drop(client_input);
        tokio::time::timeout(
            Duration::from_secs(10),
            relay(connected, relay_input, relay_output),
        )
        .await
        .expect("relay ends after the input closes");
        let seen = tokio::time::timeout(Duration::from_secs(10), gateway)
            .await
            .expect("fake Gateway sees the close")
            .expect("fake Gateway task");
        assert_eq!(seen, b"bye");
    }

    #[tokio::test]
    async fn probe_sees_a_listening_gateway_and_not_a_missing_one() {
        let endpoint = test_endpoint("probe");
        assert!(!endpoint_accepts(&endpoint).await);
        let mut listener = PrivateIpcListener::bind(&endpoint).expect("fake Gateway binds");
        let accepting = tokio::spawn(async move { listener.accept().await.map(drop) });
        assert!(endpoint_accepts(&endpoint).await);
        accepting.await.expect("accept task").expect("accepted");
    }

    #[test]
    fn the_relay_child_is_akra_with_the_relay_command_and_endpoint() {
        let command = super::super::connection::gateway_relay_command(
            std::path::Path::new("/opt/akra/bin/akra"),
            "/home/user/.akra/run/gateway.sock",
        );
        assert_eq!(command.get_program(), "/opt/akra/bin/akra");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [GATEWAY_RELAY_COMMAND, "/home/user/.akra/run/gateway.sock"]
        );
    }

    #[test]
    fn only_a_busy_pipe_is_retried() {
        assert_eq!(
            is_pipe_busy(&std::io::Error::from_raw_os_error(231)),
            cfg!(windows)
        );
        assert!(!is_pipe_busy(&std::io::Error::from(
            std::io::ErrorKind::NotFound
        )));
    }

    #[test]
    fn unit_tests_never_find_the_developers_gateway() {
        // 시험 빌드의 endpoint 는 아무도 듣지 않는다 (`gateway_endpoint::client_endpoint`).
        assert_eq!(reachable_gateway_endpoint(), None);
    }
}
