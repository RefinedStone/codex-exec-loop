//! Gateway 의 `akra/dots/*` 처리이다 (D-19). 사용자의 클라우드 에이전트(dot) 목록을 준다. 조회는 외부 서비스
//! 요청(블로킹)이므로 요청마다 블로킹 작업으로 돌린다. 접속의 읽기 고리는 기다리지 않는다. dots 가 꺼져 있으면
//! 이 처리 자체가 없고, 요청은 모르는 method 로 답한다.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::application::port::outbound::dots_port::DotsPort;

pub(super) const DOTS_METHOD_PREFIX: &str = "akra/dots/";
/// params 없음 -> result `{ dots: [DotSummary] }`
const DOTS_LIST_METHOD: &str = "akra/dots/list";
/// params `{ roomId }` -> result `{ messages: [DotMessage] }`. 오래된 것부터이다.
const DOTS_MESSAGES_METHOD: &str = "akra/dots/messages";
/// params `{ roomId, text }` -> result `{ sent: true }`. 실계정 쓰기이다. 클라이언트는 사용자가 허락한 뒤에만
/// 보낸다 (D-19).
const DOTS_SEND_METHOD: &str = "akra/dots/send";
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

fn respond(dots: &dyn DotsPort, id: Value, method: &str, text: &str) -> String {
    let result = match method {
        DOTS_LIST_METHOD => dots
            .list_dots()
            .and_then(|dots| Ok(json!({ "dots": serde_json::to_value(dots)? }))),
        DOTS_MESSAGES_METHOD => {
            let Some(params) = params::<MessagesParams>(text) else {
                return error_response(id, JSON_RPC_INVALID_PARAMS, "`roomId` is required");
            };
            dots.room_messages(&params.room_id)
                .and_then(|messages| Ok(json!({ "messages": serde_json::to_value(messages)? })))
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
    use crate::application::port::outbound::dots_port::{DotMessage, DotSummary};
    use anyhow::{Result, anyhow};

    struct FakeDots(bool);

    static SENT: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

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

        fn room_messages(&self, room_id: &str) -> Result<Vec<DotMessage>> {
            Ok(vec![DotMessage {
                id: format!("{room_id}-m1"),
                created_at: "2026-11-14T01:00:00Z".into(),
                author: "Mika".into(),
                from_dot: true,
                text: "hi".into(),
            }])
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
}
