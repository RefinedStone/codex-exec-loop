//! 이슈 출처가 반환하는 관찰 값과 페이지 커서다 (#2537).
//! 수명 상태나 실행 권한을 바꾸지 않는다. 제목과 본문은 공개 정책을 지난 공유본만 반환한다.

use serde::{Deserialize, Serialize};

use crate::domain::issue_policy::{IssueDisclosurePolicy, IssueSensitiveSpanAction};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueProjectScope, IssueProvider, IssueReference,
};

use super::IssueAutomationSnapshot;

mod web;
pub use web::IssueSourceWeb;

pub const ISSUE_SOURCE_CURSOR_VERSION: u32 = 1;
pub const MAX_ISSUE_SOURCE_PAGE_SIZE: u16 = 100;
const MAX_CURSOR_BYTES: usize = 4096;
const MAX_SELECTION_LABELS: usize = 100;

/// 외부 오류의 원문을 갖지 않는 닫힌 실패 코드다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSourceFailure {
    InvalidRequest,
    UnsupportedProvider,
    InvalidCursor,
    ProviderUnavailable,
    MalformedResponse,
    IdentityMismatch,
    MetadataIncomplete,
}

impl std::fmt::Display for IssueSourceFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "invalid issue source request",
            Self::UnsupportedProvider => "unsupported issue source provider",
            Self::InvalidCursor => "invalid issue source cursor",
            Self::ProviderUnavailable => "issue source is unavailable",
            Self::MalformedResponse => "issue source returned malformed data",
            Self::IdentityMismatch => "issue source identity does not match",
            Self::MetadataIncomplete => "issue source metadata is incomplete",
        })
    }
}

impl std::error::Error for IssueSourceFailure {}

/// 최초 이름 조회로 확인한 저장소의 안정 좌표다. 표시 이름이나 이슈 번호를 식별자로 쓰지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourceRepository {
    pub provider: IssueProvider,
    pub server_id: String,
    pub repository_id: String,
    pub project_scope: IssueProjectScope,
}

impl IssueSourceRepository {
    pub fn from_reference(issue: &IssueReference) -> Self {
        Self {
            provider: issue.provider,
            server_id: issue.server_id.clone(),
            repository_id: issue.repository_id.clone(),
            project_scope: issue.project_scope.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), IssueSourceFailure> {
        validate_value(&self.server_id)?;
        validate_value(&self.repository_id)?;
        validate_value(&self.project_scope.workspace_dir)
    }

    pub fn matches_reference(&self, issue: &IssueReference) -> bool {
        self.provider == issue.provider
            && self.server_id == issue.server_id
            && self.repository_id == issue.repository_id
            && self.project_scope == issue.project_scope
    }
}

/// 발견은 열린 이슈만 읽는다. 빈 라벨 조건을 모든 이슈라는 뜻으로 해석하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "labels",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IssueDiscoverySelection {
    AllOpen,
    LabelsAny(Vec<String>),
}

impl IssueDiscoverySelection {
    /// 라벨 이름은 정확히 비교한다. 정렬과 중복 제거만 하고 대소문자나 공백을 바꾸지 않는다.
    pub fn normalized(&self) -> Result<Self, IssueSourceFailure> {
        match self {
            Self::AllOpen => Ok(Self::AllOpen),
            Self::LabelsAny(labels) => {
                for label in labels {
                    validate_value(label)?;
                }
                let mut labels = labels.clone();
                labels.sort();
                labels.dedup();
                if labels.is_empty() || labels.len() > MAX_SELECTION_LABELS {
                    return Err(IssueSourceFailure::InvalidRequest);
                }
                Ok(Self::LabelsAny(labels))
            }
        }
    }

