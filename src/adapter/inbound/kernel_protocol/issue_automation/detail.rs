//! 같은 저장소의 안정 ID 한 건을 조회한다. 없음과 저장 읽기 실패는 다른 상태다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::issue_interaction::{
    IssueActionState, IssueControlState, IssueConversationState, IssueDecisionHistoryState,
};
use super::super::{IssueDecisionRequestState, KernelProtocolError, KernelQuery};
use super::{IssueListItem, IssueTriageObserveReportState};
use crate::domain::issue_automation::collection::{
    IssueCollectionDetail, IssueFeedbackDetail, IssueFeedbackDetailReason,
};
use crate::domain::issue_automation::decision::IssueDecisionState;
use crate::domain::issue_work::IssueReference;

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

    pub fn from_domain(detail: &IssueCollectionDetail) -> Self {
        let report = detail.report.as_ref().map(Into::into);
        match &detail.item {
            Some(item) => Self::Ready(Box::new(IssueDetailState {
                revision: detail.revision,
                item: item.into(),
                open_decisions: item
                    .open_decisions
                    .iter()
                    .filter(|stored| stored.request.state() == IssueDecisionState::Open)
                    .map(Into::into)
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
