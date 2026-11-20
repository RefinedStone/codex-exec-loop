//! 사용자의 ChatGPT 개인 대화를 ChatGPT 백엔드 REST 로 읽는다 (D-19, plan 25). dot 목록(`cloud_dots.rs`)과 같은
//! 토큰·curl 틀을 쓰지만 경로와 신원 헤더가 다르다. 공식 Codex 데스크톱 앱이 ChatGPT 기록을 읽는 것과 같은 요청이다:
//! 목록 `GET /backend-api/conversations`, 한 건 `GET /backend-api/conversation/{id}`. 앱은 이 경로에
//! `X-OpenAI-Product-Sku` 를 붙이지 않는다. 읽기만 한다. 보내기는 없다.
//!
//! 제한: 서비스는 계정 단위 기록 조회 제한(HTTP 429)을 건다. 공식 앱도 같은 제한을 쓰므로 토큰·헤더를 똑같이 맞춰도
//! 사라지지 않는다 (2026-11-20 실계정 확인). 그래서 429 는 `ChatsError::RateLimited` 로 재시도 시각을 올려
//! 클라이언트가 백오프하게 하고, 한 번이라도 성공했으면 마지막 성공을 그대로 돌려준다(공식 앱이 캐시를 보이는 것과 같다).
//!
//! 인증은 클라우드 다리와 같은 `~/.codex/auth.json` 의 토큰이다. 요청은 `curl -q --config -` 로 보낸다. 토큰은
//! stdin 의 config 로만 넘기므로 argv·디스크·로그에 남지 않는다. 오류 글자에는 응답 본문을 넣지 않는다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context, anyhow};
use serde::Deserialize;

use super::cloud_upstream::{CloudCredentials, default_user_agent, load_credentials};
use crate::application::port::outbound::chats_port::{
    ChatConversation, ChatMessage, ChatSummary, ChatsError, ChatsPort,
};
use crate::subprocess;

/// 최근 대화 목록이다. `limit=28` 은 공식 앱의 첫 페이지 크기이다. 최신순이다.
const LIST_URL: &str = "https://chatgpt.com/backend-api/conversations?offset=0&limit=28&order=updated&is_archived=false&hide_snorlax=false";
const CONVERSATION_URL_PREFIX: &str = "https://chatgpt.com/backend-api/conversation/";
const MAX_CONVERSATION_ID_LENGTH: usize = 128;
/// 한 대화에서 보일 메시지의 상한이다. 아주 긴 대화에서 메모리를 막는다.
const MAX_CONVERSATION_MESSAGES: usize = 4_000;
const MAX_ERROR_DETAIL_CHARS: usize = 200;
const CONNECT_TIMEOUT_SECONDS: u32 = 10;
const MAX_TIME_SECONDS: u32 = 20;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;

/// 캐시에 두는 대화의 상한이다. 넘으면 대화 캐시를 비운다. Gateway 는 오래 실행된다.
const MAX_CACHED_CONVERSATIONS: usize = 32;

/// 마지막으로 성공한 조회이다. 제한(429) 때 그대로 돌려준다. 계정 하나의 것이다: 로그인 계정이 바뀌면 비운다.
/// 안 그러면 이전 계정의 개인 대화가 새 계정의 답으로 나간다.
#[derive(Default)]
struct ChatsCache {
    account_id: String,
    list: Option<Vec<ChatSummary>>,
    reads: HashMap<String, ChatConversation>,
}

#[derive(Clone, Copy)]
struct ChatsHttp<'a> {
    status: u16,
    retry_after_seconds: Option<u64>,
    body: &'a str,
}

pub(crate) struct CloudChatsRestAdapter {
    auth_path: PathBuf,
    /// 믿을 수 있는 curl 의 절대 경로이다. 찾지 못했으면 그 이유이다.
    curl: std::result::Result<PathBuf, String>,
    cache: Mutex<ChatsCache>,
}

impl CloudChatsRestAdapter {
    pub(crate) fn new(auth_path: PathBuf) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let curl = crate::trusted_executable::resolve_native_from_current_path("curl", &cwd)
            .map_err(|error| error.to_string());
        Self {
            auth_path,
            curl,
            cache: Mutex::new(ChatsCache::default()),
        }
    }
}

