use anyhow::Result;

use crate::domain::work_context::ExternalResourceRef;

/*
 * 작업 컨텍스트 장부이다 (plan 22). 외부 문서와, 그 문서가 어느 범위(스레드 등)에서 나왔는지를 저장한다.
 * 범위는 `(scope_kind, scope_id)` 로 적는다. 지금은 `thread` 만 쓰고, 파일 경로나 작업 범위는 같은 표에 더한다.
 */
pub const WORK_CONTEXT_SCOPE_THREAD: &str = "thread";

/// 범위 하나에서 본 문서이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkContextLink {
    pub resource: ExternalResourceRef,
    /// 어디서 나왔는지이다: `user`, `agent`, `tool`.
    pub source: String,
    /// 처음 본 시각 (RFC 3339) 이다.
    pub first_seen_at: String,
}

pub trait WorkContextStorePort: Send + Sync {
    /// 문서들을 범위에 잇는다. 이미 이어진 문서는 그대로 둔다.
    fn link_resources(
        &self,
        scope_kind: &str,
        scope_id: &str,
        source: &str,
        resources: &[ExternalResourceRef],
    ) -> Result<()>;

    /// 범위에 이어진 문서들이다. 처음 본 순서이다.
    fn links(&self, scope_kind: &str, scope_id: &str) -> Result<Vec<WorkContextLink>>;

    /// 범위에서 마지막으로 읽은 위치이다. 수집기가 같은 기록을 다시 읽지 않게 한다.
    fn scan_marker(&self, scope_kind: &str, scope_id: &str) -> Result<Option<String>>;

    fn set_scan_marker(&self, scope_kind: &str, scope_id: &str, marker: &str) -> Result<()>;
}
