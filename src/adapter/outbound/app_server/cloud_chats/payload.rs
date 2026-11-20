//! ChatGPT history response decoding.
use std::collections::{HashMap, HashSet};

use anyhow::Context;
use serde::{Deserialize, de::IgnoredAny};

use crate::application::port::outbound::chats_port::{
    ChatConversation, ChatMessage, ChatSummary, ChatsError,
};

const MAX_CONVERSATION_ID_LENGTH: usize = 128;
pub(super) const MAX_CONVERSATION_MESSAGES: usize = 4_000;

/// Accept only characters that cannot change the conversation URL path or query.
pub(super) fn is_safe_conversation_id(id: &str) -> bool {
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
pub(super) fn parse_chat_list(body: &str) -> Result<Vec<ChatSummary>, ChatsError> {
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
    /// 받는 쪽이다. 사람에게 보이는 메시지는 `all` 이다. 도구 호출은 도구의 이름이다.
    #[serde(default)]
    recipient: Option<String>,
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
    parts: Vec<ContentPart>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ContentPart {
    Text(String),
    Other(IgnoredAny),
}

/// 대화 응답을 보일 메시지로 옮긴다. `current_node` 에서 `parent` 를 따라 뿌리까지 올라간 뒤 뒤집어 오래된 것부터
/// 놓는다 (공식 앱과 같은 경로이다). 사람(`user`)과 에이전트(`assistant`)의 글 메시지만 남긴다: 숨긴 메시지, 도구
/// 호출, 시스템 메시지, 빈 글은 뺀다. 글은 `content.parts` 의 문자열만 잇는다.
pub(super) fn parse_conversation(id: &str, body: &str) -> Result<ChatConversation, ChatsError> {
    let response: ConversationResponse =
        serde_json::from_str(body).context("failed to parse the chat conversation response")?;
    let mapping = &response.mapping;

    // 잎에서 보이는 메시지만 세어 최신 메시지를 남긴다. 순환한 노드는 두 번 읽지 않는다.
    let mut visited = HashSet::new();
    let mut cursor = response.current_node.as_deref();
    let mut messages = Vec::new();
    while let Some(node_id) = cursor {
        if !visited.insert(node_id) {
            break;
        }
        let Some(node) = mapping.get(node_id) else {
            break;
        };
        if let Some(message) = node.message.as_ref().and_then(visible_chat_message) {
            messages.push(message);
            if messages.len() == MAX_CONVERSATION_MESSAGES {
                break;
            }
        }
        cursor = node.parent.as_deref();
    }
    messages.reverse();

    Ok(ChatConversation {
        id: id.to_string(),
        title: non_empty_title(response.title),
        messages,
    })
}

/// 보일 메시지이면 옮기고, 아니면 없다. 역할은 `user`/`assistant` 만, 종류는 글(첨부가 붙은 글 포함)만, 숨긴 것과
/// 도구에게 보낸 것은 뺀다. eamonma/chatgpt-etl 의 spec.md §1.3.2/§1.4 는 내보낸 대화 656건에서
/// `all` 이 아닌 도구 수신자와 `multimodal_text` 139건의 혼합 `parts` 를 기록한다.
/// 라이브 조회는 HTTP 429 를 받아 이 필드의 모양을 확인하지 못했다 (2026-11-20).
fn visible_chat_message(message: &NodeMessage) -> Option<ChatMessage> {
    if message
        .metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_visually_hidden_from_conversation)
    {
        return None;
    }
    // 도구에게 보낸 메시지(메모리 저장, 이미지 생성 등)는 사람에게 한 답이 아니다.
    if message
        .recipient
        .as_deref()
        .is_some_and(|recipient| recipient != "all")
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
    // content_type 이 없으면 글로 본다. `multimodal_text` 는 첨부(이미지, 파일)가 붙은 글이다: 아래에서 글 조각만
    // 남는다. 그 밖(코드 실행, 도구 결과 등)은 뺀다.
    if let Some(content_type) = content.content_type.as_deref()
        && content_type != "text"
        && content_type != "multimodal_text"
    {
        return None;
    }
    let text = content
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.as_str()),
            ContentPart::Other(_) => None,
        })
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