impl ChatsPort for CloudChatsRestAdapter {
    fn list_chats(&self) -> Result<Vec<ChatSummary>, ChatsError> {
        let credentials = load_credentials(&self.auth_path)?;
        match self.get(LIST_URL, &credentials, "chats list") {
            Ok(body) => {
                let chats = parse_chat_list(&body)?;
                if let Some(mut cache) = self.cache_for(&credentials.account_id) {
                    cache.list = Some(chats.clone());
                }
                Ok(chats)
            }
            // 한 번이라도 읽었으면 마지막 성공을 돌려준다. 첫 조회부터 제한이면 그 오류를 올린다.
            Err(ChatsError::RateLimited {
                retry_after_seconds,
            }) => match self
                .cache_for(&credentials.account_id)
                .and_then(|cache| cache.list.clone())
            {
                Some(list) => Ok(list),
                None => Err(ChatsError::RateLimited {
                    retry_after_seconds,
                }),
            },
            Err(error) => Err(error),
        }
    }

    fn read_chat(&self, id: &str) -> Result<ChatConversation, ChatsError> {
        if !is_safe_conversation_id(id) {
            return Err(ChatsError::Other(anyhow!(
                "the conversation id is not valid"
            )));
        }
        let url = format!("{CONVERSATION_URL_PREFIX}{id}");
        let credentials = load_credentials(&self.auth_path)?;
        match self.get(&url, &credentials, "chat read") {
            Ok(body) => {
                let conversation = parse_conversation(id, &body)?;
                if let Some(mut cache) = self.cache_for(&credentials.account_id) {
                    if cache.reads.len() >= MAX_CACHED_CONVERSATIONS
                        && !cache.reads.contains_key(id)
                    {
                        cache.reads.clear();
                    }
                    cache.reads.insert(id.to_string(), conversation.clone());
                }
                Ok(conversation)
            }
            Err(ChatsError::RateLimited {
                retry_after_seconds,
            }) => match self
                .cache_for(&credentials.account_id)
                .and_then(|cache| cache.reads.get(id).cloned())
            {
                Some(conversation) => Ok(conversation),
                None => Err(ChatsError::RateLimited {
                    retry_after_seconds,
                }),
            },
            Err(error) => Err(error),
        }
    }
}

impl CloudChatsRestAdapter {
    /// 그 계정의 캐시를 잠근다. 다른 계정의 캐시가 남아 있으면 먼저 비운다.
    fn cache_for(&self, account_id: &str) -> Option<MutexGuard<'_, ChatsCache>> {
        let mut cache = self.cache.lock().ok()?;
        if cache.account_id != account_id {
            *cache = ChatsCache {
                account_id: account_id.to_string(),
                ..ChatsCache::default()
            };
        }
        Some(cache)
    }

    /// 인증된 GET 하나를 보내고 본문을 돌려준다. 429 는 `RateLimited` 이다. `label` 은 오류 글자에만 쓴다.
    /// 자격증명은 부르는 쪽이 한 번 읽어 넘긴다: 요청을 보낸 계정과 캐시의 계정이 같아야 한다.
    fn get(
        &self,
        url: &str,
        credentials: &CloudCredentials,
        label: &str,
    ) -> Result<String, ChatsError> {
        let curl = match &self.curl {
            Ok(path) => path,
            Err(error) => {
                return Err(ChatsError::Other(anyhow!(
                    "trusted curl executable could not be pinned for chats: {error}"
                )));
            }
        };
        let config = build_curl_config(url, credentials, &default_user_agent());

        let mut command = Command::new(curl);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        crate::trusted_executable::configure_credential_command_environment(
            &mut command,
            &cwd,
            false,
        )?;
        // `-q` 는 argv[1] 이어야 한다. 아니면 curl 이 ~/.curlrc 를 먼저 읽어 trace·proxy 로 토큰이 샐 수 있다.
        command.args(["-q", "--config", "-"]);
        let output = subprocess::command_output_with_input_timeout_and_limits(
            &mut command,
            &format!("curl -q --config - {label}"),
            config.as_bytes(),
            Duration::from_secs(u64::from(MAX_TIME_SECONDS) + 1),
            MAX_RESPONSE_BYTES,
            MAX_ERROR_BYTES,
        )
        .with_context(|| format!("failed to wait for curl during the {label} request"))?;
        if !output.status.success() {
            // 연결·시간 초과 같은 curl 자체의 실패이다. stderr 는 넣지 않는다.
            return Err(ChatsError::Other(anyhow!(
                "the {label} request failed ({})",
                output.status
            )));
        }
        let stdout = String::from_utf8(output.stdout)
            .with_context(|| format!("the {label} response was not utf-8"))?;
        let response = split_http_response(&stdout)
            .with_context(|| format!("the {label} response had no HTTP status"))?;
        if response.status == 429 {
            return Err(ChatsError::RateLimited {
                retry_after_seconds: response.retry_after_seconds,
            });
        }
        if response.status >= 400 {
            return Err(ChatsError::Other(anyhow!(
                "{}",
                http_error(label, response.status, response.body)
            )));
        }
        Ok(response.body.to_string())
    }
}

