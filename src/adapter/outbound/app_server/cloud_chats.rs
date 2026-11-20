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

mod http;
mod payload;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::anyhow;

use super::cloud_upstream::{CloudCredentials, load_credentials};
use crate::application::port::outbound::chats_port::{
    ChatConversation, ChatSummary, ChatsError, ChatsPort, ChatsRead,
};
use payload::{is_safe_conversation_id, parse_chat_list, parse_conversation};

// 첫 페이지 크기는 공식 앱과 같은 28개이다.
const LIST_URL: &str = "https://chatgpt.com/backend-api/conversations?offset=0&limit=28&order=updated&is_archived=false&hide_snorlax=false";
const CONVERSATION_URL_PREFIX: &str = "https://chatgpt.com/backend-api/conversation/";
const MAX_CACHED_CONVERSATIONS: usize = 32;

#[derive(Clone, Copy)]
enum Query<'a> {
    List,
    Read(&'a str),
}

impl Query<'_> {
    fn url(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::List => LIST_URL.into(),
            Self::Read(id) => format!("{CONVERSATION_URL_PREFIX}{id}").into(),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::List => "chats list",
            Self::Read(_) => "chat read",
        }
    }
}

#[derive(Default)]
struct ChatsCache {
    account_id: String,
    list: Option<Vec<ChatSummary>>,
    reads: VecDeque<ChatConversation>,
    retry_after: Option<(Instant, Duration)>,
}

impl ChatsCache {
    fn select_account(&mut self, account_id: &str) {
        if self.account_id != account_id {
            *self = Self {
                account_id: account_id.to_owned(),
                ..Self::default()
            };
        }
    }

    fn fetch(
        &mut self,
        fetch: impl FnOnce() -> Result<String, ChatsError>,
        now: impl Fn() -> Instant,
    ) -> Result<String, ChatsError> {
        if let Some((received_at, delay)) = self.retry_after {
            let remaining = delay.saturating_sub(now().saturating_duration_since(received_at));
            if !remaining.is_zero() {
                return Err(ChatsError::RateLimited {
                    retry_after_seconds: Some(whole_seconds_rounded_up(remaining)),
                });
            }
            self.retry_after = None;
        }
        let result = fetch();
        if let Err(ChatsError::RateLimited {
            retry_after_seconds: Some(seconds),
        }) = result
        {
            self.retry_after = Some((now(), Duration::from_secs(seconds)));
        }
        result
    }

    fn remember(&mut self, conversation: ChatConversation) {
        self.reads.retain(|cached| cached.id != conversation.id);
        self.reads.push_front(conversation);
        self.reads.truncate(MAX_CACHED_CONVERSATIONS);
    }
}

fn whole_seconds_rounded_up(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
}

fn last_good<T: Clone>(
    value: Option<&T>,
    retry_after_seconds: Option<u64>,
) -> Result<ChatsRead<T>, ChatsError> {
    value
        .cloned()
        .map(|value| ChatsRead::last_good(value, retry_after_seconds))
        .ok_or(ChatsError::RateLimited {
            retry_after_seconds,
        })
}

pub(crate) struct CloudChatsRestAdapter {
    auth_path: PathBuf,
    // 믿을 수 있는 curl 의 절대 경로이다.
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

    fn cache_for_current_account(
        &self,
    ) -> Result<(MutexGuard<'_, ChatsCache>, CloudCredentials), ChatsError> {
        // 직렬화하면 앞선 요청의 429 를 뒤 요청의 서비스 호출 전에 반영할 수 있다.
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("the chats cache lock is poisoned"))?;
        let credentials = load_credentials(&self.auth_path)?;
        cache.select_account(&credentials.account_id);
        Ok((cache, credentials))
    }
}

impl ChatsPort for CloudChatsRestAdapter {
    fn list_chats(&self) -> Result<ChatsRead<Vec<ChatSummary>>, ChatsError> {
        let (mut cache, credentials) = self.cache_for_current_account()?;
        match cache.fetch(|| self.get(Query::List, &credentials), Instant::now) {
            Ok(body) => {
                let chats = parse_chat_list(&body)?;
                cache.list = Some(chats.clone());
                Ok(ChatsRead::fresh(chats))
            }
            Err(ChatsError::RateLimited {
                retry_after_seconds,
            }) => last_good(cache.list.as_ref(), retry_after_seconds),
            Err(error) => Err(error),
        }
    }

    fn read_chat(&self, id: &str) -> Result<ChatsRead<ChatConversation>, ChatsError> {
        if !is_safe_conversation_id(id) {
            return Err(ChatsError::Other(anyhow!(
                "the conversation id is not valid"
            )));
        }
        let (mut cache, credentials) = self.cache_for_current_account()?;
        match cache.fetch(|| self.get(Query::Read(id), &credentials), Instant::now) {
            Ok(body) => {
                let conversation = parse_conversation(id, &body)?;
                cache.remember(conversation.clone());
                Ok(ChatsRead::fresh(conversation))
            }
            Err(ChatsError::RateLimited {
                retry_after_seconds,
            }) => last_good(
                cache.reads.iter().find(|cached| cached.id == id),
                retry_after_seconds,
            ),
            Err(error) => Err(error),
        }
    }
}
