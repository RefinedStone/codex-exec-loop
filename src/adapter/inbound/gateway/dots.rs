//! Gateway 의 `akra/dots/*` 처리이다 (D-19). 사용자의 클라우드 에이전트(dot) 목록을 준다. 조회는 외부 서비스
//! 요청(블로킹)이므로 요청마다 블로킹 작업으로 돌린다. 접속의 읽기 고리는 기다리지 않는다. dots 가 꺼져 있으면
//! 이 처리 자체가 없고, 요청은 모르는 method 로 답한다.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::application::port::outbound::dots_port::{DotLiveState, DotsPort};

pub(super) const DOTS_METHOD_PREFIX: &str = "akra/dots/";
/// params 없음 -> result `{ dots: [DotSummary] }`
const DOTS_LIST_METHOD: &str = "akra/dots/list";
/// params `{ roomId }` -> result `{ messages: [DotMessage] }`. 오래된 것부터이다.
const DOTS_MESSAGES_METHOD: &str = "akra/dots/messages";
/// params `{ roomId, text }` -> result `{ sent: true }`. 실계정 쓰기이다. 클라이언트는 사용자가 허락한 뒤에만
/// 보낸다 (D-19).
const DOTS_SEND_METHOD: &str = "akra/dots/send";
/// params `{ roomId }` -> result `{ watching: true }`. 그 뒤 방의 실시간 상태가 바뀔 때마다 이 접속에
/// `akra/dots/live` 알림(params = DotLiveState)이 온다. 읽기만 한다. 접속이 끝나면 보기도 끝난다.
const DOTS_WATCH_METHOD: &str = "akra/dots/watch";
/// params `{ roomId }` -> result `{ watching: false }`.
const DOTS_UNWATCH_METHOD: &str = "akra/dots/unwatch";
const DOTS_LIVE_NOTIFICATION: &str = "akra/dots/live";
/// 접속 하나가 볼 수 있는 방의 수이다. 방마다 WebSocket 하나와 curl 하나를 쓴다.
const MAX_WATCHED_ROOMS: usize = 8;
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const JSON_RPC_INVALID_PARAMS: i64 = -32602;
const DOTS_ERROR: i64 = -32000;

/// 요청 하나를 블로킹 작업으로 처리하고 답을 접속의 보내기 함으로 보낸다. 답은 원격 서비스의 내용이므로 기록하지 않는다 (plan 25 §3.1).
pub(super) fn spawn_request(
    dots: Arc<dyn DotsPort>,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
) {
    tokio::spawn(async move {
        let fallback_id = id.clone();
        let response =
            tokio::task::spawn_blocking(move || respond(dots.as_ref(), id, &method, &text))
                .await
                .unwrap_or_else(|_| {
                    error_response(fallback_id, DOTS_ERROR, "the dots request stopped")
                });
        let _ = outbox.send(Message::text(response)).await;
    });
}

