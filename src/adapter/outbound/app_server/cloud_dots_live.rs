//! dot 메시지 방의 실시간 상태를 본다 (D-19, dots-integration §4 "실시간"). dot 이 답을 만드는 중인지, 끝났는지를
//! 서비스가 밀어 준다. Desktop 은 이것으로 "생각 중" 표시와 새 답 알림을 만든다.
//!
//! 통로는 둘이고 순서가 있다 (2026-11-14 실계정 확인):
//! 1. `GET /celsius/ws/user` 가 서명된 `websocket_url` 을 준다. 그 WebSocket 에 명령 묶음(JSON 배열)으로 `connect`
//!    와 주제 `calpico-room-live:<방 id>` 의 `subscribe` 를 보낸다.
//! 2. 구독 답을 받은 뒤 `POST /messaging/rooms/{id}/live` (SSE)를 연다. 이 스트림은 `calpico-room-live-ready` 와 ping
//!    만 준다. 열려 있는 동안 서비스가 상태를 WebSocket 의 주제로 민다. SSE 없이 구독만 하면 아무것도 오지 않는다.
//!
//! 읽기만 한다. `websocket_url` 은 서명을 담으므로 로그에 남기지 않는다.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio_tungstenite::tungstenite::Message;

use super::cloud_dots::CloudDotsRestAdapter;
use super::cloud_upstream::ensure_crypto_provider;
use crate::application::port::outbound::dots_port::{DotLiveListener, DotLiveState};

const LIVE_SOCKET_URL: &str = "https://chatgpt.com/backend-api/celsius/ws/user";
const ROOMS_URL: &str = "https://chatgpt.com/backend-api/messaging/rooms";
const LIVE_TOPIC_PREFIX: &str = "calpico-room-live:";
const CONNECT_COMMAND_ID: u64 = 1;
const SUBSCRIBE_COMMAND_ID: u64 = 2;
const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(15);
const PING_EVERY: Duration = Duration::from_secs(25);
/// 다시 붙기 전에 기다리는 시간이다. 실패가 이어지면 두 배씩 늘려 상한에서 멈춘다.
const RECONNECT_FIRST: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(60);