    /// 일부 라벨만 읽었어도 일치 하나는 충분하다. 불일치를 확정하려면 목록이 완전해야 한다.
    pub fn matches_labels(
        &self,
        labels: &IssueMetadataList<IssueSourceLabel>,
    ) -> Result<bool, IssueSourceFailure> {
        match self.normalized()? {
            Self::AllOpen => Ok(true),
            Self::LabelsAny(selected) => {
                if labels.completeness == IssueMetadataCompleteness::Unavailable {
                    return Err(IssueSourceFailure::MetadataIncomplete);
                }
                if labels
                    .items
                    .iter()
                    .any(|label| selected.contains(&label.name))
                {
                    return Ok(true);
                }
                if labels.completeness == IssueMetadataCompleteness::Complete {
                    Ok(false)
                } else {
                    Err(IssueSourceFailure::MetadataIncomplete)
                }
            }
        }
    }
}

/// 호출자가 이미 해석한 공개 정책이다. 출처 adapter는 정책 파일이나 application service를 읽지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSourceDisclosure {
    pub policy: IssueDisclosurePolicy,
    pub policy_version: String,
}

impl IssueSourceDisclosure {
    pub fn validate(&self) -> Result<(), IssueSourceFailure> {
        validate_value(&self.policy_version)
    }
}

/// 커서 저장용 공개 정책 값이다. 판 문자열이 같아도 실제 처리 네 개가 달라지면 커서를 재사용하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourcePolicyActions {
    pub secret: IssueSensitiveSpanAction,
    pub name: IssueSensitiveSpanAction,
    pub path: IssueSensitiveSpanAction,
    pub log: IssueSensitiveSpanAction,
}

