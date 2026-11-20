//! Gateway 의 `akra/chats/*` 처리이다 (D-19, plan 25). 사용자의 ChatGPT 개인 대화를 읽어 준다. 조회는 외부 서비스
//! 요청(블로킹)이므로 요청마다 블로킹 작업으로 돌린다. chats 가 꺼져 있으면 이 처리 자체가 없고, 요청은 모르는
//! method 로 답한다. 답은 원격 서비스의 내용이므로 기록하지 않는다 (plan 25 §3.1).

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::application::port::outbound::chats_port::{ChatsError, ChatsLimited, ChatsPort};

pub(super) const CHATS_METHOD_PREFIX: &str = "akra/chats/";
/// params 없음 -> result `{ chats: [ChatSummary], limited?, retryAfterSeconds? }`. 최신순이다.
const CHATS_LIST_METHOD: &str = "akra/chats/list";
/// params `{ id }` -> result `ChatConversation` 에 `limited?`, `retryAfterSeconds?` 를 더한 것. 메시지는 오래된
/// 것부터이다.
const CHATS_READ_METHOD: &str = "akra/chats/read";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const JSON_RPC_INVALID_PARAMS: i64 = -32602;
const CHATS_ERROR: i64 = -32000;
/// 서비스가 기록 조회를 제한했다. `data.retryAfterSeconds` 로 클라이언트가 백오프한다.
const CHATS_RATE_LIMITED: i64 = -32010;

