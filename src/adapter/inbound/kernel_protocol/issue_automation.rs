//! 이슈 장부의 조회/이벤트 DTO다. 현재 관찰의 공유본만 직렬화한다 (#2538).
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{IssueAutonomyState, KernelProtocolError, KernelQuery};
use crate::domain::issue_automation::autonomy::IssueAutonomyProjection;
use crate::domain::issue_automation::collection::{
    IssueCollectionFailure, IssueCollectionItem, IssueCollectionPage, IssueCollectionProjection,
};
use crate::domain::issue_automation::source::{
    IssueContentWithheldReason, IssueLabelAttribution, IssueMetadataCompleteness,
    IssueMetadataList, IssuePrPresence, IssueRemoteState, IssueSourceActor, IssueSourceContent,
    IssueSourceFailure, IssueSourceLabel, IssueSourceObservation, IssueSourceRead,
};
use crate::domain::issue_automation::{
    IssueAutomationBinding, IssueAutomationState as DomainWorkflowState,
};
use crate::domain::issue_work::{IssueComparisonBasis, IssueReference};
pub use crate::domain::kernel_event::IssueAutomationModeRecord as IssueAutomationMode;
use crate::domain::kernel_event::{
    IssueEventProjectScope, IssueEventProvider, IssueEventReference,
};

mod detail;
mod presentation;
mod timeline;
mod triage;
pub use detail::IssueState;
pub use presentation::{IssueListGroup, IssueListPresentation};
#[cfg(test)]
pub use triage::IssueTriageReasonState;
pub use triage::{IssueTriageObserveReportState, IssueTriageState};

