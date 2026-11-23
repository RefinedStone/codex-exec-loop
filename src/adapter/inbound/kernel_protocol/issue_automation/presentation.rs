//! 읽기 전용 목록의 문구와 묶음을 서버에서 정한다 (#2539).
//! 판단·수명 전이는 수행하지 않으며 기존 안정 ID 순서를 바꾸지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::decision::{IssueDecisionKind, IssueDecisionState};
use crate::domain::issue_automation::delivery::{
    IssueDeliveryDiffVerdict, IssueDeliveryRiskReason,
};
use crate::domain::issue_automation::policy::{IssueAutomationGate, IssueAutomationGateAction};
use crate::domain::issue_automation::source::{
    IssueRemoteState, IssueSourceContent, IssueSourceRead,
};
use crate::domain::issue_automation::triage::{IssueTriageEligibility, IssueTriageReadiness};
use crate::domain::issue_automation::{IssueAutomationAutonomy, IssueAutomationState};

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
    Triage,
    ProtectedPath,
    DiffTooLarge,
    BinaryDiff,
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
    MyTurn,
    Upcoming,
    Running,
    InReview,
    Done,
    Excluded,
}

impl IssueListGroup {
    pub(super) fn from_items(items: &[IssueCollectionItem]) -> Vec<Self> {
        if items.is_empty() {
            return vec![Self::collection(0)];
        }
        [
            (IssueListGroupId::MyTurn, "내 차례"),
            (IssueListGroupId::Upcoming, "곧 시작"),
            (IssueListGroupId::Running, "실행 중"),
            (IssueListGroupId::InReview, "리뷰 중"),
            (IssueListGroupId::Done, "끝남"),
            (IssueListGroupId::Excluded, "제외"),
            (IssueListGroupId::Collection, "수집한 이슈"),
        ]
        .into_iter()
        .filter_map(|(id, label)| {
            let item_indices: Vec<_> = items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| (group_id(item) == id).then_some(index))
                .collect();
            (!item_indices.is_empty()).then(|| Self {
                id,
                label: label.to_owned(),
                item_indices,
            })
        })
        .collect()
    }

    pub(super) fn collection(item_count: usize) -> Self {
        Self {
            id: IssueListGroupId::Collection,
            label: "수집한 이슈".to_owned(),
            item_indices: (0..item_count).collect(),
        }
    }
}

