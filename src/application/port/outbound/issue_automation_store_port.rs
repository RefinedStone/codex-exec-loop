//! 이슈 자동화 장부의 영속 경계다 (#2536).
//! 원문이나 임의 JSON을 받지 않는다. 공유본, 원본의 필드 해시, 도메인 상태만 저장한다.
//! workspace_dir는 composition이 정한 대표 checkout의 절대 경로이며 기록의 project scope와 같아야 한다.
use anyhow::Result;

use crate::domain::issue_automation::{
    IssueAutomationBinding, IssueAutomationBindingId, TrackedIssue,
};
use crate::domain::issue_work::IssueReference;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIssueAutomationBinding {
    pub binding: IssueAutomationBinding,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationBindingInsertResult {
    pub binding: StoredIssueAutomationBinding,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationIssueInsertResult {
    pub issue: TrackedIssue,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAutomationBindingSaveResult {
    Saved {
        record: Box<StoredIssueAutomationBinding>,
        changed: bool,
    },
    Conflict {
        actual_revision: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAutomationIssueSaveResult {
    Saved {
        record: Box<TrackedIssue>,
        changed: bool,
    },
    Conflict {
        actual_revision: Option<u64>,
    },
}

/// 저장소 runtime의 별도 장부다. planning DB와 schema를 공유하지 않는다.
/// 같은 안정 ID의 재삽입은 기존 내용을 보존한다. 갱신은 revision CAS로만 한다.
pub trait IssueAutomationStorePort: Send + Sync {
    /// 기존 ID의 좌표는 바꾸지 않는다. 같은 좌표이면 기존 자율 수준과 revision을 그대로 반환한다.
    fn insert_binding(
        &self,
        workspace_dir: &str,
        binding: &IssueAutomationBinding,
    ) -> Result<IssueAutomationBindingInsertResult>;

    fn get_binding(
        &self,
        workspace_dir: &str,
        binding_id: &IssueAutomationBindingId,
    ) -> Result<Option<StoredIssueAutomationBinding>>;

    fn list_bindings(&self, workspace_dir: &str) -> Result<Vec<StoredIssueAutomationBinding>>;

    /// 바인딩 ID와 원격/프로젝트 좌표는 불변이다. 자율 수준을 바꾸는 허가는 상위 명령 경계가 검사한다.
    fn save_binding(
        &self,
        workspace_dir: &str,
        expected_revision: u64,
        binding: &IssueAutomationBinding,
    ) -> Result<IssueAutomationBindingSaveResult>;

    /// 신규 발견만 받는다(revision 0). 같은 전체 IssueReference가 있으면 그 행을 그대로 반환한다.
    /// 반복 수집이 상태, 표시, 공유본 또는 기존 바인딩을 초기화하지 않는다.
    fn insert_issue(
        &self,
        workspace_dir: &str,
        issue: &TrackedIssue,
    ) -> Result<IssueAutomationIssueInsertResult>;

    fn get_issue(
        &self,
        workspace_dir: &str,
        reference: &IssueReference,
    ) -> Result<Option<TrackedIssue>>;

    fn list_issues(
        &self,
        workspace_dir: &str,
        binding_id: Option<&IssueAutomationBindingId>,
    ) -> Result<Vec<TrackedIssue>>;

    /// candidate.revision은 expected_revision과 같아야 한다. 상태 변경은 도메인 전이 규칙을 다시 검사한다.
    /// 성공은 revision을 1 올리고, 내용이 같은 no-op은 유지한다.
    /// 응답 유실 재시도는 현재 revision이 expected+1이고 내용이 정확히 같을 때만 인정한다.
    /// 그보다 뒤의 동일 내용은 ABA일 수 있으므로 conflict다. 실패를 미존재나 빈 장부로 숨기지 않는다.
    fn save_issue(
        &self,
        workspace_dir: &str,
        expected_revision: u64,
        issue: &TrackedIssue,
    ) -> Result<IssueAutomationIssueSaveResult>;
}
