//! 읽기 전용 목록의 문구와 묶음을 서버에서 정한다 (#2539).
//! 판단·수명 전이는 수행하지 않으며 기존 안정 ID 순서를 바꾸지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::IssueAutomationState;
use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::source::{
    IssueRemoteState, IssueSourceContent, IssueSourceRead,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueListPresentation {
    pub number: Option<u64>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub status: IssueListStatus,
    pub reason: IssueListReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueListStatus {
    pub code: IssueListStatusCode,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueListStatusCode {
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
    Observed,
    Withheld,
    Unknown,
    NotYetObserved,
}

impl IssueListStatusCode {
    fn label(self) -> &'static str {
        match self {
            Self::Discovered => "발견됨",
            Self::Ineligible => "대상 아님",
            Self::NeedsInfo => "정보 필요",
            Self::AwaitingDecision => "결정 대기",
            Self::Ready => "준비됨",
            Self::Queued => "대기 중",
            Self::Running => "진행 중",
            Self::InReview => "리뷰 중",
            Self::Done => "완료",
            Self::Failed => "실패",
            Self::Cancelled => "취소됨",
            Self::Observed => "관찰됨",
            Self::Withheld => "공개 보류",
            Self::Unknown => "확인 불가",
            Self::NotYetObserved => "관찰 전",
        }
    }
}

impl From<IssueAutomationState> for IssueListStatusCode {
    fn from(value: IssueAutomationState) -> Self {
        match value {
            IssueAutomationState::Discovered => Self::Discovered,
            IssueAutomationState::Ineligible => Self::Ineligible,
            IssueAutomationState::NeedsInfo => Self::NeedsInfo,
            IssueAutomationState::AwaitingDecision => Self::AwaitingDecision,
            IssueAutomationState::Ready => Self::Ready,
            IssueAutomationState::Queued => Self::Queued,
            IssueAutomationState::Running => Self::Running,
            IssueAutomationState::InReview => Self::InReview,
            IssueAutomationState::Done => Self::Done,
            IssueAutomationState::Failed => Self::Failed,
            IssueAutomationState::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueListReason {
    pub code: IssueListReasonCode,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueListReasonCode {
    Observed,
    SourceReadFailed,
    DisclosureConfirmationRequired,
    NotYetObserved,
    Held,
    Stale,
    RemoteClosed,
    Workflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueListGroup {
    pub id: IssueListGroupId,
    pub label: String,
    /// 이 페이지 items의 인덱스다. 클라이언트는 이 순서와 묶음을 그대로 표시한다.
    pub item_indices: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueListGroupId {
    Collection,
}

impl IssueListGroup {
    pub(super) fn collection(item_count: usize) -> Self {
        Self {
            id: IssueListGroupId::Collection,
            label: "수집한 이슈".to_owned(),
            item_indices: (0..item_count).collect(),
        }
    }
}

impl IssueListPresentation {
    pub(super) fn from_domain(item: &IssueCollectionItem) -> Self {
        let (number, url, title) = current_fields(item);
        let status = status_code(item);
        let (reason, summary) = reason(item);
        Self {
            number,
            url,
            title,
            status: IssueListStatus {
                code: status,
                label: status.label().to_owned(),
            },
            reason: IssueListReason {
                code: reason,
                summary: summary.to_owned(),
            },
        }
    }
}

fn current_fields(item: &IssueCollectionItem) -> (Option<u64>, Option<String>, Option<String>) {
    let Some(IssueSourceRead::Observed(observation)) = &item.current else {
        return (None, None, None);
    };
    let web = observation
        .web
        .as_ref()
        .filter(|web| web.validate_for(&item.issue).is_ok());
    let title = match &observation.content {
        IssueSourceContent::Shared(snapshot) => Some(snapshot.shared_copy.title.clone()),
        IssueSourceContent::Withheld { .. } => None,
    };
    (
        web.map(|web| web.number),
        web.map(|web| web.url.clone()),
        title,
    )
}

fn status_code(item: &IssueCollectionItem) -> IssueListStatusCode {
    if let Some(tracked) = &item.tracked {
        return tracked.state.into();
    }
    match &item.current {
        None => IssueListStatusCode::NotYetObserved,
        Some(IssueSourceRead::Unknown { .. }) => IssueListStatusCode::Unknown,
        Some(IssueSourceRead::Observed(observation)) => match &observation.content {
            IssueSourceContent::Shared(_) => IssueListStatusCode::Observed,
            IssueSourceContent::Withheld { .. } => IssueListStatusCode::Withheld,
        },
    }
}

fn reason(item: &IssueCollectionItem) -> (IssueListReasonCode, &'static str) {
    let observation = match &item.current {
        None => {
            return (
                IssueListReasonCode::NotYetObserved,
                "아직 현재 이슈 정보를 읽지 않았습니다.",
            );
        }
        Some(IssueSourceRead::Unknown { .. }) => {
            return (
                IssueListReasonCode::SourceReadFailed,
                "현재 이슈 정보를 읽지 못했습니다.",
            );
        }
        Some(IssueSourceRead::Observed(observation)) => observation,
    };
    if matches!(&observation.content, IssueSourceContent::Withheld { .. }) {
        return (
            IssueListReasonCode::DisclosureConfirmationRequired,
            "내용 공개에 사용자 확인이 필요합니다.",
        );
    }
    if let Some(tracked) = &item.tracked {
        if tracked.flags.held {
            return (IssueListReasonCode::Held, "작업이 보류되어 있습니다.");
        }
        if tracked.flags.stale {
            return (
                IssueListReasonCode::Stale,
                "이슈 내용이 바뀌어 다시 확인해야 합니다.",
            );
        }
    }
    if observation.remote_state == IssueRemoteState::Closed {
        return (
            IssueListReasonCode::RemoteClosed,
            "원격 이슈가 닫혀 있습니다.",
        );
    }
    match &item.tracked {
        Some(tracked) => (
            IssueListReasonCode::Workflow,
            workflow_summary(tracked.state),
        ),
        None => (
            IssueListReasonCode::Observed,
            "현재 이슈 정보를 확인했습니다.",
        ),
    }
}

fn workflow_summary(state: IssueAutomationState) -> &'static str {
    match state {
        IssueAutomationState::Discovered => "이슈를 수집했습니다.",
        IssueAutomationState::Ineligible => "자동 처리 대상에서 제외되었습니다.",
        IssueAutomationState::NeedsInfo => "진행에 필요한 정보가 부족합니다.",
        IssueAutomationState::AwaitingDecision => "사용자 결정을 기다리고 있습니다.",
        IssueAutomationState::Ready => "작업을 시작할 준비가 되었습니다.",
        IssueAutomationState::Queued => "작업 시작을 기다리고 있습니다.",
        IssueAutomationState::Running => "작업이 진행 중입니다.",
        IssueAutomationState::InReview => "작업 결과를 리뷰하고 있습니다.",
        IssueAutomationState::Done => "작업이 완료되었습니다.",
        IssueAutomationState::Failed => "작업이 실패한 상태입니다.",
        IssueAutomationState::Cancelled => "작업이 취소되었습니다.",
    }
}

#[cfg(test)]
mod tests;