/// 방 하나의 실시간 보기를 시작한다. 돌려준 값을 버리면 보기를 멈추고 연결과 curl 을 정리한다.
pub(super) fn start_room_watch(
    adapter: CloudDotsRestAdapter,
    room_id: String,
    on_state: DotLiveListener,
) -> Box<dyn Send> {
    Box::new(AbortOnDrop(tokio::spawn(run_watch(
        adapter, room_id, on_state,
    ))))
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// 끊기면 다시 붙는다. 끊긴 동안은 `connected: false` 를 한 번 알린다.
async fn run_watch(adapter: CloudDotsRestAdapter, room_id: String, on_state: DotLiveListener) {
    let mut wait = RECONNECT_FIRST;
    loop {
        let mut subscribed = false;
        let outcome = watch_once(&adapter, &room_id, &on_state, &mut subscribed).await;
        if let Err(error) = outcome {
            // 주소(서명)와 토큰은 오류 글자에 넣지 않는다.
            tracing::warn!(error = %format!("{error:#}"), "dots live watch ended");
        }
        on_state(DotLiveState::disconnected(&room_id));
        wait = next_reconnect_wait(wait, subscribed);
        tokio::time::sleep(wait).await;
    }
}

/// 구독까지 갔던 연결이 끊겼으면 처음 간격으로 돌아간다. 붙지도 못했으면 간격을 늘린다.
fn next_reconnect_wait(previous: Duration, was_subscribed: bool) -> Duration {
    if was_subscribed {
        RECONNECT_FIRST
    } else {
        (previous * 2).min(RECONNECT_MAX)
    }
}

async fn watch_once(
    adapter: &CloudDotsRestAdapter,
    room_id: &str,
    on_state: &DotLiveListener,
    subscribed: &mut bool,
) -> Result<()> {
    let descriptor = {
        let adapter = adapter.clone();
        tokio::task::spawn_blocking(move || adapter.get(LIVE_SOCKET_URL, "dots live socket"))
            .await
            .context("the dots live socket request stopped")??
    };
    let socket_url = parse_socket_url(&descriptor)?;
    ensure_crypto_provider();
    let (mut socket, _response) = tokio_tungstenite::connect_async(socket_url.as_str())
        .await
        .map_err(|_| anyhow!("the dots live socket did not connect"))?;
    socket
        .send(Message::text(connect_batch(room_id)))
        .await
        .map_err(|_| anyhow!("the dots live socket closed before the subscription"))?;

    // 구독 답을 받은 뒤에 SSE 를 연다. 공식 앱과 같은 순서이다.
    let subscription = tokio::time::timeout(SUBSCRIBE_TIMEOUT, async {
        while let Some(frame) = socket.next().await {
            if let Ok(Message::Text(text)) = frame
                && let Some(outcome) = subscription_outcome(text.as_str())
            {
                return outcome;
            }
        }
        Err(anyhow!(
            "the dots live socket closed before the subscription"
        ))
    })
    .await
    .map_err(|_| anyhow!("the dots live subscription timed out"))?;
    subscription?;
    *subscribed = true;

    // 붙었다는 것만으로는 상태를 알리지 않는다. 서비스의 첫 실제 상태가 "연결됨"을 알린다. 여기서 "연결됨, 일하지
    // 않음"을 먼저 보내면, 끊기기 전에 일하던 방이 아직 일하는 중이어도 받는 쪽이 답이 끝난 것으로 본다.
    let mut live_stream = start_live_stream(adapter, room_id).await?;
    let mut ping = tokio::time::interval(PING_EVERY);
    loop {
        tokio::select! {
            frame = socket.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    for state in parse_live_states(text.as_str(), room_id) {
                        on_state(state);
                    }
                }
                Some(Ok(Message::Close(_))) | None => bail!("the dots live socket closed"),
                Some(Ok(_)) => {}
                Some(Err(_)) => bail!("the dots live socket failed"),
            },
            status = live_stream.wait() => {
                bail!("the dots live stream ended ({})", status.map(|status| status.to_string()).unwrap_or_default());
            }
            _ = ping.tick() => {
                if socket.send(Message::Ping(Default::default())).await.is_err() {
                    bail!("the dots live socket closed");
                }
            }
        }
    }
}

/// `connect` 와 주제 구독을 한 묶음으로 보낸다. presence 는 `background` 이다: 이 연결은 화면이 아니라 데몬이다.
fn connect_batch(room_id: &str) -> String {
    serde_json::json!([
        {
            "id": CONNECT_COMMAND_ID,
            "command": { "type": "connect", "presence": { "type": "presence", "state": "background" } },
        },
        {
            "id": SUBSCRIBE_COMMAND_ID,
            "command": { "type": "subscribe", "topic_id": format!("{LIVE_TOPIC_PREFIX}{room_id}") },
        },
    ])
    .to_string()
}

#[derive(Deserialize)]
struct SocketDescriptor {
    websocket_url: String,
}

fn parse_socket_url(descriptor: &str) -> Result<String> {
    let descriptor: SocketDescriptor = serde_json::from_str(descriptor)
        .context("failed to parse the dots live socket response")?;
    if !descriptor.websocket_url.starts_with("wss://") {
        bail!("the dots live socket address is not a secure WebSocket");
    }
    Ok(descriptor.websocket_url)
}

