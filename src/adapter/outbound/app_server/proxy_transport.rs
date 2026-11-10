//! Gateway upstream app-server 에 `codex app-server proxy` 로 붙는 전송이다 (plan 18 K3-5, Q-K3-6).
//!
//! proxy 의 stdio 는 upstream 의 WebSocket 바이트를 그대로 나른다. 그래서 연결은 WebSocket client 로 말한다.
//! 이 모듈은 그 WebSocket 을 기존 줄 단위 전송(JSON 한 줄 = 메시지 하나)으로 바꾼다.
//! - 쓰기: frame 하나(JSON 한 줄)를 text 메시지 하나로 보낸다.
//! - 읽기: 전용 스레드가 text 메시지마다 한 줄을 파이프에 쓴다. 기존 줄 reader 가 그 파이프를 읽는다.
//!
//! 두 방향은 proxy stdin 을 함께 쓴다(쓰기 메시지와 ping 에 대한 pong). 메시지 하나는 잠금 안에서 통째로 쓴다.

use std::io::{self, PipeReader, Read, Write};
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocket, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

use super::connection::AppServerFrameSink;

/// app-server 의 WebSocket 경로이다. proxy 는 바이트만 나르므로 호스트 이름은 의미가 없다.
const HANDSHAKE_REQUEST_PATH: &str = "/rpc";
/// 101 응답 머리의 상한이다. 정상 응답은 200 바이트 안팎이다.
const MAX_HANDSHAKE_RESPONSE_BYTES: usize = 16 * 1024;
/// Gateway 세션과 같은 상한이다. 긴 thread 의 `thread/resume` 응답이 기본 한도를 넘을 수 있다.
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 256 << 20;
/// proxy 가 핸드셰이크에 답할 때까지 기다리는 최대 시간이다. 정상 응답은 바로 온다. 넘으면 연결을 포기하고,
/// 호출자가 proxy 프로세스를 끝낸다.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct ProxyStdio {
    pub(super) frames: ProxyFrameSink,
    pub(super) lines: PipeReader,
}

/// proxy stdio 위에서 WebSocket 핸드셰이크를 하고 두 방향을 연다.
pub(super) fn open(mut stdin: ChildStdin, stdout: ChildStdout) -> Result<ProxyStdio> {
    let stdout = handshake(&mut stdin, stdout, HANDSHAKE_TIMEOUT)?;
    let out = Arc::new(Mutex::new(stdin));
    let (lines, line_writer) = io::pipe().context("failed to open the proxy line pipe")?;
    let reader = WebSocket::from_raw_socket(
        ProxyReadSide {
            stdout,
            out: SharedProxyOut::new(out.clone()),
        },
        Role::Client,
        Some(websocket_config()),
    );
    thread::Builder::new()
        .name("akra-app-server-proxy-reader".to_string())
        .spawn(move || forward_messages_as_lines(reader, line_writer))
        .context("failed to start the proxy reader thread")?;
    let frames = ProxyFrameSink {
        socket: WebSocket::from_raw_socket(
            ProxyWriteSide {
                out: SharedProxyOut::new(out),
            },
            Role::Client,
            Some(websocket_config()),
        ),
    };
    Ok(ProxyStdio { frames, lines })
}

fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
}

/// 핸드셰이크를 보내고 `timeout` 안에 답을 확인한다. 답을 읽은 stdout 을 돌려준다. 읽기는 블로킹이므로 전용
/// 스레드에서 한다. 시간을 넘기면 그 스레드는 proxy 가 끝날 때(호출자가 끝낸다) 함께 끝난다.
fn handshake<R: Read + Send + 'static>(
    stdin: &mut impl Write,
    stdout: R,
    timeout: Duration,
) -> Result<R> {
    let key = generate_key();
    let request = format!(
        "GET {HANDSHAKE_REQUEST_PATH} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stdin
        .write_all(request.as_bytes())
        .and_then(|()| stdin.flush())
        .context("failed to send the proxy WebSocket handshake")?;
    let (stdout, response) = read_response_head_within(stdout, timeout)?;
    check_handshake_response(&response?, &key)?;
    Ok(stdout)
}

fn read_response_head_within<R: Read + Send + 'static>(
    mut stdout: R,
    timeout: Duration,
) -> Result<(R, Result<String>)> {
    let (reply, answer) = mpsc::channel();
    thread::Builder::new()
        .name("akra-app-server-proxy-handshake".to_string())
        .spawn(move || {
            let response = read_response_head(&mut stdout);
            let _ = reply.send((stdout, response));
        })
        .context("failed to start the proxy handshake reader")?;
    match answer.recv_timeout(timeout) {
        Ok(answered) => Ok(answered),
        Err(RecvTimeoutError::Timeout) => {
            bail!("app-server proxy did not answer the WebSocket handshake within {timeout:?}")
        }
        Err(RecvTimeoutError::Disconnected) => {
            bail!("the proxy handshake reader stopped without an answer")
        }
    }
}

fn check_handshake_response(response: &str, key: &str) -> Result<()> {
    let status = response.lines().next().unwrap_or_default();
    if !status.starts_with("HTTP/1.1 101") {
        bail!("app-server proxy refused the WebSocket handshake: {status}")
    }
    let expected = derive_accept_key(key.as_bytes());
    let accepted = response.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("sec-websocket-accept") && value.trim() == expected
        })
    });
    if !accepted {
        bail!("app-server proxy answered the WebSocket handshake with a wrong accept key")
    }
    Ok(())
}

