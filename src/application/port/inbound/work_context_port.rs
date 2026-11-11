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
    /// 이 스레드가 읽은 뒤에 문서가 바뀌었다 (plan 22 2단계). 버전을 확인한 적이 없으면 `false` 이다.
    pub changed: bool,
    /// 문서의 버전을 마지막으로 확인한 시각 (RFC 3339) 이다. 확인할 수 없는 종류이거나 아직 확인 전이면 없다.
    pub version_checked_at: Option<String>,
}

pub trait WorkContextPort: Send + Sync {
    /// 스레드의 새 기록에서 외부 문서를 찾아 장부에 적고, 그 스레드의 문서 전부를 돌려준다.
    fn thread_resources(&self, thread_id: &str) -> Result<Vec<ThreadResource>>;

    /// 그 스레드의 문서가 바뀌었는지 외부 서비스에 확인한 뒤 문서 전부를 돌려준다. 최근에 확인한 문서는 다시
    /// 묻지 않는다. 서비스에 닿지 않는 문서는 이전 값을 그대로 둔다.
    fn refresh_thread_resources(&self, thread_id: &str) -> Result<Vec<ThreadResource>>;

    /// 스레드가 그 문서의 지금 버전을 읽었다고 적고, 문서 전부를 돌려준다.
    fn mark_thread_resource_read(
        &self,
        thread_id: &str,
        resource_kind: &str,
        resource_key: &str,
    ) -> Result<Vec<ThreadResource>>;
}