/// 서비스의 프레임은 항목의 배열이다. 구독 명령의 답이 있으면 그 결과이고, 없으면 `None` 이다.
fn subscription_outcome(text: &str) -> Option<Result<()>> {
    let frames: Vec<serde_json::Value> = serde_json::from_str(text).ok()?;
    frames.iter().find_map(|frame| {
        let id = frame.get("id")?.as_u64()?;
        let kind = frame.get("type")?.as_str()?;
        match (id, kind) {
            (SUBSCRIBE_COMMAND_ID, "reply") => Some(Ok(())),
            (CONNECT_COMMAND_ID | SUBSCRIBE_COMMAND_ID, "error") => Some(Err(anyhow!(
                "the dots live socket refused the subscription ({})",
                frame
                    .get("error")
                    .and_then(|error| error.as_str())
                    .unwrap_or("unknown")
            ))),
            _ => None,
        }
    })
}

#[derive(Deserialize)]
struct LiveThread {
    #[serde(default)]
    stream_state: Option<String>,
    #[serde(default)]
    turn_status: Option<String>,
    #[serde(default)]
    is_active: Option<bool>,
    #[serde(default)]
    computer_status: Option<String>,
    #[serde(default)]
    activity_status: Option<LiveActivity>,
}

#[derive(Deserialize)]
struct LiveActivity {
    #[serde(default)]
    status_label: Option<String>,
}

/// 프레임에서 이 방의 `calpico-room-live-state` 를 꺼낸다. 다른 주제, 다른 방, 모르는 종류는 버린다.
///
/// 붙는 중(`stream_state: attaching`)인 thread 뿐인 상태도 버린다. 그 상태는 턴의 상태를 아직 모른다
/// (`turn_status: null`). 일하지 않는 것으로 읽으면, 다시 붙은 직후에 답이 끝난 것처럼 보인다. 서비스는 바로 뒤에
/// `attached` 상태를 보낸다 (2026-11-14 실측: 0.6초 뒤).
fn parse_live_states(text: &str, room_id: &str) -> Vec<DotLiveState> {
    let Ok(frames) = serde_json::from_str::<Vec<serde_json::Value>>(text) else {
        return Vec::new();
    };
    frames
        .iter()
        .filter(|frame| frame.get("type").and_then(|kind| kind.as_str()) == Some("message"))
        .filter_map(|frame| frame.get("payload")?.get("payload"))
        .filter(|state| {
            state.get("type").and_then(|kind| kind.as_str()) == Some("calpico-room-live-state")
                && state.get("room_id").and_then(|room| room.as_str()) == Some(room_id)
        })
        .filter_map(|state| {
            let threads: Vec<LiveThread> =
                serde_json::from_value(state.get("threads")?.clone()).ok()?;
            if !threads.is_empty()
                && threads
                    .iter()
                    .all(|thread| thread.stream_state.as_deref() == Some("attaching"))
            {
                return None;
            }
            let working = threads.iter().any(|thread| {
                thread.turn_status.as_deref() == Some("in_progress")
                    || thread.is_active == Some(true)
            });
            Some(DotLiveState {
                room_id: room_id.to_string(),
                connected: true,
                working,
                // 일하는 중일 때만 서비스의 상태 글자를 쓴다. 끝난 턴의 글자는 지난 상태이다.
                activity_label: threads
                    .iter()
                    .filter(|_| working)
                    .find_map(|thread| thread.activity_status.as_ref()?.status_label.clone())
                    .filter(|label| !label.trim().is_empty()),
                using_computer: threads
                    .iter()
                    .any(|thread| thread.computer_status.as_deref() == Some("using_computer")),
            })
        })
        .collect()
}

