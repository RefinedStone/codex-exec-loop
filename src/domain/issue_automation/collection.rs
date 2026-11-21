//! Observe 수집의 영속 설정, 현재 관찰 투영과 다음 페이지 경계다 (#2538).
//! 공급자 호출, 스케줄링이나 워크플로 전이는 여기서 수행하지 않는다.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::source::{IssueDiscoveryCursor, IssueSourceFailure, IssueSourceRead};
use super::triage::IssueTriageAssessment;
use super::{
    IssueAutomationBinding, IssueAutomationBindingId, IssueAutomationError, IssueAutomationFlags,
    IssueAutomationState, MAX_ISSUE_AUTOMATION_REVISION,
};
use crate::domain::issue_work::IssueReference;

pub const MAX_ISSUE_COLLECTION_PAGE_SIZE: u16 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueCollectionMode {
    Off,
    Observe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionConfig {
    pub mode: IssueCollectionMode,
}

impl Default for IssueCollectionConfig {
    fn default() -> Self {
        Self {
            mode: IssueCollectionMode::Observe,
        }
    }
}

/// 원격 읽기를 시작할 때 복사한다. 응답 적용은 두 값이 모두 일치해야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueCollectionFence {
    pub epoch: u64,
    pub revision: u64,
}

impl IssueCollectionFence {
    pub fn validate(&self) -> Result<(), IssueAutomationError> {
        validate_revision(self.epoch)?;
        validate_revision(self.revision)?;
        if self.revision < self.epoch {
            return Err(IssueAutomationError::InvalidRevision);
        }
        Ok(())
    }
}

/// 한 재동기화 순회의 고정 상한과 마지막 처리 ID다.
/// ID의 순서는 공급자/서버/저장소/이슈/프로젝트의 BINARY 순서다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueResyncCursor {
    pub after: IssueReference,
    pub through: IssueReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIssueCollection {
    pub config: IssueCollectionConfig,
    pub epoch: u64,
    /// 수집 설정·관찰·체크포인트의 판이다. 개별 TrackedIssue 판과는 별개다.
    pub revision: u64,
    /// 최초 안정 저장소 ID가 확인되기 전에는 없다.
    pub binding_id: Option<IssueAutomationBindingId>,
    pub discovery_cursor: Option<IssueDiscoveryCursor>,
    pub resync_cursor: Option<IssueResyncCursor>,
    pub status: IssueCollectionStatus,
}

impl StoredIssueCollection {
    pub fn fence(&self) -> IssueCollectionFence {
        IssueCollectionFence {
            epoch: self.epoch,
            revision: self.revision,
        }
    }

