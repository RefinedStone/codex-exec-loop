//! 같은 저장소의 안정 ID 한 건을 조회한다. 없음과 저장 읽기 실패는 다른 상태다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::issue_interaction::{
    IssueActionState, IssueControlState, IssueConversationState, IssueDecisionHistoryState,
};
use super::super::{IssueDecisionRequestState, KernelProtocolError, KernelQuery};
use super::{IssueListItem, IssueTriageObserveReportState};
use crate::domain::issue_automation::collection::{
    IssueCollectionDetail, IssueExternalState, IssueFeedbackDetail, IssueFeedbackDetailReason,
};
use crate::domain::issue_automation::decision::{IssueDecisionState, IssueDecisionTimeout};
use crate::domain::issue_automation::execution::IssueExecutionIntegration;
use crate::domain::issue_work::IssueReference;

mod execution;
pub use execution::IssueExecutionDetailState;

const MAX_ISSUE_COORDINATE_BYTES: usize = 4096;
const MAX_ISSUE_REFERENCE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueState {
    Unavailable {},
    NotFound {
        revision: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report: Option<IssueTriageObserveReportState>,
    },
    Ready(Box<IssueDetailState>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDetailState {
    pub revision: u64,
    /// 이 응답을 조회한 서버 UTC 시각이다. 결정의 revision과 독립적이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_unix_ms: Option<u64>,
    pub item: IssueListItem,
    #[serde(default)]
    pub open_decisions: Vec<IssueDecisionRequestState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<IssueControlState>,
    #[serde(default)]
    pub actions: Vec<IssueActionState>,
    #[serde(default)]
    pub decision_history: Vec<IssueDecisionHistoryState>,
    #[serde(default)]
    pub conversation: IssueConversationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<IssueTriageObserveReportState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<IssueFeedbackDetailState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<IssueExecutionDetailState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline: Option<super::timeline::IssueTimelineState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueFeedbackDetailState {
    pub revision: u64,
    pub publication_attempts: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<IssueFeedbackDetailReasonState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration: Option<IssueExecutionIntegration>,
    #[serde(default)]
    pub external_state: IssueExternalStateRecord,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueExternalStateRecord {
    Open,
    Closed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueFeedbackDetailReasonState {
    FeedbackUnconfirmed,
}

impl From<&IssueFeedbackDetail> for IssueFeedbackDetailState {
    fn from(detail: &IssueFeedbackDetail) -> Self {
        Self {
            revision: detail.revision,
            publication_attempts: detail.publication_attempts,
            integration: detail.integration.clone(),
            external_state: match detail.external_state {
                IssueExternalState::Open => IssueExternalStateRecord::Open,
                IssueExternalState::Closed => IssueExternalStateRecord::Closed,
                IssueExternalState::Unknown => IssueExternalStateRecord::Unknown,
            },
            comment_id: detail.comment_id.clone(),
            reason: detail.reason.map(|reason| match reason {
                IssueFeedbackDetailReason::FeedbackUnconfirmed => {
                    IssueFeedbackDetailReasonState::FeedbackUnconfirmed
                }
            }),
        }
    }
}

impl IssueState {
    pub fn unavailable() -> Self {
        Self::Unavailable {}
    }

    pub fn with_as_of_unix_ms(mut self, at: Option<u64>) -> Self {
        if let Self::Ready(detail) = &mut self {
            detail.as_of_unix_ms = at;
        }
        self
    }

    pub fn from_domain(detail: &IssueCollectionDetail) -> Self {
        let report = detail.report.as_ref().map(Into::into);
        match &detail.item {
            Some(item) => Self::Ready(Box::new(IssueDetailState {
                revision: detail.revision,
                as_of_unix_ms: None,
                item: item.into(),
                open_decisions: item
                    .open_decisions
                    .iter()
                    .filter(|stored| stored.request.state() == IssueDecisionState::Open)
                    .map(|stored| {
                        let mut request = IssueDecisionRequestState::from(stored);
                        if request.kind == crate::domain::issue_automation::decision::IssueDecisionKind::StartApproval
                            && stored.request.definition().basis.work_order_id.is_none()
                            && let Some(crate::domain::issue_automation::source::IssueSourceRead::Observed(observed)) = &item.current
                            && let crate::domain::issue_automation::source::IssueSourceContent::Shared(snapshot) = &observed.content
                            && stored.request.definition().basis.field_hash.as_ref() == Some(&snapshot.basis.field_hash)
                        {
                            request.criteria_scenarios = crate::domain::issue_automation::triage::issue_acceptance_criteria(&snapshot.shared_copy.body).ok();
                        }
                        if stored.request.definition().on_timeout == IssueDecisionTimeout::Arbiter {
                            request.arbiter_wait = detail
                                .arbiter_waits
                                .iter()
                                .find(|wait| {
                                    wait.decision_id == *stored.request.id()
                                        && wait.decision_revision == stored.revision
                                        && wait.basis == stored.request.definition().basis
                                })
                                .map(|wait| super::super::issue_decision::IssueArbiterWaitState {
                                    reason: wait.reason,
                                });
                        }
                        request
                    })
                    .collect(),
                control: detail
                    .interaction
                    .as_ref()
                    .and_then(|interaction| interaction.control.as_ref())
                    .map(Into::into),
                actions: detail
                    .interaction
                    .as_ref()
                    .map(|interaction| interaction.actions.iter().map(Into::into).collect())
                    .unwrap_or_default(),
                decision_history: detail
                    .interaction
                    .as_ref()
                    .map(|interaction| {
                        interaction
                            .decision_history
                            .iter()
                            .map(Into::into)
                            .collect()
                    })
                    .unwrap_or_default(),
                conversation: detail
                    .interaction
                    .as_ref()
                    .map(|interaction| (&interaction.conversation).into())
                    .unwrap_or_default(),
                report,
                feedback: detail.feedback.as_ref().map(Into::into),
                execution: detail.execution.as_ref().map(Into::into),
                timeline: detail.timeline.as_ref().map(Into::into),
            })),
            None => Self::NotFound {
                revision: detail.revision,
                report,
            },
        }
    }
}

impl KernelQuery {
    /// 표시 번호/URL로 다시 찾지 않는다. 대표 workspace와 전체 안정 좌표를 그대로 검증한다.
    pub fn issue_reference(
        &self,
        workspace: &str,
    ) -> Result<Option<IssueReference>, KernelProtocolError> {
        let Self::Issue { issue } = self else {
            return Ok(None);
        };
        let reference: IssueReference = issue.clone().into();
        let coordinates = [
            reference.server_id.as_str(),
            reference.repository_id.as_str(),
            reference.issue_id.as_str(),
            reference.project_scope.workspace_dir.as_str(),
        ];
        if reference.project_scope.workspace_dir != workspace
            || coordinates.iter().any(|value| {
                value.is_empty()
                    || value.len() > MAX_ISSUE_COORDINATE_BYTES
                    || value.trim() != *value
                    || value.chars().any(char::is_control)
            })
            || coordinates.iter().map(|value| value.len()).sum::<usize>()
                > MAX_ISSUE_REFERENCE_BYTES
        {
            return Err(KernelProtocolError::invalid_params(
                "invalid issue reference",
            ));
        }
        Ok(Some(reference))
    }
}

#[cfg(test)]
mod tests;
