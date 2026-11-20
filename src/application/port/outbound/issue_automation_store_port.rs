//! 이슈 자동화 장부의 영속 경계다 (#2536).
//! 원문이나 임의 JSON을 받지 않는다. 공유본, 원본의 필드 해시, 도메인 상태만 저장한다.
//! workspace_dir는 composition이 정한 대표 checkout의 절대 경로이며 기록의 project scope와 같아야 한다.
use anyhow::Result;

use super::issue_source_port::{IssueDiscoveryPage, IssueDiscoveryRequest};
use crate::domain::issue_automation::collection::{
    IssueCollectionFailure, IssueCollectionFence, IssueCollectionMode, IssueCollectionPage,
    IssueCollectionQuery, IssueResyncBatch, StoredIssueCollection,
};
use crate::domain::issue_automation::source::IssueSourceRead;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCollectionModeSaveResult {
    Saved {
        state: Box<StoredIssueCollection>,
        changed: bool,
    },
    Conflict {
        current: Option<Box<StoredIssueCollection>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionCommit {
    pub state: StoredIssueCollection,
    /// observation + legacy tracked 합집합에 전체 안정 ID가 처음 생긴 경우만 포함한다.
    pub newly_tracked: Vec<IssueReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCollectionApplyResult {
    Applied(Box<IssueCollectionCommit>),
    Discarded { current: Box<StoredIssueCollection> },
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

    fn get_collection(&self, workspace_dir: &str) -> Result<Option<StoredIssueCollection>>;

    /// 없을 때 Observe를 먼저 commit한다. 기존 Off 설정은 그대로 보존한다.
    fn ensure_collection(&self, workspace_dir: &str) -> Result<StoredIssueCollection>;

    /// None은 최초 설정 전용이다. 모드 변화는 epoch와 revision을 함께 올린다.
    /// 성공 반환은 동기 commit 뒤이며, 재시도는 바로 다음 epoch의 같은 모드까지만 인정한다.
    fn set_collection_mode(
        &self,
        workspace_dir: &str,
        expected_epoch: Option<u64>,
        mode: IssueCollectionMode,
    ) -> Result<IssueCollectionModeSaveResult>;

    /// Observe + fence + 요청 cursor가 모두 일치할 때만 페이지 전체와 다음 cursor를 commit한다.
    /// 최초 binding과 Withheld 관찰도 같은 트랜잭션에서 저장한다.
    fn apply_discovery_page(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        request: &IssueDiscoveryRequest,
        page: &IssueDiscoveryPage,
    ) -> Result<IssueCollectionApplyResult>;

    /// 같은 안정 저장소의 공개 정책/선택/페이지 크기 변경 시 호출한다.
    /// cursor가 있으면 epoch와 revision을 함께 올려 이전 응답을 폐기한다.
    /// cursor가 없으면 no-op이며 재동기화 cursor는 보존한다.
    fn reset_discovery_cursor(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
    ) -> Result<IssueCollectionApplyResult>;

    /// None은 Off 또는 오래된 fence다. 유효한 빈 대상 목록은 Some(empty batch)다.
    /// 완료 작업을 제외하고 observation-only와 legacy tracked를 안정 ID 순서로 함께 순회한다.
    fn read_resync_batch(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        limit: u16,
    ) -> Result<Option<IssueResyncBatch>>;

    /// 대상별 읽기(실패는 Unknown) 전체와 재동기화 cursor를 원자적으로 적용한다.
    fn apply_resync_batch(
        &self,
        workspace_dir: &str,
        batch: &IssueResyncBatch,
        reads: &[IssueSourceRead],
    ) -> Result<IssueCollectionApplyResult>;

    /// 관찰/cursor를 움직이지 않는 시도 결과다. 실패의 외부 원문은 받지 않는다.
    fn record_collection_attempt(
        &self,
        workspace_dir: &str,
        expected: IssueCollectionFence,
        at_unix_ms: u64,
        failure: Option<IssueCollectionFailure>,
    ) -> Result<IssueCollectionApplyResult>;

    /// 현재 관찰만 투영한다. Withheld/Unknown에 과거 snapshot을 대신 끼우지 않는다.
    fn query_collection_issues(
        &self,
        workspace_dir: &str,
        query: &IssueCollectionQuery,
    ) -> Result<IssueCollectionPage>;
}