/// curl config 이다. stdin 으로만 넘긴다. 헤더는 공식 앱이 기록을 읽을 때와 같다 (`X-OpenAI-Product-Sku` 는 빼고).
/// 끝줄은 `본문 + "\n" + Retry-After + "\n" + 상태 코드` 이다 (`write-out`).
fn build_curl_config(url: &str, credentials: &CloudCredentials, user_agent: &str) -> String {
    let header = |name: &str, value: &str| {
        format!(
            "header = \"{}: {}\"\n",
            name,
            escape_curl_config_value(value)
        )
    };
    let mut config = format!(
        "silent\nshow-error\nwrite-out = \"\\n%header{{retry-after}}\\n%{{http_code}}\"\nmax-time = {MAX_TIME_SECONDS}\nconnect-timeout = {CONNECT_TIMEOUT_SECONDS}\nrequest = \"GET\"\nurl = \"{}\"\n",
        escape_curl_config_value(url),
    );
    config.push_str(&header(
        "Authorization",
        &format!("Bearer {}", credentials.access_token),
    ));
    config.push_str(&header("ChatGPT-Account-Id", &credentials.account_id));
    config.push_str(&header("originator", "Codex Desktop"));
    config.push_str(&header("User-Agent", user_agent));
    config
}

/// curl config 의 따옴표 값 안에서 뜻이 있는 글자만 escape 한다.
fn escape_curl_config_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// curl 의 출력에서 마지막 두 줄(Retry-After, 상태 코드)을 떼고 본문을 남긴다. 본문은 줄바꿈을 담을 수 있으므로
/// 오른쪽에서 두 번 나눈다. Retry-After 헤더가 없으면 빈 줄이다.
fn split_http_response(stdout: &str) -> Option<ChatsHttp<'_>> {
    let (rest, status_line) = stdout.rsplit_once('\n')?;
    let (body, retry_after_line) = rest.rsplit_once('\n')?;
    Some(ChatsHttp {
        status: status_line.trim().parse().ok()?,
        retry_after_seconds: retry_after_line.trim().parse().ok(),
        body,
    })
}

/// HTTP 오류의 글자이다. 서비스가 준 짧은 이유(`detail` 글자)만 넣는다. 그 밖의 본문은 넣지 않는다.
fn http_error(label: &str, status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("detail")?.as_str().map(str::to_string));
    match detail {
        Some(detail) => {
            let detail: String = detail.chars().take(MAX_ERROR_DETAIL_CHARS).collect();
            format!("the {label} request failed (HTTP {status}): {detail}")
        }
        None => format!("the {label} request failed (HTTP {status})"),
    }
}

/// 대화 id 는 주소의 한 조각으로 들어간다. 글자·숫자·`-`·`_` 만 받는다. 다른 경로나 query 를 만들 수 없다.
fn is_safe_conversation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_CONVERSATION_ID_LENGTH
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[derive(Deserialize)]
struct ListResponse {
    #[serde(default)]
    items: Vec<ListItemResponse>,
}

/// 서비스의 대화 하나이다. 쓰는 필드만 읽는다 (경로·필드가 자주 바뀐다).
#[derive(Deserialize)]
struct ListItemResponse {
    id: String,
    #[serde(default)]
    title: Option<String>,
}

// shortcut: 첫 페이지만 읽는다 (`offset`/`cursor` 무시). 오래된 대화가 필요해지면 `offset` 으로 페이지를 넘긴다.
fn parse_chat_list(body: &str) -> Result<Vec<ChatSummary>, ChatsError> {
    let response: ListResponse =
        serde_json::from_str(body).context("failed to parse the chats list response")?;
    Ok(response
        .items
        .into_iter()
        .map(|item| ChatSummary {
            title: non_empty_title(item.title),
            id: item.id,
        })
        .collect())
}

#[derive(Deserialize)]
struct ConversationResponse {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    current_node: Option<String>,
    #[serde(default)]
    mapping: HashMap<String, MappingNode>,
}