/// 머리 끝(`\r\n\r\n`)까지만 읽는다. 그 뒤 바이트는 WebSocket 메시지이므로 읽지 않는다.
fn read_response_head(stdout: &mut impl Read) -> Result<String> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HANDSHAKE_RESPONSE_BYTES {
            bail!("app-server proxy handshake response is too long")
        }
        match stdout.read(&mut byte) {
            Ok(0) => bail!("app-server proxy closed during the WebSocket handshake"),
            Ok(_) => head.push(byte[0]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(error).context("failed to read the proxy WebSocket handshake");
            }
        }
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// 받은 메시지를 한 줄씩 파이프에 쓴다. 연결이 끝나면 파이프를 닫아 줄 reader 가 끝을 보게 한다.
fn forward_messages_as_lines<S: Read + Write>(
    mut socket: WebSocket<S>,
    mut line_writer: io::PipeWriter,
) {
    loop {
        let line = match socket.read() {
            Ok(Message::Text(text)) => text.as_bytes().to_vec(),
            Ok(Message::Binary(bytes)) => bytes.to_vec(),
            Ok(Message::Close(_)) | Err(_) => return,
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {
                // ping 에 대한 pong 은 tungstenite 가 쌓아 둔다. 바로 보낸다.
                let _ = socket.flush();
                continue;
            }
        };
        if line_writer
            .write_all(&line)
            .and_then(|()| line_writer.write_all(b"\n"))
            .is_err()
        {
            return;
        }
    }
}

/// 쓰기 방향이다. 연결의 stdin writer 스레드가 frame 마다 부른다.
pub(super) struct ProxyFrameSink {
    socket: WebSocket<ProxyWriteSide>,
}

impl AppServerFrameSink for ProxyFrameSink {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        let text = std::str::from_utf8(frame)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .trim_end_matches('\n');
        self.socket
            .send(Message::Text(Utf8Bytes::from(text.to_string())))
            .map_err(io::Error::other)
    }
}

/// proxy stdin 을 두 방향이 함께 쓴다. `write` 는 모으기만 하고 `flush` 가 모은 바이트를 잠금 안에서 한 번에
/// 쓴다. tungstenite 는 메시지마다 write 뒤 flush 하므로 메시지가 서로 섞이지 않는다.
struct SharedProxyOut {
    out: Arc<Mutex<ChildStdin>>,
    pending: Vec<u8>,
}

impl SharedProxyOut {
    fn new(out: Arc<Mutex<ChildStdin>>) -> Self {
        Self {
            out,
            pending: Vec::new(),
        }
    }
}

impl Write for SharedProxyOut {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut out = self
            .out
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = out.write_all(&self.pending).and_then(|()| out.flush());
        self.pending.clear();
        result
    }
}

struct ProxyReadSide {
    stdout: ChildStdout,
    out: SharedProxyOut,
}

impl Read for ProxyReadSide {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stdout.read(buffer)
    }
}

impl Write for ProxyReadSide {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.out.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

struct ProxyWriteSide {
    out: SharedProxyOut,
}

impl Read for ProxyWriteSide {
    /// 쓰기 방향은 읽지 않는다. 읽기는 `ProxyReadSide` 가 한다.
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }
}

impl Write for ProxyWriteSide {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.out.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const TEST_HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(300);

    #[test]
    fn the_handshake_checks_the_status_and_the_accept_key() {
        let mut sent = Vec::new();
        let Err(error) = handshake(
            &mut sent,
            Cursor::new(b"HTTP/1.1 400 Bad Request\r\n\r\n".to_vec()),
            TEST_HANDSHAKE_TIMEOUT,
        ) else {
            panic!("a refused upgrade must fail");
        };
        assert!(format!("{error:#}").contains("400"), "{error:#}");
        let request = String::from_utf8(sent).expect("request is text");
        assert!(request.starts_with("GET /rpc HTTP/1.1\r\n"), "{request}");

        let Err(error) = handshake(
            &mut Vec::new(),
            Cursor::new(
                b"HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: wrong\r\n\r\n".to_vec(),
            ),
            TEST_HANDSHAKE_TIMEOUT,
        ) else {
            panic!("a wrong accept key must fail");
        };
        assert!(format!("{error:#}").contains("accept key"), "{error:#}");
    }

    #[test]
    fn the_handshake_gives_up_when_the_proxy_never_answers() {
        let (silent, _open_writer) = io::pipe().expect("pipe should open");
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            let result = handshake(&mut Vec::new(), silent, TEST_HANDSHAKE_TIMEOUT);
            let _ = done.send(result.err().map(|error| format!("{error:#}")));
        });

        let error = finished
            .recv_timeout(Duration::from_secs(3))
            .expect("the handshake must end at its deadline");
        assert!(
            error.is_some_and(|error| error.contains("did not answer")),
            "a silent proxy is an error"
        );
    }

    #[test]
    fn the_response_head_stops_before_the_first_message() {
        let mut stdout = Cursor::new(b"HTTP/1.1 101 OK\r\nA: b\r\n\r\nNEXT".to_vec());
        let head = read_response_head(&mut stdout).expect("head should read");
        assert_eq!(head, "HTTP/1.1 101 OK\r\nA: b\r\n\r\n");
        let mut rest = String::new();
        stdout.read_to_string(&mut rest).expect("rest should read");
        assert_eq!(rest, "NEXT", "message bytes stay unread");
    }
}
