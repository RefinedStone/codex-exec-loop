use anyhow::Result;
use serde::{Deserialize, Serialize};

// 작업 컨텍스트 inbound 계약이다 (plan 22). Gateway 의 `akra/context/*` 와 이후의 `akra context` CLI 가 쓴다.

/// 범위 하나에 이어진 외부 문서 하나이다. 범위는 스레드, 저장소, 작업, 저장소 안 경로이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextResource {
    pub kind: String,
    pub key: String,
    pub url: String,
    pub title: String,
    pub source: String,
    pub first_seen_at: String,
    /// 이 범위가 확인한 뒤에 문서가 바뀌었다 (plan 22 2단계). 버전을 확인한 적이 없으면 `false` 이다.
    pub changed: bool,
    /// 문서의 버전을 마지막으로 확인한 시각 (RFC 3339) 이다. 확인할 수 없는 종류이거나 아직 확인 전이면 없다.
    pub version_checked_at: Option<String>,
    /// 이 범위가 확인한 버전이다. 스레드는 읽은 버전, 다른 범위는 잇던 때 확인한 버전이다.
    pub confirmed_version: Option<String>,
    /// 장부가 아는 문서의 지금 버전이다. 마지막으로 확인한 값이다.
    pub current_version: Option<String>,
}

/// 스레드가 아닌 범위이다 (A03). 모두 `cwd` 가 속한 저장소 안의 범위이다.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ContextScope {
    Repository,
    #[serde(rename_all = "camelCase")]
    Task {
        task_id: String,
    },
    /// 저장소 기준 상대 경로이다. 파일 또는 폴더이다.
    Path {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextScopeRequest {
    /// 저장소를 찾는 폴더이다. 같은 저장소의 다른 worktree 도 같은 범위가 된다.
    pub cwd: String,
    pub scope: ContextScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachContextRequest {
    pub scope: ContextScopeRequest,
    /// 문서의 주소이다. 인식기가 아는 문서 하나여야 한다.
    pub url: String,
    /// 누가 이었는지이다: `user`, `agent`, `tool`.
    pub source: String,
}

pub trait WorkContextPort: Send + Sync {
    /// 스레드의 새 기록에서 외부 문서를 찾아 장부에 적고, 그 스레드의 문서 전부를 돌려준다.
    fn thread_resources(&self, thread_id: &str) -> Result<Vec<ContextResource>>;

    /// 그 스레드의 문서가 바뀌었는지 외부 서비스에 확인한 뒤 문서 전부를 돌려준다. 최근에 확인한 문서는 다시
    /// 묻지 않는다. 서비스에 닿지 않는 문서는 이전 값을 그대로 둔다.
    fn refresh_thread_resources(&self, thread_id: &str) -> Result<Vec<ContextResource>>;

    /// 스레드가 그 문서의 지금 버전을 읽었다고 적고, 문서 전부를 돌려준다.
    fn mark_thread_resource_read(
        &self,
        thread_id: &str,
        resource_kind: &str,
        resource_key: &str,
    ) -> Result<Vec<ContextResource>>;

    /// 문서를 범위에 잇고 그 범위의 문서 전부를 돌려준다. 문서의 지금 버전을 확인해 그 범위의 확인 버전으로
    /// 적는다. 이미 이어진 문서는 처음의 출처와 확인 버전을 그대로 둔다.
    fn attach_scope_resource(&self, request: &AttachContextRequest)
    -> Result<Vec<ContextResource>>;

    /// 범위의 문서 전부이다. 장부의 값만 쓰고 외부 서비스에 묻지 않는다.
    fn scope_resources(&self, request: &ContextScopeRequest) -> Result<Vec<ContextResource>>;

    /// 범위의 버전을 기존 상한과 실패 예산 안에서 확인한다.
    fn refresh_scope_resources(
        &self,
        request: &ContextScopeRequest,
    ) -> Result<Vec<ContextResource>>;

    /// 사용자가 실제로 본 버전만 기록한다.
    fn mark_scope_resource_read(
        &self,
        request: &ContextScopeRequest,
        resource_kind: &str,
        resource_key: &str,
        observed_version: &str,
    ) -> Result<Vec<ContextResource>>;
}