#[derive(Deserialize)]
struct MappingNode {
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    message: Option<NodeMessage>,
}

#[derive(Deserialize)]
struct NodeMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    author: Option<MessageAuthor>,
    #[serde(default)]
    content: Option<MessageContent>,
    #[serde(default)]
    metadata: Option<MessageMetadata>,
}

#[derive(Deserialize)]
struct MessageAuthor {
    #[serde(default)]
    role: Option<String>,
}

#[derive(Deserialize)]
struct MessageMetadata {
    #[serde(default)]
    is_visually_hidden_from_conversation: bool,
}

#[derive(Deserialize)]
struct MessageContent {
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    parts: Vec<serde_json::Value>,
}

/// 대화 응답을 보일 메시지로 옮긴다. `current_node` 에서 `parent` 를 따라 뿌리까지 올라간 뒤 뒤집어 오래된 것부터
/// 놓는다 (공식 앱과 같은 경로이다). 사람(`user`)과 에이전트(`assistant`)의 글 메시지만 남긴다: 숨긴 메시지, 도구
/// 호출, 시스템 메시지, 빈 글은 뺀다. 글은 `content.parts` 의 문자열만 잇는다.
fn parse_conversation(id: &str, body: &str) -> Result<ChatConversation, ChatsError> {
    let response: ConversationResponse =
        serde_json::from_str(body).context("failed to parse the chat conversation response")?;
    let mapping = &response.mapping;

    // current_node 에서 parent 로 올라가며 노드를 모은다. 순환이 있어도 노드 수를 넘으면 멈춘다.
    let mut ordered_leaf_first: Vec<&MappingNode> = Vec::new();
    let mut cursor = response.current_node.clone();
    let mut guard = 0usize;
    while let Some(node_id) = cursor {
        let Some(node) = mapping.get(&node_id) else {
            break;
        };
        ordered_leaf_first.push(node);
        cursor = node.parent.clone();
        guard += 1;
        if guard > mapping.len() {
            break;
        }
    }

    let mut messages: Vec<ChatMessage> = ordered_leaf_first
        .into_iter()
        .rev()
        .filter_map(|node| node.message.as_ref())
        .filter_map(visible_chat_message)
        .take(MAX_CONVERSATION_MESSAGES)
        .collect();
    // id 가 없는 메시지는 위에서 걸러진다. 번호가 섞이지 않도록 순서를 유지한다.
    messages.shrink_to_fit();

    Ok(ChatConversation {
        id: id.to_string(),
        title: non_empty_title(response.title),
        messages,
    })
}

/// 보일 메시지이면 옮기고, 아니면 없다. 역할은 `user`/`assistant` 만, 종류는 글만, 숨긴 것은 뺀다.
fn visible_chat_message(message: &NodeMessage) -> Option<ChatMessage> {
    if message
        .metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_visually_hidden_from_conversation)
    {
        return None;
    }
    let role = message
        .author
        .as_ref()
        .and_then(|author| author.role.clone())?;
    if role != "user" && role != "assistant" {
        return None;
    }
    let content = message.content.as_ref()?;
    // content_type 이 없으면 글로 본다. 그 밖(이미지, 도구 결과 등)은 뺀다.
    if let Some(content_type) = content.content_type.as_deref()
        && content_type != "text"
    {
        return None;
    }
    let text = content
        .parts
        .iter()
        .filter_map(|part| part.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return None;
    }
    Some(ChatMessage {
        id: message.id.clone()?,
        role,
        text,
    })
}