/// 실제 실행 상태는 보존하고, 현재 전달 결정을 기다릴 때만 사람의 차례로 표시한다.
fn group_id(item: &IssueCollectionItem) -> IssueListGroupId {
    if pending_delivery_review(item).is_some() {
        return IssueListGroupId::MyTurn;
    }
    if let Some(tracked) = &item.tracked {
        match tracked.state {
            IssueAutomationState::Running => return IssueListGroupId::Running,
            IssueAutomationState::InReview => return IssueListGroupId::InReview,
            IssueAutomationState::Done | IssueAutomationState::Cancelled => {
                return IssueListGroupId::Done;
            }
            _ => {}
        }
        if tracked.flags.held || tracked.flags.stale || tracked.flags.unknown {
            return IssueListGroupId::MyTurn;
        }
        match tracked.state {
            IssueAutomationState::Queued => return IssueListGroupId::Upcoming,
            IssueAutomationState::AwaitingDecision | IssueAutomationState::Failed => {
                return IssueListGroupId::MyTurn;
            }
            _ => {}
        }
    }
    let Some(triage) = &item.triage else {
        return IssueListGroupId::Collection;
    };
    if triage.stale {
        return IssueListGroupId::MyTurn;
    }
    let assessment = &triage.assessment;
    match assessment.eligibility() {
        IssueTriageEligibility::Ineligible => return IssueListGroupId::Excluded,
        IssueTriageEligibility::Unknown => return IssueListGroupId::MyTurn,
        IssueTriageEligibility::Eligible => {}
    }
    if assessment.readiness() != IssueTriageReadiness::Ready {
        return IssueListGroupId::MyTurn;
    }
    if assessment.effective_mode() <= IssueAutomationAutonomy::Observe {
        return IssueListGroupId::Collection;
    }
    if item.start_budget_wait.is_some() {
        return IssueListGroupId::Upcoming;
    }
    // 관찰 보고의 가정 수준은 실제 시작 관문이나 목록의 진행 약속이 아니다.
    match assessment
        .gates()
        .iter()
        .find(|gate| gate.gate == IssueAutomationGate::Start)
    {
        Some(gate)
            if gate.enabled
                && matches!(
                    gate.action,
                    IssueAutomationGateAction::Auto
                        | IssueAutomationGateAction::TimeoutDefault
                        | IssueAutomationGateAction::TimeoutArbiter
                ) =>
        {
            IssueListGroupId::Upcoming
        }
        _ => IssueListGroupId::MyTurn,
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
    if pending_delivery_review(item).is_some() {
        return IssueListStatusCode::AwaitingDecision;
    }
    if let Some(tracked) = &item.tracked
        && !matches!(
            tracked.state,
            IssueAutomationState::Discovered
                | IssueAutomationState::Ineligible
                | IssueAutomationState::NeedsInfo
                | IssueAutomationState::Ready
        )
    {
        return tracked.state.into();
    }
    if let Some(triage) = &item.triage
        && !triage.stale
    {
        return match triage.assessment.eligibility() {
            IssueTriageEligibility::Ineligible => IssueListStatusCode::Ineligible,
            IssueTriageEligibility::Unknown => IssueListStatusCode::Unknown,
            IssueTriageEligibility::Eligible => match triage.assessment.readiness() {
                IssueTriageReadiness::Ready => IssueListStatusCode::Ready,
                IssueTriageReadiness::NeedsInfo => IssueListStatusCode::NeedsInfo,
                IssueTriageReadiness::Unknown => IssueListStatusCode::Unknown,
            },
        };
    }
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
    if item.triage.as_ref().is_some_and(|triage| triage.stale) {
        return (
            IssueListReasonCode::Stale,
            "판정의 근거가 바뀌어 다시 확인해야 합니다.",
        );
    }
    if observation.remote_state == IssueRemoteState::Closed {
        return (
            IssueListReasonCode::RemoteClosed,
            "원격 이슈가 닫혀 있습니다.",
        );
    }
    if let Some(review) = pending_delivery_review(item) {
        return match review.reasons[0] {
            IssueDeliveryRiskReason::ProtectedPath => (
                IssueListReasonCode::ProtectedPath,
                "보호 경로 변경으로 결과 제출 승인을 기다립니다.",
            ),
            IssueDeliveryRiskReason::DiffTooLarge => (
                IssueListReasonCode::DiffTooLarge,
                "변경량 한도 초과로 결과 제출 승인을 기다립니다.",
            ),
            IssueDeliveryRiskReason::BinaryDiff => (
                IssueListReasonCode::BinaryDiff,
                "바이너리 변경으로 결과 제출 승인을 기다립니다.",
            ),
        };
    }
    if item.start_budget_wait.is_some() {
        return (
            IssueListReasonCode::Triage,
            "사용자 예산 한도에 도달해 작업 시작을 기다립니다.",
        );
    }
    if let Some(triage) = &item.triage
        && let Some(reason) = triage.assessment.reasons().first()
    {
        return (
            IssueListReasonCode::Triage,
            super::triage::reason_summary(*reason),
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

fn pending_delivery_review(item: &IssueCollectionItem) -> Option<&IssueDeliveryDiffVerdict> {
    if item.tracked.as_ref().is_some_and(|tracked| {
        matches!(
            tracked.state,
            IssueAutomationState::Done | IssueAutomationState::Cancelled
        ) || tracked.flags.held
            || tracked.flags.stale
            || tracked.flags.unknown
    }) || item.triage.as_ref().is_some_and(|triage| triage.stale)
    {
        return None;
    }
    let Some(IssueSourceRead::Observed(observation)) = &item.current else {
        return None;
    };
    if observation.remote_state != IssueRemoteState::Open
        || !matches!(&observation.content, IssueSourceContent::Shared(_))
    {
        return None;
    }
    let work = item.work.as_ref().filter(|work| !work.delivery_granted)?;
    item.open_decisions.iter().find_map(|stored| {
        let request = &stored.request;
        let definition = request.definition();
        (request.state() == IssueDecisionState::Open
            && request.issue() == &item.issue
            && definition.kind == IssueDecisionKind::RiskException
            && definition.gate == IssueAutomationGate::Delivery
            && definition.basis.work_order_id.as_deref() == Some(work.work_order_id.as_str())
            && definition.basis.delivery.is_some())
        .then_some(definition.delivery_review.as_ref())
        .flatten()
        .filter(|review| review.requires_human())
    })
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