#[derive(Deserialize)]
struct RequestEnvelope {
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagesParams {
    room_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendParams {
    room_id: String,
    text: String,
}

fn params<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let envelope = serde_json::from_str::<RequestEnvelope>(text).ok()?;
    serde_json::from_value(envelope.params).ok()
}

/// 접속 하나가 보고 있는 방들이다. 값을 버리면 모든 보기가 멈춘다 (접속이 끝날 때).
#[derive(Default)]
pub(super) struct DotWatches {
    rooms: HashMap<String, Box<dyn Send>>,
}

impl DotWatches {
    /// `akra/dots/watch` 와 `akra/dots/unwatch` 이면 처리하고 답을 돌려준다. 다른 method 이면 `None` 이다.
    pub(super) fn handle(
        &mut self,
        dots: &dyn DotsPort,
        id: &Value,
        method: &str,
        text: &str,
        outbox: &mpsc::Sender<Message>,
    ) -> Option<String> {
        if method != DOTS_WATCH_METHOD && method != DOTS_UNWATCH_METHOD {
            return None;
        }
        let Some(params) = params::<MessagesParams>(text) else {
            return Some(error_response(
                id.clone(),
                JSON_RPC_INVALID_PARAMS,
                "`roomId` is required",
            ));
        };
        if method == DOTS_UNWATCH_METHOD {
            self.rooms.remove(&params.room_id);
            return Some(watch_response(id, false));
        }
        if self.rooms.contains_key(&params.room_id) {
            return Some(watch_response(id, true));
        }
        if self.rooms.len() >= MAX_WATCHED_ROOMS {
            return Some(error_response(
                id.clone(),
                DOTS_ERROR,
                "this connection already watches the most rooms it can",
            ));
        }
        let outbox = outbox.clone();
        let on_state = Box::new(move |state: DotLiveState| {
            // 보내기 함이 찼으면 이 상태는 버린다. 다음 상태가 화면을 고친다.
            let _ = outbox.try_send(Message::text(live_notification(&state)));
        });
        Some(match dots.watch_room(&params.room_id, on_state) {
            Ok(watch) => {
                self.rooms.insert(params.room_id, watch);
                watch_response(id, true)
            }
            Err(error) => error_response(id.clone(), DOTS_ERROR, &format!("{error:#}")),
        })
    }
}

fn watch_response(id: &Value, watching: bool) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "watching": watching } }).to_string()
}

fn live_notification(state: &DotLiveState) -> String {
    json!({ "jsonrpc": "2.0", "method": DOTS_LIVE_NOTIFICATION, "params": state }).to_string()
}