fn non_empty_title(title: Option<String>) -> String {
    match title {
        Some(title) if !title.trim().is_empty() => title,
        _ => "(제목 없음)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_body_retry_after_and_status() {
        let stdout = "{\"a\":1}\nline2\n15\n429";
        let parsed = split_http_response(stdout).expect("parses");
        assert_eq!(parsed.status, 429);
        assert_eq!(parsed.retry_after_seconds, Some(15));
        assert_eq!(parsed.body, "{\"a\":1}\nline2");
    }

    #[test]
    fn splits_when_retry_after_header_is_absent() {
        let stdout = "{}\n\n200";
        let parsed = split_http_response(stdout).expect("parses");
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.retry_after_seconds, None);
        assert_eq!(parsed.body, "{}");
    }

    #[test]
    fn parses_the_conversation_list() {
        let body =
            r#"{"items":[{"id":"c-1","title":"첫 대화"},{"id":"c-2","title":""},{"id":"c-3"}]}"#;
        let chats = parse_chat_list(body).expect("parses");
        assert_eq!(chats.len(), 3);
        assert_eq!(chats[0].id, "c-1");
        assert_eq!(chats[0].title, "첫 대화");
        assert_eq!(chats[1].title, "(제목 없음)");
        assert_eq!(chats[2].title, "(제목 없음)");
    }

    #[test]
    fn walks_the_mapping_oldest_first_and_keeps_user_and_assistant_text() {
        let body = r#"{
            "title":"T",
            "current_node":"n3",
            "mapping":{
                "root":{"parent":null,"message":null},
                "n1":{"parent":"root","message":{"id":"m1","author":{"role":"user"},"content":{"content_type":"text","parts":["hello"]}}},
                "n2":{"parent":"n1","message":{"id":"m2","author":{"role":"assistant"},"content":{"content_type":"text","parts":["hi ","there"]}}},
                "n3":{"parent":"n2","message":{"id":"m3","author":{"role":"tool"},"content":{"content_type":"text","parts":["ignored"]}}}
            }
        }"#;
        let conversation = parse_conversation("conv-x", body).expect("parses");
        assert_eq!(conversation.id, "conv-x");
        assert_eq!(conversation.title, "T");
        assert_eq!(conversation.messages.len(), 2);
        assert_eq!(conversation.messages[0].role, "user");
        assert_eq!(conversation.messages[0].text, "hello");
        assert_eq!(conversation.messages[1].role, "assistant");
        assert_eq!(conversation.messages[1].text, "hi \nthere");
    }

    #[test]
    fn drops_hidden_and_empty_messages() {
        let body = r#"{
            "current_node":"n2",
            "mapping":{
                "n1":{"parent":null,"message":{"id":"m1","author":{"role":"assistant"},"content":{"content_type":"text","parts":[""]},"metadata":{}}},
                "n2":{"parent":"n1","message":{"id":"m2","author":{"role":"user"},"content":{"content_type":"text","parts":["visible"]},"metadata":{"is_visually_hidden_from_conversation":true}}}
            }
        }"#;
        let conversation = parse_conversation("c", body).expect("parses");
        assert!(conversation.messages.is_empty());
    }

    #[test]
    fn rejects_an_unsafe_conversation_id() {
        assert!(is_safe_conversation_id(
            "6b001ae7-5a64-83ee-a0dd-4379a74c8a31"
        ));
        assert!(!is_safe_conversation_id("../secrets"));
        assert!(!is_safe_conversation_id("a/b"));
        assert!(!is_safe_conversation_id(""));
    }

    #[test]
    fn the_cache_of_one_account_is_not_served_to_another() {
        let adapter = CloudChatsRestAdapter::new(PathBuf::from("unused-auth.json"));
        {
            let mut cache = adapter.cache_for("account-a").expect("cache locks");
            cache.list = Some(vec![ChatSummary {
                id: "c-1".to_string(),
                title: "A 의 대화".to_string(),
            }]);
            cache.reads.insert(
                "c-1".to_string(),
                ChatConversation {
                    id: "c-1".to_string(),
                    title: "A 의 대화".to_string(),
                    messages: Vec::new(),
                },
            );
        }
        // 같은 계정은 캐시를 다시 본다.
        assert!(
            adapter
                .cache_for("account-a")
                .expect("locks")
                .list
                .is_some()
        );
        // 로그인 계정이 바뀌면 이전 계정의 목록과 대화가 없다.
        let cache = adapter.cache_for("account-b").expect("cache locks");
        assert!(cache.list.is_none());
        assert!(cache.reads.is_empty());
    }

    /// 실계정 읽기 확인이다 (조회만). 네트워크와 실토큰이 필요하므로 `#[ignore]` 이다. 개수만 찍는다.
    /// 수동 실행: `cargo test --lib cloud_chats::tests::live_chat_list -- --ignored --nocapture`.
    #[test]
    #[ignore = "real ChatGPT account and network; run manually"]
    fn live_chat_list() {
        let auth_path = super::super::cloud_upstream::codex_auth_path().expect("auth path");
        match CloudChatsRestAdapter::new(auth_path).list_chats() {
            Ok(chats) => println!("chats: {}", chats.len()),
            Err(ChatsError::RateLimited {
                retry_after_seconds,
            }) => println!("rate limited; retry after {retry_after_seconds:?}"),
            Err(error) => panic!("list failed: {error}"),
        }
    }
}