impl From<IssueDisclosurePolicy> for IssueSourcePolicyActions {
    fn from(policy: IssueDisclosurePolicy) -> Self {
        Self {
            secret: policy.secret,
            name: policy.name,
            path: policy.path,
            log: policy.log,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedIssueDiscoveryCursor {
    pub version: u32,
    pub repository: IssueSourceRepository,
    pub selection: IssueDiscoverySelection,
    pub page_size: u16,
    pub policy_version: String,
    pub actions: IssueSourcePolicyActions,
    pub opaque: String,
}

/// 한 페이지 탐색의 다음 위치다. 증분 동기화의 watermark나 실행 허가가 아니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDiscoveryCursor {
    record: PersistedIssueDiscoveryCursor,
}

impl IssueDiscoveryCursor {
    pub fn new(
        repository: IssueSourceRepository,
        selection: IssueDiscoverySelection,
        page_size: u16,
        disclosure: &IssueSourceDisclosure,
        opaque: impl Into<String>,
    ) -> Result<Self, IssueSourceFailure> {
        repository.validate()?;
        disclosure.validate()?;
        validate_page_size(page_size)?;
        let record = PersistedIssueDiscoveryCursor {
            version: ISSUE_SOURCE_CURSOR_VERSION,
            repository,
            selection: selection.normalized()?,
            page_size,
            policy_version: disclosure.policy_version.clone(),
            actions: disclosure.policy.into(),
            opaque: opaque.into(),
        };
        Self::hydrate(record)
    }

    pub fn hydrate(record: PersistedIssueDiscoveryCursor) -> Result<Self, IssueSourceFailure> {
        let valid = (|| {
            record.repository.validate()?;
            validate_page_size(record.page_size)?;
            validate_value(&record.policy_version)?;
            validate_value(&record.opaque)?;
            if record.version != ISSUE_SOURCE_CURSOR_VERSION
                || record.opaque.len() > MAX_CURSOR_BYTES
                || record.opaque.chars().any(char::is_control)
                || record.selection.normalized()? != record.selection
            {
                return Err(IssueSourceFailure::InvalidCursor);
            }
            Ok(())
        })();
        valid.map_err(|_| IssueSourceFailure::InvalidCursor)?;
        Ok(Self { record })
    }

    pub fn to_record(&self) -> PersistedIssueDiscoveryCursor {
        self.record.clone()
    }

    /// 모든 요청 좌표가 같은 경우에만 provider의 opaque 위치를 돌려준다.
    pub fn validate_for(
        &self,
        repository: &IssueSourceRepository,
        selection: &IssueDiscoverySelection,
        page_size: u16,
        disclosure: &IssueSourceDisclosure,
    ) -> Result<&str, IssueSourceFailure> {
        repository
            .validate()
            .map_err(|_| IssueSourceFailure::InvalidCursor)?;
        disclosure
            .validate()
            .map_err(|_| IssueSourceFailure::InvalidCursor)?;
        let selection = selection
            .normalized()
            .map_err(|_| IssueSourceFailure::InvalidCursor)?;
        if self.record.repository != *repository
            || self.record.selection != selection
            || self.record.page_size != page_size
            || self.record.policy_version != disclosure.policy_version
            || self.record.actions != IssueSourcePolicyActions::from(disclosure.policy)
        {
            return Err(IssueSourceFailure::InvalidCursor);
        }
        Ok(&self.record.opaque)
    }
}

pub fn validate_page_size(page_size: u16) -> Result<(), IssueSourceFailure> {
    if page_size == 0 || page_size > MAX_ISSUE_SOURCE_PAGE_SIZE {
        return Err(IssueSourceFailure::InvalidRequest);
    }
    Ok(())
}

fn validate_value(value: &str) -> Result<(), IssueSourceFailure> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(IssueSourceFailure::InvalidRequest);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueRemoteState {
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMetadataCompleteness {
    Complete,
    Truncated,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueMetadataList<T> {
    pub items: Vec<T>,
    pub completeness: IssueMetadataCompleteness,
}

/// 공급자가 명시한 계정 종류다. UserAccount는 실제 조작자가 사람임을 보증하지 않는다.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSourceActorKind {
    UserAccount,
    Bot,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourceActor {
    pub id: String,
    pub login: String,
    /// 과거 관찰에 종류가 없으면 계정 이름이나 안정 ID로 추정하지 않는다.
    #[serde(default)]
    pub kind: IssueSourceActorKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourceLabel {
    pub id: String,
    pub name: String,
}

/// 현재 라벨에 대응하는 실제 최신 적용 이벤트다. 작성자나 담당자를 라벨 행위자로 추정하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueLabelAttribution {
    pub label: IssueSourceLabel,
    pub event_id: String,
    /// 이벤트는 있으나 계정이 없어졌으면 None이다. 신뢰하는 행위자로 간주하지 않는다.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub actor: Option<IssueSourceActor>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuePrPresence {
    Present,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueContentWithheldReason {
    UserConfirmationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IssueSourceContent {
    Shared(IssueAutomationSnapshot),
    /// 원본은 읽었지만 공개할 수 없는 상태다. 빈 공유본이나 이전 공유본으로 바꾸지 않는다.
    Withheld {
        basis: IssueComparisonBasis,
        policy_version: String,
        reason: IssueContentWithheldReason,
    },
}

/// 저장된 생성 근거다. 근거가 없다는 것은 외부나 사람이 만들었다는 증명이 아니다.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCreationProvenance {
    #[default]
    Unattributed,
    AkraCreated,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourceObservation {
    pub issue: IssueReference,
    /// 현재 관찰의 표시 번호와 검증한 URL이다. 과거 JSON에 없으면 다른 값으로 채우지 않는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<IssueSourceWeb>,
    pub remote_state: IssueRemoteState,
    /// Provider 관찰은 Unattributed로 시작하고 수집 서비스가 로컬 생성 근거를 붙인다.
    #[serde(default)]
    pub creation_provenance: IssueCreationProvenance,
    pub labels: IssueMetadataList<IssueSourceLabel>,
    pub assignees: IssueMetadataList<IssueSourceActor>,
    /// null은 작성자 계정이 없는 경우다. 필드 누락이나 답 파싱 실패와 구분한다.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub author: Option<IssueSourceActor>,
    pub label_actors: IssueMetadataList<IssueLabelAttribution>,
    pub linked_pull_request: IssuePrPresence,
    pub content: IssueSourceContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IssueSourceRead {
    Observed(Box<IssueSourceObservation>),
    /// 대상은 유지하되 과거 관찰 값을 섞지 않는다. 읽지 못함은 닫힘이나 변화 없음이 아니다.
    Unknown {
        issue: IssueReference,
        reason: IssueSourceFailure,
    },
}

// null은 허용하지만 필드 자체의 누락을 null로 대신하지 않는다.
fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[cfg(test)]
#[path = "source/tests.rs"]
mod tests;