fn respond(dots: &dyn DotsPort, id: Value, method: &str, text: &str) -> String {
    let result = match method {
        DOTS_LIST_METHOD => dots
            .list_dots()
            .and_then(|dots| Ok(json!({ "dots": serde_json::to_value(dots)? }))),
        DOTS_MESSAGES_METHOD => {
            let Some(params) = params::<MessagesParams>(text) else {
                return error_response(id, JSON_RPC_INVALID_PARAMS, "`roomId` is required");
            };
            // 답은 `{ messages, dotReadAt }` 이다.
            dots.room_messages(&params.room_id)
                .and_then(|room| Ok(serde_json::to_value(room)?))
        }
        DOTS_SEND_METHOD => {
            let Some(params) = params::<SendParams>(text) else {
                return error_response(
                    id,
                    JSON_RPC_INVALID_PARAMS,
                    "`roomId` and `text` are required",
                );
            };
            dots.send_room_message(&params.room_id, &params.text)
                .map(|()| json!({ "sent": true }))
        }
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
        Err(error) => error_response(id, DOTS_ERROR, &format!("{error:#}")),
    }
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::dots_port::{
        DotLiveListener, DotMessage, DotRoomMessages, DotSummary,
    };
    use anyhow::{Result, anyhow};

    struct FakeDots(bool);

    static SENT: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());
    static WATCHING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    /// 버리면 그 방을 보기 목록에서 뺀다.
    struct StopsWatching(String);

    impl Drop for StopsWatching {
        fn drop(&mut self) {
            WATCHING.lock().unwrap().retain(|room| room != &self.0);
        }
    }

    impl DotsPort for FakeDots {
        fn list_dots(&self) -> Result<Vec<DotSummary>> {
            if !self.0 {
                return Err(anyhow!("the dots list request failed (exit status: 22)"));
            }
            Ok(vec![DotSummary {
                id: "dot-1".into(),
                display_name: "Mika".into(),
                status: "active".into(),
                kind: "orbit".into(),
                paused: false,
                root_thread_id: Some("thread-1".into()),
                messaging_room_id: Some("room-1".into()),
            }])
        }

        fn room_messages(&self, room_id: &str) -> Result<DotRoomMessages> {
            Ok(DotRoomMessages {
                messages: vec![DotMessage {
                    id: format!("{room_id}-m1"),
                    created_at: "2026-11-14T01:00:00Z".into(),
                    author: "Mika".into(),
                    from_dot: true,
                    text: "hi".into(),
                }],
                dot_read_at: Some("2026-11-14T01:00:03Z".into()),
            })
        }

        fn watch_room(&self, room_id: &str, on_state: DotLiveListener) -> Result<Box<dyn Send>> {
            if !self.0 {
                return Err(anyhow!("the message room id is not valid"));
            }
            // 가짜는 보기를 시작하자마자 "일하는 중"을 한 번 알린다.
            on_state(DotLiveState {
                working: true,
                activity_label: Some("Thinking".into()),
                ..DotLiveState::connected(room_id)
            });
            WATCHING.lock().unwrap().push(room_id.to_string());
            Ok(Box::new(StopsWatching(room_id.to_string())))
        }

        fn send_room_message(&self, room_id: &str, text: &str) -> Result<()> {
            SENT.lock().unwrap().push((room_id.into(), text.into()));
            Ok(())
        }
    }

    #[test]
    fn a_send_needs_a_room_and_a_text_and_reaches_the_port_once() {
        let request = r#"{"jsonrpc":"2.0","id":6,"method":"akra/dots/send","params":{"roomId":"room-send","text":"hello"}}"#;
        let response: Value = serde_json::from_str(&respond(
            &FakeDots(true),
            json!(6),
            "akra/dots/send",
            request,
        ))
        .unwrap();
        assert_eq!(response["result"]["sent"], json!(true));
        let sent: Vec<_> = SENT
            .lock()
            .unwrap()
            .iter()
            .filter(|(room, _)| room == "room-send")
            .cloned()
            .collect();
        assert_eq!(sent, vec![("room-send".to_string(), "hello".to_string())]);

        let missing: Value = serde_json::from_str(&respond(
            &FakeDots(true),
            json!(7),
            "akra/dots/send",
            r#"{"jsonrpc":"2.0","id":7,"method":"akra/dots/send","params":{"roomId":"room-other"}}"#,
        ))
        .unwrap();
        assert_eq!(missing["error"]["code"], json!(JSON_RPC_INVALID_PARAMS));
        assert!(
            !SENT
                .lock()
                .unwrap()
                .iter()
                .any(|(room, _)| room == "room-other")
        );
    }

    const LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"akra/dots/list"}"#;

    #[test]
    fn the_list_answers_dots_in_camel_case() {
        let response: Value =
            serde_json::from_str(&respond(&FakeDots(true), json!(1), "akra/dots/list", LIST))
                .unwrap();
        assert_eq!(response["id"], json!(1));
        let dot = &response["result"]["dots"][0];
        assert_eq!(dot["displayName"], json!("Mika"));
        assert_eq!(dot["rootThreadId"], json!("thread-1"));
        assert_eq!(dot["messagingRoomId"], json!("room-1"));
    }

    #[test]
    fn the_messages_of_a_room_need_a_room_id() {
        let request = r#"{"jsonrpc":"2.0","id":4,"method":"akra/dots/messages","params":{"roomId":"room-1"}}"#;
        let response: Value = serde_json::from_str(&respond(
            &FakeDots(true),
            json!(4),
            "akra/dots/messages",
            request,
        ))
        .unwrap();
        let message = &response["result"]["messages"][0];
        assert_eq!(message["id"], json!("room-1-m1"));
        assert_eq!(message["fromDot"], json!(true));
        assert_eq!(message["createdAt"], json!("2026-11-14T01:00:00Z"));
        assert_eq!(
            response["result"]["dotReadAt"],
            json!("2026-11-14T01:00:03Z")
        );

        let missing: Value = serde_json::from_str(&respond(
            &FakeDots(true),
            json!(5),
            "akra/dots/messages",
            r#"{"jsonrpc":"2.0","id":5,"method":"akra/dots/messages"}"#,
        ))
        .unwrap();
        assert_eq!(missing["error"]["code"], json!(JSON_RPC_INVALID_PARAMS));
    }

    #[test]
    fn a_failed_list_and_an_unknown_method_are_errors() {
        let failed: Value =
            serde_json::from_str(&respond(&FakeDots(false), json!(2), "akra/dots/list", LIST))
                .unwrap();
        assert_eq!(failed["error"]["code"], json!(DOTS_ERROR));
        let unknown: Value =
            serde_json::from_str(&respond(&FakeDots(true), json!(3), "akra/dots/nope", LIST))
                .unwrap();
        assert_eq!(unknown["error"]["code"], json!(JSON_RPC_METHOD_NOT_FOUND));
    }

    fn watch_request(method: &str, room: &str) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":9,"method":"{method}","params":{{"roomId":"{room}"}}}}"#)
    }

    #[test]
    fn watching_a_room_pushes_its_live_state_until_the_watch_is_dropped() {
        let (outbox, mut received) = mpsc::channel::<Message>(8);
        let mut watches = DotWatches::default();
        let request = watch_request("akra/dots/watch", "room-watch");
        let response: Value = serde_json::from_str(
            &watches
                .handle(
                    &FakeDots(true),
                    &json!(9),
                    "akra/dots/watch",
                    &request,
                    &outbox,
                )
                .expect("watch is handled"),
        )
        .unwrap();
        assert_eq!(response["result"]["watching"], json!(true));

        let pushed: Value = serde_json::from_str(
            received
                .try_recv()
                .expect("a live state")
                .to_text()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(pushed["method"], json!("akra/dots/live"));
        assert_eq!(pushed["params"]["roomId"], json!("room-watch"));
        assert_eq!(pushed["params"]["working"], json!(true));
        assert_eq!(pushed["params"]["activityLabel"], json!("Thinking"));
        assert!(pushed.get("id").is_none());

        // 같은 방을 다시 보라고 해도 보기는 하나이다.
        watches.handle(
            &FakeDots(true),
            &json!(10),
            "akra/dots/watch",
            &request,
            &outbox,
        );
        assert_eq!(
            WATCHING
                .lock()
                .unwrap()
                .iter()
                .filter(|room| *room == "room-watch")
                .count(),
            1
        );

        // 접속이 끝나면(값을 버리면) 보기도 끝난다.
        drop(watches);
        assert!(
            !WATCHING
                .lock()
                .unwrap()
                .iter()
                .any(|room| room == "room-watch")
        );
    }

    #[test]
    fn unwatch_stops_one_room_and_other_methods_are_not_handled() {
        let (outbox, _received) = mpsc::channel::<Message>(8);
        let mut watches = DotWatches::default();
        let watch = watch_request("akra/dots/watch", "room-unwatch");
        watches.handle(
            &FakeDots(true),
            &json!(1),
            "akra/dots/watch",
            &watch,
            &outbox,
        );
        let unwatch = watch_request("akra/dots/unwatch", "room-unwatch");
        let response: Value = serde_json::from_str(
            &watches
                .handle(
                    &FakeDots(true),
                    &json!(2),
                    "akra/dots/unwatch",
                    &unwatch,
                    &outbox,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(response["result"]["watching"], json!(false));
        assert!(
            !WATCHING
                .lock()
                .unwrap()
                .iter()
                .any(|room| room == "room-unwatch")
        );

        assert!(
            watches
                .handle(&FakeDots(true), &json!(3), "akra/dots/list", LIST, &outbox)
                .is_none()
        );
        let failed: Value = serde_json::from_str(
            &watches
                .handle(
                    &FakeDots(false),
                    &json!(4),
                    "akra/dots/watch",
                    &watch,
                    &outbox,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(failed["error"]["code"], json!(DOTS_ERROR));
    }

    #[test]
    fn a_connection_watches_a_limited_number_of_rooms() {
        let (outbox, _received) = mpsc::channel::<Message>(64);
        let mut watches = DotWatches::default();
        for index in 0..MAX_WATCHED_ROOMS {
            let request = watch_request("akra/dots/watch", &format!("room-limit-{index}"));
            watches.handle(
                &FakeDots(true),
                &json!(index),
                "akra/dots/watch",
                &request,
                &outbox,
            );
        }
        let over = watch_request("akra/dots/watch", "room-limit-over");
        let response: Value = serde_json::from_str(
            &watches
                .handle(
                    &FakeDots(true),
                    &json!(99),
                    "akra/dots/watch",
                    &over,
                    &outbox,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(response["error"]["code"], json!(DOTS_ERROR));
    }
}