/// SSE 를 curl 로 연다. 토큰은 stdin 의 config 로만 넘긴다. 본문(ready, ping)은 쓰지 않으므로 버린다. 자식은 값을
/// 버리면 끝난다.
async fn start_live_stream(
    adapter: &CloudDotsRestAdapter,
    room_id: &str,
) -> Result<tokio::process::Child> {
    let (command, config) = adapter.stream_command(&format!("{ROOMS_URL}/{room_id}/live"), "{}")?;
    let mut command = tokio::process::Command::from(command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .context("failed to start curl for the dots live stream")?;
    let mut stdin = child
        .stdin
        .take()
        .context("the dots live stream has no stdin")?;
    stdin
        .write_all(config.as_bytes())
        .await
        .context("failed to configure the dots live stream")?;
    drop(stdin);
    Ok(child)
}

/// 데몬이 curl 을 띄울 때 콘솔 창이 뜨지 않게 한다.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const ROOM: &str = "room1";

    fn state_frame(room: &str, threads: serde_json::Value) -> String {
        serde_json::json!([{
            "type": "message",
            "topic_id": format!("calpico-room-live:{room}"),
            "payload": {
                "type": "calpico-room-live",
                "payload": {
                    "type": "calpico-room-live-state",
                    "version": 1,
                    "room_id": room,
                    "threads": threads,
                    "subscription_id": "sub-1",
                    "revision": 3,
                },
                "metadata": null,
            },
        }])
        .to_string()
    }

    #[test]
    fn the_first_frame_connects_and_subscribes_to_the_room_topic() {
        let batch: serde_json::Value = serde_json::from_str(&connect_batch(ROOM)).unwrap();
        assert_eq!(batch[0]["command"]["type"], "connect");
        assert_eq!(batch[0]["command"]["presence"]["state"], "background");
        assert_eq!(batch[1]["id"], SUBSCRIBE_COMMAND_ID);
        assert_eq!(batch[1]["command"]["type"], "subscribe");
        assert_eq!(batch[1]["command"]["topic_id"], "calpico-room-live:room1");
    }

    #[test]
    fn the_subscription_reply_or_an_error_ends_the_wait() {
        let replies = r#"[{"id":1,"type":"reply","reply":{"type":"connect"}},{"id":2,"type":"reply","reply":{"type":"subscribe","topic_id":"calpico-room-live:room1","recovered":false}}]"#;
        assert!(subscription_outcome(replies).unwrap().is_ok());
        let refused = r#"[{"id":2,"type":"error","error":"not_connected"}]"#;
        let error = subscription_outcome(refused).unwrap().unwrap_err();
        assert!(format!("{error:#}").contains("not_connected"));
        // 연결 답만 있으면 아직 기다린다.
        assert!(
            subscription_outcome(r#"[{"id":1,"type":"reply","reply":{"type":"connect"}}]"#)
                .is_none()
        );
        assert!(subscription_outcome("not json").is_none());
    }

    #[test]
    fn a_turn_in_progress_is_working_with_the_service_label() {
        let frame = state_frame(
            ROOM,
            serde_json::json!([{
                "thread_id": "t1", "stream_state": "attached", "turn_id": "turn-1",
                "turn_status": "in_progress", "is_active": true, "summary": [],
                "activity_status": { "status_type": "thinking", "status_label": "Thinking" },
            }]),
        );
        let states = parse_live_states(&frame, ROOM);
        assert_eq!(
            states,
            vec![DotLiveState {
                room_id: ROOM.to_string(),
                connected: true,
                working: true,
                activity_label: Some("Thinking".to_string()),
                using_computer: false,
            }]
        );
    }

    #[test]
    fn a_finished_turn_is_idle_and_drops_the_old_label() {
        let frame = state_frame(
            ROOM,
            serde_json::json!([{
                "thread_id": "t1", "stream_state": "attached", "turn_id": "turn-1",
                "turn_status": "completed", "is_active": false,
                "computer_status": "using_computer",
                "activity_status": { "status_type": "thinking", "status_label": "Thinking" },
            }]),
        );
        let states = parse_live_states(&frame, ROOM);
        assert!(!states[0].working);
        assert_eq!(states[0].activity_label, None);
        assert!(states[0].using_computer);
    }

    #[test]
    fn a_thread_that_is_still_attaching_says_nothing_about_the_turn() {
        // 다시 붙은 직후의 첫 상태이다. 턴의 상태를 모른다. "일하지 않음"으로 알리면 답이 끝난 것처럼 보인다.
        let attaching = state_frame(
            ROOM,
            serde_json::json!([{
                "thread_id": "t1", "stream_state": "attaching", "turn_id": null,
                "turn_status": null, "is_active": null, "summary": [],
            }]),
        );
        assert!(parse_live_states(&attaching, ROOM).is_empty());
        // 붙은 뒤의 상태는 알린다.
        let attached = state_frame(
            ROOM,
            serde_json::json!([{
                "thread_id": "t1", "stream_state": "attached", "turn_id": "turn-1",
                "turn_status": "in_progress", "is_active": true,
            }]),
        );
        assert!(parse_live_states(&attached, ROOM)[0].working);
    }

    #[test]
    fn other_rooms_other_events_and_replies_are_ignored() {
        assert!(
            parse_live_states(&state_frame("other-room", serde_json::json!([])), ROOM).is_empty()
        );
        let error = r#"[{"type":"message","topic_id":"calpico-room-live:room1","payload":{"type":"calpico-room-live","payload":{"type":"calpico-room-live-error","room_id":"room1","code":"room_unavailable"}}}]"#;
        assert!(parse_live_states(error, ROOM).is_empty());
        assert!(parse_live_states(r#"[{"id":2,"type":"reply","reply":{}}]"#, ROOM).is_empty());
        assert!(parse_live_states("not json", ROOM).is_empty());
    }

    #[test]
    fn the_socket_address_must_be_a_secure_websocket() {
        assert_eq!(
            parse_socket_url(r#"{"websocket_url":"wss://ws.example/p/ws/user/u?verify=x"}"#)
                .unwrap(),
            "wss://ws.example/p/ws/user/u?verify=x"
        );
        assert!(parse_socket_url(r#"{"websocket_url":"ws://plain.example/"}"#).is_err());
        assert!(parse_socket_url("{}").is_err());
    }

    #[test]
    fn reconnect_waits_grow_until_a_subscription_resets_them() {
        let mut wait = RECONNECT_FIRST;
        let mut seen = Vec::new();
        for _ in 0..6 {
            wait = next_reconnect_wait(wait, false);
            seen.push(wait.as_secs());
        }
        assert_eq!(seen, vec![10, 20, 40, 60, 60, 60]);
        assert_eq!(next_reconnect_wait(wait, true), RECONNECT_FIRST);
    }

    /// 실계정 읽기 확인이다 (조회만, 메시지를 보내지 않는다). 첫 dot 의 방을 12초 본다. 구독과 SSE 가 되면 서비스가
    /// 현재 상태를 한 번 민다. 받은 상태의 수와 종류만 찍는다.
    /// 수동 실행: `cargo test --lib cloud_dots_live::tests::live_room_watch -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "real ChatGPT account and network; run manually"]
    async fn live_room_watch() {
        use crate::application::port::outbound::dots_port::DotsPort;
        let auth_path = super::super::cloud_upstream::codex_auth_path().expect("auth path");
        let adapter = CloudDotsRestAdapter::new(auth_path);
        let dots = {
            let adapter = adapter.clone();
            tokio::task::spawn_blocking(move || adapter.list_dots())
                .await
                .unwrap()
                .expect("the dot list must load")
        };
        let room_id = dots
            .iter()
            .find_map(|dot| dot.messaging_room_id.clone())
            .expect("a dot with a message room");
        let seen: Arc<Mutex<Vec<DotLiveState>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let watch = start_room_watch(
            adapter,
            room_id,
            Box::new(move |state| sink.lock().unwrap().push(state)),
        );
        tokio::time::sleep(Duration::from_secs(12)).await;
        drop(watch);
        let seen = seen.lock().unwrap();
        eprintln!(
            "live: {} states, connected {}, working {}, disconnected {}",
            seen.len(),
            seen.iter().filter(|state| state.connected).count(),
            seen.iter().filter(|state| state.working).count(),
            seen.iter().filter(|state| !state.connected).count()
        );
        assert!(
            seen.iter().any(|state| state.connected),
            "the watch must connect"
        );
    }
}
