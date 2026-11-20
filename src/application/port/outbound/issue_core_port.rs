use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::domain::issue_work::{IssueReference, IssueSharedCopy};

/*
 * Issue Core 의 일반 계약이다 (#2415): 공유본 조회와 게시. 사람 UI 와 AI 어댑터가 같은 계약을 쓴다.
 *
 * 이 계약의 값은 모두 공유본이다. 복원은 여기에 없다. 복원은 사용자 확인이 필요한 별도 진입점
 * (`evidence_vault_port`)이다.
 */

/// 이 계약의 판이다. 요청이나 응답의 뜻이 바뀌면 올린다.
pub const ISSUE_CORE_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSharedCopyRequest {
    pub issue: IssueReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSharedCopyResponse {
    pub issue: IssueReference,
    pub shared_copy: IssueSharedCopy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuePublishRequest {
    pub issue: IssueReference,
    /// 공급자에 쓸 공유본이다.
    pub shared_copy: IssueSharedCopy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuePublishResponse {
    pub issue: IssueReference,
    /// 게시 뒤에 공급자가 가진 공유본이다.
    pub published_copy: IssueSharedCopy,
}

pub trait IssueCorePort: Send + Sync {
    /// 구현이 따르는 계약의 판이다.
    fn contract_version(&self) -> u32 {
        ISSUE_CORE_CONTRACT_VERSION
    }

    /// 이슈의 공유본을 읽는다. 아무것도 고치지 않는다.
    fn read_shared_copy(&self, request: IssueSharedCopyRequest) -> Result<IssueSharedCopyResponse>;

    /// 공유본을 기존 이슈에 게시한다.
    fn publish(&self, request: IssuePublishRequest) -> Result<IssuePublishResponse>;
}