    pub fn validate(&self) -> Result<(), IssueAutomationError> {
        self.fence().validate()?;
        if self.discovery_cursor.is_some() && self.binding_id.is_none() {
            return Err(IssueAutomationError::InvalidValue {
                field: "collection.binding_id",
            });
        }
        if let Some(cursor) = &self.resync_cursor {
            super::validate_reference(&cursor.after)?;
            super::validate_reference(&cursor.through)?;
            if cursor.after.project_scope != cursor.through.project_scope {
                return Err(IssueAutomationError::IdentityChanged);
            }
        }
        self.status.validate()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueCollectionStatus {
    pub last_attempt_at_ms: Option<u64>,
    pub last_success_at_ms: Option<u64>,
    pub last_failure: Option<IssueCollectionFailure>,
    pub consecutive_failures: u32,
}

impl IssueCollectionStatus {
    fn validate(&self) -> Result<(), IssueAutomationError> {
        for timestamp in [self.last_attempt_at_ms, self.last_success_at_ms]
            .into_iter()
            .flatten()
        {
            if timestamp > MAX_ISSUE_AUTOMATION_REVISION {
                return Err(IssueAutomationError::InvalidValue {
                    field: "collection.timestamp",
                });
            }
        }
        if (self.last_failure.is_some()) != (self.consecutive_failures > 0)
            || (self.last_success_at_ms.is_some() && self.last_attempt_at_ms.is_none())
            || (self.last_failure.is_some() && self.last_attempt_at_ms.is_none())
        {
            return Err(IssueAutomationError::InvalidValue {
                field: "collection.status",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "reason",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IssueCollectionFailure {
    Source(IssueSourceFailure),
    OriginUnavailable,
    PolicyUnavailable,
}

/// 설정과 그 설정이 가리키는 안정 저장소 바인딩을 함께 보여준다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionProjection {
    pub state: StoredIssueCollection,
    pub binding: Option<IssueAutomationBinding>,
    /// 목록·단건·이벤트가 공유하는 공개 판이다. state.revision은 수집 CAS에만 쓴다.
    pub catalog_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueResyncBatch {
    pub expected: IssueCollectionFence,
    pub targets: Vec<IssueReference>,
    pub next_cursor: Option<IssueResyncCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTrackedSummary {
    pub state: IssueAutomationState,
    pub flags: IssueAutomationFlags,
}

/// 과거 워크플로 snapshot을 포함하지 않는다.
/// 현재 Shared 관찰에만 공개 내용이 있고, Withheld/Unknown에서는 대체 내용이 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionItem {
    pub issue: IssueReference,
    pub binding_id: IssueAutomationBindingId,
    /// 최초 Withheld 관찰은 워크플로 행을 만들지 않는다.
    pub tracked: Option<IssueTrackedSummary>,
    /// v1에서 넘어온 아직 재관찰하지 않은 행은 None이다.
    pub current: Option<IssueSourceRead>,
    /// 현재 관찰 행의 CAS 판이다. 과거 snapshot에서 만들지 않는다.
    pub observation_revision: Option<u64>,
    pub tracked_revision: Option<u64>,
    pub triage: Option<IssueTriageProjection>,
    /// 같은 catalog snapshot에서 읽은 열린 결정 요청이다.
    pub open_decisions: Vec<super::decision::StoredIssueDecision>,
    pub work: Option<super::coordinator::IssueWorkSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionQuery {
    pub binding_id: Option<IssueAutomationBindingId>,
    pub after: Option<IssueReference>,
    pub limit: u16,
}

impl IssueCollectionQuery {
    pub fn validate(&self) -> Result<(), IssueAutomationError> {
        if self.limit == 0 || self.limit > MAX_ISSUE_COLLECTION_PAGE_SIZE {
            return Err(IssueAutomationError::InvalidValue {
                field: "collection.limit",
            });
        }
        if let Some(after) = &self.after {
            super::validate_reference(after)?;
        }
        Ok(())
    }
}

impl Default for IssueCollectionQuery {
    fn default() -> Self {
        Self {
            binding_id: None,
            after: None,
            limit: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionPage {
    pub revision: u64,
    pub items: Vec<IssueCollectionItem>,
    pub next_after: Option<IssueReference>,
    /// 화면 페이지가 아닌 같은 읽기 snapshot의 전체 binding 집계다.
    pub report: Option<IssueTriageObserveReport>,
}

/// 저장된 판정만 담는다. 현재 입력/정책과 다르면 stale이며 실행 관문으로 쓸 수 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTriageProjection {
    pub assessment: IssueTriageAssessment,
    pub revision: u64,
    pub observation_revision: Option<u64>,
    pub tracked_revision: Option<u64>,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionDetail {
    pub revision: u64,
    pub item: Option<IssueCollectionItem>,
    pub report: Option<IssueTriageObserveReport>,
}

/// 범주는 서로 겹치지 않는다. 낡거나 읽지 못한 판정은 시작/승인 건수에 포함하지 않는다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueTriageObserveReport {
    pub revision: u64,
    pub binding_id: Option<IssueAutomationBindingId>,
    pub policy_version: Option<String>,
    pub assumed_mode: Option<super::IssueAutomationAutonomy>,
    pub policy_blocked: bool,
    pub total: u64,
    pub automatic_start: u64,
    pub human_approval: u64,
    pub deferred: u64,
    pub excluded: u64,
    pub needs_info: u64,
    pub unknown: u64,
    pub observation_only: u64,
    pub stale: u64,
    pub untriaged: u64,
    pub blocked: u64,
}

fn validate_revision(value: u64) -> Result<(), IssueAutomationError> {
    if value == 0 || value > MAX_ISSUE_AUTOMATION_REVISION {
        return Err(IssueAutomationError::InvalidRevision);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
