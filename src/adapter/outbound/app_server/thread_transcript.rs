//! 스레드 기록에서 글을 읽는다 (plan 22). Gateway 의 upstream app-server 에 세션 하나를 열고
//! `thread/items/list` 를 새 것부터 읽는다. 이미 읽은 아이템을 만나면 멈춘다. 모델을 부르지 않는다.
//!
//! 사람과 AI 의 글, 실행한 명령, MCP 도구 인자만 읽는다. 명령 출력과 도구 결과는 읽지 않는다. 크고, 문서와
//! 무관한 주소가 많기 때문이다.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamStream,
};
use crate::application::port::outbound::thread_transcript_port::{
    ThreadTextFragment, ThreadTextPage, ThreadTranscriptPort,
};

const UPSTREAM_WEBSOCKET_URL: &str = "ws://localhost/rpc";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const PAGE_LIMIT: u64 = 100;
/// 한 번에 읽는 최대 쪽수이다. 넘으면 거기까지만 읽고, 가장 새 아이템을 읽은 위치로 남긴다.
/// shortcut: 처음 여는 아주 긴 스레드는 최근 5,000 아이템만 본다. 더 필요하면 오래된 쪽을 나누어 읽는다.
const MAX_PAGES: usize = 50;
/// 명령 출력이 든 아이템 한 쪽은 tungstenite 기본 한도(64 MiB)를 넘을 수 있다. Gateway 세션과 같은 값이다.
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 256 << 20;

type Upstream = WebSocketStream<Box<dyn GatewayUpstreamStream>>;

pub(crate) struct AppServerThreadTranscript {
    upstream: Arc<dyn GatewayUpstreamPort>,
}

impl AppServerThreadTranscript {
    pub(crate) fn new(upstream: Arc<dyn GatewayUpstreamPort>) -> Self {
        Self { upstream }
    }
}

impl ThreadTranscriptPort for AppServerThreadTranscript {
    /// 블로킹 호출이다. Gateway 는 블로킹 작업에서 부른다. upstream 연결은 이 호출의 runtime 에 묶인다.
    fn text_after(&self, thread_id: &str, after_item_id: Option<&str>) -> Result<ThreadTextPage> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to start the thread transcript reader")?;
        runtime.block_on(read_text_after(
            self.upstream.as_ref(),
            thread_id,
            after_item_id,
        ))
    }
}

async fn read_text_after(
    upstream: &dyn GatewayUpstreamPort,
    thread_id: &str,
    after_item_id: Option<&str>,
) -> Result<ThreadTextPage> {
    let stream = upstream.open_session()?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES));
    let (mut websocket, _response) =
        tokio_tungstenite::client_async_with_config(UPSTREAM_WEBSOCKET_URL, stream, Some(config))
            .await
            .context("upstream WebSocket handshake failed")?;
    request(
        &mut websocket,
        1,
        "initialize",
        json!({
            "clientInfo": { "name": "akra-work-context", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "experimentalApi": false }
        }),
    )
    .await?;
    send(
        &mut websocket,
        &json!({ "jsonrpc": "2.0", "method": "initialized" }),
    )
    .await?;

    let mut newest_first: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    for page_index in 0..MAX_PAGES {
        let mut params =
            json!({ "threadId": thread_id, "limit": PAGE_LIMIT, "sortDirection": "desc" });
        if let Some(cursor) = &cursor {
            params["cursor"] = json!(cursor);
        }
        let page = request(
            &mut websocket,
            page_index as u64 + 2,
            "thread/items/list",
            params,
        )
        .await?;
        let entries = page
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let reached_known = collect_until_known(entries, after_item_id, &mut newest_first);
        cursor = page
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_string);
        if reached_known || cursor.is_none() {
            break;
        }
    }
    let _ = websocket.close(None).await;
    Ok(text_page(newest_first))
}

/// 새 것부터 온 아이템을 모은다. 이미 읽은 아이템을 만나면 그 앞까지만 모으고 true 를 돌려준다.
fn collect_until_known(
    entries: Vec<Value>,
    known_item_id: Option<&str>,
    items: &mut Vec<Value>,
) -> bool {
    for entry in entries {
        let item = entry.get("item").cloned().unwrap_or(entry);
        if known_item_id.is_some() && item.get("id").and_then(Value::as_str) == known_item_id {
            return true;
        }
        items.push(item);
    }
    false
}

