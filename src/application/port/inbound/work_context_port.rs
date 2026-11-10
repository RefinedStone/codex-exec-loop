use anyhow::Result;
use serde::Serialize;

// 작업 컨텍스트 inbound 계약이다 (plan 22). Gateway 의 `akra/context/thread` 와 이후의 `akra context` CLI 가 쓴다.

/// 스레드에 나온 외부 문서 하나이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResource {
    pub kind: String,
    pub key: String,
    pub url: String,
    pub title: String,
    pub source: String,
    pub first_seen_at: String,
}

pub trait WorkContextPort: Send + Sync {
    /// 스레드의 새 기록에서 외부 문서를 찾아 장부에 적고, 그 스레드의 문서 전부를 돌려준다.
    fn thread_resources(&self, thread_id: &str) -> Result<Vec<ThreadResource>>;
}