/// 요청 하나를 블로킹 작업으로 처리하고 답을 접속의 보내기 함으로 보낸다.
pub(super) fn spawn_request(
    chats: Arc<dyn ChatsPort>,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
) {
    tokio::spawn(async move {
        let fallback_id = id.clone();
        let response =
            tokio::task::spawn_blocking(move || respond(chats.as_ref(), id, &method, &text))
                .await
                .unwrap_or_else(|_| {
                    error_response(fallback_id, CHATS_ERROR, "the chats request stopped", None)
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
struct ReadParams {
    id: String,
}

fn params<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let envelope = serde_json::from_str::<RequestEnvelope>(text).ok()?;
    serde_json::from_value(envelope.params).ok()
}

fn respond(chats: &dyn ChatsPort, id: Value, method: &str, text: &str) -> String {
    let result = match method {
        CHATS_LIST_METHOD => chats.list_chats().and_then(|read| {
            to_value(&read.value).map(|chats| with_limit(json!({ "chats": chats }), read.limited))
        }),
        CHATS_READ_METHOD => {
            let Some(params) = params::<ReadParams>(text) else {
                return error_response(id, JSON_RPC_INVALID_PARAMS, "`id` is required", None);
            };
            chats.read_chat(&params.id).and_then(|read| {
                to_value(&read.value).map(|conversation| with_limit(conversation, read.limited))
            })
        }
        _ => {
            return error_response(
                id,
                JSON_RPC_METHOD_NOT_FOUND,
                &format!("unknown gateway method `{method}`"),
                None,
            );
        }
    };
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        Err(ChatsError::RateLimited {
            retry_after_seconds,
        }) => error_response(
            id,
            CHATS_RATE_LIMITED,
            "ChatGPT limited history reads; retry later",
            retry_after_seconds,
        ),
        Err(error) => error_response(id, CHATS_ERROR, &format!("{error}"), None),
    }
}

/// 답에 제한 상태를 붙인다. `limited: true` 이면 값은 제한 전에 마지막으로 읽은 것이다. 서비스가 `Retry-After` 를
/// 줬으면 `retryAfterSeconds` 도 붙는다. 방금 읽은 값에는 아무것도 붙이지 않는다.
fn with_limit(mut result: Value, limited: Option<ChatsLimited>) -> Value {
    if let Some(limited) = limited {
        result["limited"] = json!(true);
        if let Some(seconds) = limited.retry_after_seconds {
            result["retryAfterSeconds"] = json!(seconds);
        }
    }
    result
}

/// 답을 JSON 으로 옮긴다. 우리 구조체의 직렬화는 실패하지 않지만, 혹시 실패하면 일반 오류로 돌린다.
fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, ChatsError> {
    serde_json::to_value(value).map_err(|error| {
        ChatsError::Other(anyhow::anyhow!(
            "failed to encode the chats response: {error}"
        ))
    })
}

fn error_response(id: Value, code: i64, message: &str, retry_after_seconds: Option<u64>) -> String {
    let mut error = json!({ "code": code, "message": message });
    if let Some(seconds) = retry_after_seconds {
        error["data"] = json!({ "retryAfterSeconds": seconds });
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::chats_port::{
        ChatConversation, ChatMessage, ChatSummary, ChatsRead,
    };

    struct FakeChats {
        rate_limited: bool,
    }

    fn one_chat() -> Vec<ChatSummary> {
        vec![ChatSummary {
            id: "c-1".to_string(),
            title: "첫 대화".to_string(),
        }]
    }

    fn one_conversation(id: &str) -> ChatConversation {
        ChatConversation {
            id: id.to_string(),
            title: "T".to_string(),
            messages: vec![ChatMessage {
                id: "m-1".to_string(),
                role: "user".to_string(),
                text: "hello".to_string(),
            }],
        }
    }

    impl ChatsPort for FakeChats {
        fn list_chats(&self) -> Result<ChatsRead<Vec<ChatSummary>>, ChatsError> {
            if self.rate_limited {
                return Err(ChatsError::RateLimited {
                    retry_after_seconds: Some(30),
                });
            }
            Ok(ChatsRead::fresh(one_chat()))
        }

        fn read_chat(&self, id: &str) -> Result<ChatsRead<ChatConversation>, ChatsError> {
            Ok(ChatsRead::fresh(one_conversation(id)))
        }
    }

    /// 제한 중이고, 마지막 성공을 돌려주는 adapter 이다.
    struct LastGoodChats;

    impl ChatsPort for LastGoodChats {
        fn list_chats(&self) -> Result<ChatsRead<Vec<ChatSummary>>, ChatsError> {
            Ok(ChatsRead::last_good(one_chat(), Some(38)))
        }

        fn read_chat(&self, id: &str) -> Result<ChatsRead<ChatConversation>, ChatsError> {
            Ok(ChatsRead::last_good(one_conversation(id), None))
        }
    }

    #[test]
    fn a_last_good_answer_says_that_reads_are_limited() {
        let list = value(&respond(
            &LastGoodChats,
            json!(6),
            CHATS_LIST_METHOD,
            r#"{"jsonrpc":"2.0","id":6,"method":"akra/chats/list"}"#,
        ));
        assert_eq!(list["result"]["chats"][0]["id"], json!("c-1"));
        assert_eq!(list["result"]["limited"], json!(true));
        assert_eq!(list["result"]["retryAfterSeconds"], json!(38));

        let read = value(&respond(
            &LastGoodChats,
            json!(7),
            CHATS_READ_METHOD,
            r#"{"jsonrpc":"2.0","id":7,"method":"akra/chats/read","params":{"id":"conv-9"}}"#,
        ));
        assert_eq!(read["result"]["messages"][0]["text"], json!("hello"));
        assert_eq!(read["result"]["limited"], json!(true));
        assert!(read["result"].get("retryAfterSeconds").is_none());
    }

    fn value(response: &str) -> Value {
        serde_json::from_str(response).expect("valid json response")
    }

    #[test]
    fn list_returns_chats() {
        let response = value(&respond(
            &FakeChats {
                rate_limited: false,
            },
            json!(1),
            CHATS_LIST_METHOD,
            r#"{"jsonrpc":"2.0","id":1,"method":"akra/chats/list"}"#,
        ));
        assert_eq!(response["result"]["chats"][0]["id"], json!("c-1"));
        // 방금 읽은 값에는 제한 표시가 없다.
        assert!(response["result"].get("limited").is_none());
    }

    #[test]
    fn read_requires_an_id() {
        let response = value(&respond(
            &FakeChats {
                rate_limited: false,
            },
            json!(2),
            CHATS_READ_METHOD,
            r#"{"jsonrpc":"2.0","id":2,"method":"akra/chats/read"}"#,
        ));
        assert_eq!(response["error"]["code"], json!(JSON_RPC_INVALID_PARAMS));
    }

    #[test]
    fn read_returns_the_conversation() {
        let response = value(&respond(
            &FakeChats {
                rate_limited: false,
            },
            json!(3),
            CHATS_READ_METHOD,
            r#"{"jsonrpc":"2.0","id":3,"method":"akra/chats/read","params":{"id":"conv-9"}}"#,
        ));
        assert_eq!(response["result"]["id"], json!("conv-9"));
        assert_eq!(response["result"]["messages"][0]["text"], json!("hello"));
    }

    #[test]
    fn rate_limit_carries_retry_after() {
        let response = value(&respond(
            &FakeChats { rate_limited: true },
            json!(4),
            CHATS_LIST_METHOD,
            r#"{"jsonrpc":"2.0","id":4,"method":"akra/chats/list"}"#,
        ));
        assert_eq!(response["error"]["code"], json!(CHATS_RATE_LIMITED));
        assert_eq!(response["error"]["data"]["retryAfterSeconds"], json!(30));
    }

    #[test]
    fn unknown_method_is_refused() {
        let response = value(&respond(
            &FakeChats {
                rate_limited: false,
            },
            json!(5),
            "akra/chats/nope",
            r#"{"jsonrpc":"2.0","id":5,"method":"akra/chats/nope"}"#,
        ));
        assert_eq!(response["error"]["code"], json!(JSON_RPC_METHOD_NOT_FOUND));
    }
}
