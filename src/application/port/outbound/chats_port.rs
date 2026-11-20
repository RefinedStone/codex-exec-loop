// chats port 는 사용자의 ChatGPT 개인 대화를 읽는 능력이다 (D-19, plan 25). dots 와 같은 토큰을 쓰지만 다른
// 경로이다: dot 은 `/tbo`·메시지 방, chat 은 `/backend-api/conversations`·`/backend-api/conversation/{id}`.
// 공식 Codex 데스크톱 앱이 ChatGPT 기록을 읽는 것과 같은 요청이다. 읽기만 한다. 보내기는 없다.
use serde::Serialize;

/// 사이드바에 보일 대화 하나이다. 외부 서비스의 필드 이름은 adapter 가 이 모양으로 옮긴다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSummary {
    pub id: String,
    /// 서비스가 준 제목이다. 없으면 기본 글자이다.
    pub title: String,
}

/// 대화 한 건의 메시지 하나이다. 걸러진 뒤의 보일 메시지이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    /// 보낸 쪽이다 (`user` 또는 `assistant`). 그 밖의 역할은 걸러진다.
    pub role: String,
    pub text: String,
}

/// 대화 한 건의 전체이다. 메시지는 오래된 것부터 놓는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConversation {
    pub id: String,
    pub title: String,
    pub messages: Vec<ChatMessage>,
}

/// chats 조회의 오류이다. 제한(HTTP 429)과 그 밖을 나눈다. 서비스는 계정 단위 기록 조회 제한을 걸고, 공식 앱도
/// 같은 제한을 쓴다. 제한은 재시도까지의 시간을 담아 클라이언트가 백오프한다.
#[derive(Debug)]
pub enum ChatsError {
    /// 서비스가 기록 조회를 제한했다. `retry_after_seconds` 는 서비스가 준 `Retry-After` 이다 (없을 수 있다).
    RateLimited { retry_after_seconds: Option<u64> },
    /// 그 밖의 실패이다 (연결, 인증, 파싱 등).
    Other(anyhow::Error),
}

impl std::fmt::Display for ChatsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatsError::RateLimited {
                retry_after_seconds,
            } => match retry_after_seconds {
                Some(seconds) => write!(
                    formatter,
                    "ChatGPT limited history reads; retry after {seconds} seconds"
                ),
                None => write!(formatter, "ChatGPT limited history reads; retry later"),
            },
            ChatsError::Other(error) => write!(formatter, "{error:#}"),
        }
    }
}

impl std::error::Error for ChatsError {}

impl From<anyhow::Error> for ChatsError {
    fn from(error: anyhow::Error) -> Self {
        ChatsError::Other(error)
    }
}

/// 서비스가 지금 기록 조회를 제한하고 있다. 값은 제한 전에 마지막으로 읽은 것이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatsLimited {
    /// 서비스가 준 `Retry-After` 이다 (없을 수 있다).
    pub retry_after_seconds: Option<u64>,
}

/// 조회의 답이다. `limited` 가 있으면 `value` 는 지금 값이 아닐 수 있다: 클라이언트는 그것을 보이고 백오프한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatsRead<T> {
    pub value: T,
    pub limited: Option<ChatsLimited>,
}

impl<T> ChatsRead<T> {
    /// 서비스에서 방금 읽은 값이다.
    pub fn fresh(value: T) -> Self {
        Self {
            value,
            limited: None,
        }
    }

    /// 제한 때문에 돌려주는 마지막 성공이다.
    pub fn last_good(value: T, retry_after_seconds: Option<u64>) -> Self {
        Self {
            value,
            limited: Some(ChatsLimited {
                retry_after_seconds,
            }),
        }
    }
}

pub trait ChatsPort: Send + Sync {
    /// 최근 대화 목록이다. 서비스가 주는 순서(최신 먼저)를 그대로 둔다. 첫 페이지만 읽는다. 읽기만 한다.
    fn list_chats(&self) -> Result<ChatsRead<Vec<ChatSummary>>, ChatsError>;

    /// 대화 한 건의 메시지이다. 오래된 것부터 놓는다. 읽기만 한다.
    fn read_chat(&self, id: &str) -> Result<ChatsRead<ChatConversation>, ChatsError>;
}