fn text_page(newest_first: Vec<Value>) -> ThreadTextPage {
    let newest_item_id = newest_first
        .first()
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let fragments = newest_first
        .iter()
        .rev()
        .filter_map(fragment_from_item)
        .collect();
    ThreadTextPage {
        fragments,
        newest_item_id,
    }
}

fn fragment_from_item(item: &Value) -> Option<ThreadTextFragment> {
    let text_field = |name: &str| item.get(name).and_then(Value::as_str).map(str::to_string);
    let (source, text) = match item.get("type").and_then(Value::as_str)? {
        "userMessage" => (
            "user",
            item.get("content")?
                .as_array()?
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "agentMessage" | "plan" => ("agent", text_field("text")?),
        "commandExecution" => ("tool", text_field("command")?),
        // 인자는 JSON 이다. 주소 찾기는 따옴표에서 끊으므로 글자 그대로 넘긴다.
        "mcpToolCall" => ("tool", item.get("arguments")?.to_string()),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    Some(ThreadTextFragment {
        source: source.to_string(),
        text,
    })
}

async fn send(websocket: &mut Upstream, message: &Value) -> Result<()> {
    websocket
        .send(Message::text(message.to_string()))
        .await
        .context("failed to send to the upstream app-server")
}

/// 요청 하나를 보내고 같은 id 의 응답을 기다린다. app-server 가 보내는 알림과 요청은 건너뛴다.
async fn request(websocket: &mut Upstream, id: u64, method: &str, params: Value) -> Result<Value> {
    send(
        websocket,
        &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
    )
    .await?;
    let wait = async {
        while let Some(message) = websocket.next().await {
            let Message::Text(text) = message.context("the upstream connection failed")? else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            if value.get("id").and_then(Value::as_u64) != Some(id) || value.get("method").is_some()
            {
                continue;
            }
            if let Some(error) = value.get("error") {
                let detail = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                bail!("`{method}` failed: {detail}")
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
        bail!("the upstream closed before it answered `{method}`")
    };
    tokio::time::timeout(REQUEST_TIMEOUT, wait)
        .await
        .with_context(|| format!("the upstream did not answer `{method}` in time"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(item: Value) -> Value {
        json!({ "turnId": "turn-1", "item": item })
    }

    #[test]
    fn items_stop_at_the_known_item_and_come_back_oldest_first() {
        let mut items = Vec::new();
        let reached = collect_until_known(
            vec![
                entry(
                    json!({ "type": "agentMessage", "id": "i3", "text": "done https://github.com/a/b/pull/1" }),
                ),
                entry(
                    json!({ "type": "userMessage", "id": "i2", "content": [{ "type": "text", "text": "fix it" }, { "type": "image" }] }),
                ),
                entry(json!({ "type": "agentMessage", "id": "i1", "text": "old" })),
            ],
            Some("i1"),
            &mut items,
        );
        assert!(reached);
        let page = text_page(items);
        assert_eq!(page.newest_item_id.as_deref(), Some("i3"));
        assert_eq!(
            page.fragments,
            vec![
                ThreadTextFragment {
                    source: "user".into(),
                    text: "fix it".into()
                },
                ThreadTextFragment {
                    source: "agent".into(),
                    text: "done https://github.com/a/b/pull/1".into()
                },
            ]
        );
    }

    #[test]
    fn commands_and_tool_arguments_are_read_but_outputs_are_not() {
        let mut items = Vec::new();
        let reached = collect_until_known(
            vec![
                entry(
                    json!({ "type": "mcpToolCall", "id": "i3", "arguments": { "id": "https://www.notion.so/x" }, "result": { "text": "https://secret.example" } }),
                ),
                entry(
                    json!({ "type": "commandExecution", "id": "i2", "command": "akra mcp call notion notion-fetch", "aggregatedOutput": "https://noise.example" }),
                ),
                entry(
                    json!({ "type": "reasoning", "id": "i1", "summary": ["https://ignored.example"] }),
                ),
            ],
            None,
            &mut items,
        );
        assert!(!reached);
        let page = text_page(items);
        assert_eq!(page.newest_item_id.as_deref(), Some("i3"));
        assert_eq!(page.fragments.len(), 2);
        assert_eq!(page.fragments[0].text, "akra mcp call notion notion-fetch");
        assert_eq!(
            page.fragments[1].text,
            r#"{"id":"https://www.notion.so/x"}"#
        );
        assert!(
            page.fragments
                .iter()
                .all(|fragment| fragment.source == "tool")
        );
    }

    #[test]
    fn an_empty_thread_has_no_marker() {
        assert_eq!(text_page(Vec::new()), ThreadTextPage::default());
    }
}
