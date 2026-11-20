//! 명시적인 사용자 요청으로 이슈 폼 설치를 제안하는 inbound 계약이다.
//! CLI는 이 경계만 사용하며 서비스와 저장 장부의 구현 타입을 알지 않는다.
use anyhow::Result;
use serde::Serialize;

pub trait IssueFormInstallPort: Send + Sync {
    fn install(&self, request: &IssueFormInstallRequest) -> Result<IssueFormInstallResponse>;
}

#[derive(Debug, Clone)]
pub struct IssueFormInstallRequest {
    pub workspace_dir: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueFormInstallStatus {
    AlreadyInstalled,
    PullRequestCreated,
    PullRequestFound,
    NeedsConfirmation,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueFormInstallReason {
    ExistingFormDiffers,
    BaseChanged,
    BranchChanged,
    PushOutcomeUnknown,
    PullRequestOutcomeUnknown,
    PullRequestMismatch,
    ConcurrentChange,
}

/// 클라이언트에 보여 주는 설치 PR이다. 저장 장부의 DTO를 노출하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueFormInstallPullRequest {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueFormInstallResponse {
    pub status: IssueFormInstallStatus,
    pub path: String,
    pub base_branch: String,
    pub head_branch: Option<String>,
    pub base_commit_sha: Option<String>,
    pub commit_sha: Option<String>,
    pub pull_request: Option<IssueFormInstallPullRequest>,
    pub reason: Option<IssueFormInstallReason>,
}