const DEFAULT_ISSUES_LIMIT: u16 = 50;
const MAX_ISSUES_LIMIT: u16 = 100;
const MAX_ISSUES_CURSOR_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueAutomationState {
    Unavailable {},
    Ready(Box<IssueAutomationReadyState>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationReadyState {
    #[serde(default)]
    pub autonomy: IssueAutonomyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_pause: Option<super::IssueBindingPauseState>,
    pub mode: IssueAutomationMode,
    pub epoch: u64,
    /// 목록과 같은 catalog 판이다. 수집 control의 CAS 판과는 별개다.
    pub revision: u64,
    pub binding_id: Option<String>,
    pub binding: Option<IssueBindingState>,
    pub last_attempt_at_ms: Option<u64>,
    pub last_success_at_ms: Option<u64>,
    pub last_failure: Option<IssueCollectionFailureState>,
    pub consecutive_failures: u32,
    /// 명령 전용 저장 조회는 실행 중 여부를 알지 못한다.
    pub collecting: Option<bool>,
    pub next_poll_after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingState {
    pub id: String,
    pub provider: IssueEventProvider,
    pub server_id: String,
    pub repository_id: String,
    pub project_scope: IssueEventProjectScope,
}

impl From<&IssueAutomationBinding> for IssueBindingState {
    fn from(binding: &IssueAutomationBinding) -> Self {
        Self {
            id: binding.id().as_str().to_string(),
            provider: binding.provider().into(),
            server_id: binding.server_id().to_string(),
            repository_id: binding.repository_id().to_string(),
            project_scope: IssueEventProjectScope {
                workspace_dir: binding.project_scope().workspace_dir.clone(),
            },
        }
    }
}

impl IssueAutomationState {
    pub fn unavailable() -> Self {
        Self::Unavailable {}
    }

    pub fn from_domain(projection: &IssueCollectionProjection) -> Self {
        let state = &projection.state;
        Self::Ready(Box::new(IssueAutomationReadyState {
            autonomy: IssueAutonomyState::default(),
            binding_pause: None,
            mode: state.config.mode.into(),
            epoch: state.epoch,
            revision: projection.catalog_revision,
            binding_id: state.binding_id.as_ref().map(ToString::to_string),
            binding: projection.binding.as_ref().map(Into::into),
            last_attempt_at_ms: state.status.last_attempt_at_ms,
            last_success_at_ms: state.status.last_success_at_ms,
            last_failure: state.status.last_failure.map(Into::into),
            consecutive_failures: state.status.consecutive_failures,
            collecting: None,
            next_poll_after_ms: None,
        }))
    }

    pub fn with_binding_pause(
        mut self,
        pause: Option<&crate::domain::issue_automation::binding_pause::IssueBindingPauseState>,
    ) -> Self {
        if let Self::Ready(state) = &mut self {
            state.binding_pause = state
                .binding_id
                .as_deref()
                .map(|id| super::IssueBindingPauseState::from_current(id, pause));
        }
        self
    }

    pub fn with_autonomy(mut self, projection: &IssueAutonomyProjection) -> Self {
        if let Self::Ready(state) = &mut self {
            state.autonomy = IssueAutonomyState::from_domain(projection);
        }
        self
    }

    pub fn with_runtime(mut self, collecting: bool, next_poll_after_ms: Option<u64>) -> Self {
        if let Self::Ready(state) = &mut self {
            state.collecting = Some(collecting);
            state.next_poll_after_ms = next_poll_after_ms;
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    content = "reason",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IssueCollectionFailureState {
    Source(IssueSourceFailureState),
    OriginUnavailable,
    PolicyUnavailable,
}

impl From<IssueCollectionFailure> for IssueCollectionFailureState {
    fn from(value: IssueCollectionFailure) -> Self {
        match value {
            IssueCollectionFailure::Source(reason) => Self::Source(reason.into()),
            IssueCollectionFailure::OriginUnavailable => Self::OriginUnavailable,
            IssueCollectionFailure::PolicyUnavailable => Self::PolicyUnavailable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueSourceFailureState {
    InvalidRequest,
    UnsupportedProvider,
    InvalidCursor,
    ProviderUnavailable,
    MalformedResponse,
    IdentityMismatch,
    MetadataIncomplete,
    FeedbackUnconfirmed,
}

impl From<IssueSourceFailure> for IssueSourceFailureState {
    fn from(value: IssueSourceFailure) -> Self {
        match value {
            IssueSourceFailure::InvalidRequest => Self::InvalidRequest,
            IssueSourceFailure::UnsupportedProvider => Self::UnsupportedProvider,
            IssueSourceFailure::InvalidCursor => Self::InvalidCursor,
            IssueSourceFailure::ProviderUnavailable => Self::ProviderUnavailable,
            IssueSourceFailure::MalformedResponse => Self::MalformedResponse,
            IssueSourceFailure::IdentityMismatch => Self::IdentityMismatch,
            IssueSourceFailure::MetadataIncomplete => Self::MetadataIncomplete,
            IssueSourceFailure::FeedbackUnconfirmed => Self::FeedbackUnconfirmed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssuesState {
    Unavailable {},
    Ready(Box<IssuesPageState>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssuesPageState {
    pub revision: u64,
    pub items: Vec<IssueListItem>,
    pub groups: Vec<IssueListGroup>,
    pub next_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<IssueTriageObserveReportState>,
}

impl IssuesState {
    pub fn unavailable() -> Self {
        Self::Unavailable {}
    }

    pub fn from_domain(page: &IssueCollectionPage) -> Self {
        Self::Ready(Box::new(IssuesPageState {
            revision: page.revision,
            items: page.items.iter().map(Into::into).collect(),
            groups: IssueListGroup::from_items(&page.items),
            next_after: page.next_after.as_ref().map(encode_issue_cursor),
            report: page.report.as_ref().map(Into::into),
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueListItem {
    pub issue: IssueEventReference,
    pub binding_id: String,
    pub tracked: Option<IssueTrackedState>,
    pub current: IssueObservationState,
    pub presentation: IssueListPresentation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<IssueTriageState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<crate::domain::issue_automation::coordinator::IssueWorkSummary>,
}

impl From<&IssueCollectionItem> for IssueListItem {
    fn from(item: &IssueCollectionItem) -> Self {
        Self {
            issue: (&item.issue).into(),
            binding_id: item.binding_id.as_str().to_string(),
            presentation: IssueListPresentation::from_domain(item),
            triage: IssueTriageState::from_item(item),
            work: item.work.clone(),
            tracked: item.tracked.as_ref().map(|tracked| IssueTrackedState {
                state: tracked.state.into(),
                held: tracked.flags.held,
                stale: tracked.flags.stale,
                unknown: tracked.flags.unknown,
            }),
            current: match &item.current {
                None => IssueObservationState::NotYetObserved {},
                Some(IssueSourceRead::Unknown { reason, .. }) => IssueObservationState::Unknown {
                    reason: (*reason).into(),
                },
                Some(IssueSourceRead::Observed(observation)) => {
                    IssueObservationState::Observed(Box::new(observation.as_ref().into()))
                }
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueTrackedState {
    pub state: IssueWorkflowState,
    pub held: bool,
    pub stale: bool,
    pub unknown: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueWorkflowState {
    Discovered,
    Ineligible,
    NeedsInfo,
    AwaitingDecision,
    Ready,
    Queued,
    Running,
    InReview,
    Done,
    Failed,
    Cancelled,
}

impl From<DomainWorkflowState> for IssueWorkflowState {
    fn from(value: DomainWorkflowState) -> Self {
        match value {
            DomainWorkflowState::Discovered => Self::Discovered,
            DomainWorkflowState::Ineligible => Self::Ineligible,
            DomainWorkflowState::NeedsInfo => Self::NeedsInfo,
            DomainWorkflowState::AwaitingDecision => Self::AwaitingDecision,
            DomainWorkflowState::Ready => Self::Ready,
            DomainWorkflowState::Queued => Self::Queued,
            DomainWorkflowState::Running => Self::Running,
            DomainWorkflowState::InReview => Self::InReview,
            DomainWorkflowState::Done => Self::Done,
            DomainWorkflowState::Failed => Self::Failed,
            DomainWorkflowState::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueObservationState {
    NotYetObserved {},
    Unknown { reason: IssueSourceFailureState },
    Observed(Box<IssueObservedState>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueObservedState {
    pub remote_state: IssueRemoteStateValue,
    pub labels: IssueMetadataState<IssueLabelState>,
    pub assignees: IssueMetadataState<IssueActorState>,
    pub author: Option<IssueActorState>,
    pub label_actors: IssueMetadataState<IssueLabelActorState>,
    pub linked_pull_request: IssuePrPresenceState,
    pub content: IssueContentState,
}

impl From<&IssueSourceObservation> for IssueObservedState {
    fn from(value: &IssueSourceObservation) -> Self {
        Self {
            remote_state: value.remote_state.into(),
            labels: IssueMetadataState::from_domain(&value.labels, |item| item.into()),
            assignees: IssueMetadataState::from_domain(&value.assignees, |item| item.into()),
            author: value.author.as_ref().map(Into::into),
            label_actors: IssueMetadataState::from_domain(&value.label_actors, |item| item.into()),
            linked_pull_request: value.linked_pull_request.into(),
            content: (&value.content).into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueRemoteStateValue {
    Open,
    Closed,
}

impl From<IssueRemoteState> for IssueRemoteStateValue {
    fn from(value: IssueRemoteState) -> Self {
        match value {
            IssueRemoteState::Open => Self::Open,
            IssueRemoteState::Closed => Self::Closed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueMetadataCompletenessState {
    Complete,
    Truncated,
    Unavailable,
}

impl From<IssueMetadataCompleteness> for IssueMetadataCompletenessState {
    fn from(value: IssueMetadataCompleteness) -> Self {
        match value {
            IssueMetadataCompleteness::Complete => Self::Complete,
            IssueMetadataCompleteness::Truncated => Self::Truncated,
            IssueMetadataCompleteness::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueMetadataState<T> {
    pub items: Vec<T>,
    pub completeness: IssueMetadataCompletenessState,
}

impl<T> IssueMetadataState<T> {
    fn from_domain<U>(value: &IssueMetadataList<U>, convert: impl FnMut(&U) -> T) -> Self {
        Self {
            items: value.items.iter().map(convert).collect(),
            completeness: value.completeness.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueActorState {
    pub id: String,
    pub login: String,
}

impl From<&IssueSourceActor> for IssueActorState {
    fn from(value: &IssueSourceActor) -> Self {
        Self {
            id: value.id.clone(),
            login: value.login.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueLabelState {
    pub id: String,
    pub name: String,
}

impl From<&IssueSourceLabel> for IssueLabelState {
    fn from(value: &IssueSourceLabel) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueLabelActorState {
    pub label: IssueLabelState,
    pub event_id: String,
    pub actor: Option<IssueActorState>,
    pub created_at: String,
}

impl From<&IssueLabelAttribution> for IssueLabelActorState {
    fn from(value: &IssueLabelAttribution) -> Self {
        Self {
            label: (&value.label).into(),
            event_id: value.event_id.clone(),
            actor: value.actor.as_ref().map(Into::into),
            created_at: value.created_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssuePrPresenceState {
    Present,
    Absent,
    Unknown,
}

impl From<IssuePrPresence> for IssuePrPresenceState {
    fn from(value: IssuePrPresence) -> Self {
        match value {
            IssuePrPresence::Present => Self::Present,
            IssuePrPresence::Absent => Self::Absent,
            IssuePrPresence::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueBasisState {
    pub field_hash: IssueHashState,
    pub provider_updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueHashState {
    pub version: u32,
    pub sha256: String,
}

impl From<&IssueComparisonBasis> for IssueBasisState {
    fn from(value: &IssueComparisonBasis) -> Self {
        Self {
            field_hash: IssueHashState {
                version: value.field_hash.version,
                sha256: value.field_hash.sha256.clone(),
            },
            provider_updated_at: value.provider_updated_at.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueSharedCopyState {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueWithheldReasonState {
    UserConfirmationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueContentState {
    Shared {
        basis: IssueBasisState,
        policy_version: String,
        shared_copy: IssueSharedCopyState,
    },
    Withheld {
        basis: IssueBasisState,
        policy_version: String,
        reason: IssueWithheldReasonState,
    },
}

impl From<&IssueSourceContent> for IssueContentState {
    fn from(value: &IssueSourceContent) -> Self {
        match value {
            IssueSourceContent::Shared(snapshot) => Self::Shared {
                basis: (&snapshot.basis).into(),
                policy_version: snapshot.policy_version.clone(),
                shared_copy: IssueSharedCopyState {
                    title: snapshot.shared_copy.title.clone(),
                    body: snapshot.shared_copy.body.clone(),
                },
            },
            IssueSourceContent::Withheld {
                basis,
                policy_version,
                reason,
            } => Self::Withheld {
                basis: basis.into(),
                policy_version: policy_version.clone(),
                reason: match reason {
                    IssueContentWithheldReason::UserConfirmationRequired => {
                        IssueWithheldReasonState::UserConfirmationRequired
                    }
                },
            },
        }
    }
}

/// 목록의 안정 정렬 위치다. 공급자 발견 커서와는 별도 namespace/판을 갖는다.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueListCursor {
    kind: IssueListCursorKind,
    version: u32,
    after: IssueEventReference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IssueListCursorKind {
    IssueList,
}

fn encode_issue_cursor(reference: &IssueReference) -> String {
    let record = IssueListCursor {
        kind: IssueListCursorKind::IssueList,
        version: 1,
        after: reference.into(),
    };
    URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&record).expect("typed issue cursor contains only serializable values"),
    )
}

fn decode_issue_cursor(
    cursor: &str,
    workspace: &str,
) -> Result<IssueReference, KernelProtocolError> {
    let invalid = || KernelProtocolError::invalid_params("invalid issues page cursor");
    if cursor.is_empty() || cursor.len() > MAX_ISSUES_CURSOR_BYTES {
        return Err(invalid());
    }
    let decoded = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
    let record: IssueListCursor = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    let reference: IssueReference = record.after.into();
    let valid_text =
        |value: &str| !value.trim().is_empty() && value.len() <= 4096 && !value.contains('\0');
    if record.version != 1
        || reference.project_scope.workspace_dir != workspace
        || ![
            reference.server_id.as_str(),
            reference.repository_id.as_str(),
            reference.issue_id.as_str(),
            reference.project_scope.workspace_dir.as_str(),
        ]
        .into_iter()
        .all(valid_text)
    {
        return Err(invalid());
    }
    Ok(reference)
}

impl KernelQuery {
    /// 저장 목록의 query 범위다. 출처 발견 커서는 받지 않으며 workspace가 다른 위치도 거절한다.
    pub fn issues_range(
        &self,
        workspace: &str,
    ) -> Result<Option<(Option<IssueReference>, u16)>, KernelProtocolError> {
        let Self::Issues { after, limit } = self else {
            return Ok(None);
        };
        let limit = limit.unwrap_or(DEFAULT_ISSUES_LIMIT);
        if limit == 0 || limit > MAX_ISSUES_LIMIT {
            return Err(KernelProtocolError::invalid_params(
                "issues limit must be between 1 and 100",
            ));
        }
        let after = after
            .as_deref()
            .map(|cursor| decode_issue_cursor(cursor, workspace))
            .transpose()?;
        Ok(Some((after, limit)))
    }
}

#[cfg(test)]
mod tests;
