//! 상세 화면의 개입 제어와 공개 가능한 결정 이력 및 대화 경로다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::IssueDecisionRequestState;
use super::issue_automation::IssueSharedCopyState;
use crate::domain::issue_automation::interaction::{
    IssueActionCommand, IssueActionProjection, IssueControlProjection, IssueConversationTarget,
    IssueDecisionHistory, IssueDecisionResolutionCode,
};
use crate::domain::kernel_event::IssueDecisionFactRecord;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueControlState {
    pub tracked: bool,
    pub held: bool,
    pub taken_over: bool,
    pub cancelled: bool,
    pub authorization_epoch: u64,
}

impl From<&IssueControlProjection> for IssueControlState {
    fn from(control: &IssueControlProjection) -> Self {
        Self {
            tracked: control.tracked,
            held: control.held,
            taken_over: control.taken_over,
            cancelled: control.cancelled,
            authorization_epoch: control.authorization_epoch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueActionState {
    pub command: IssueActionCommand,
    pub enabled: bool,
    pub reason: Option<String>,
}

impl From<&IssueActionProjection> for IssueActionState {
    fn from(action: &IssueActionProjection) -> Self {
        Self {
            command: action.command,
            enabled: action.enabled,
            reason: action.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionHistoryState {
    pub decision: IssueDecisionFactRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<IssueDecisionRequestState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_reason: Option<IssueDecisionResolutionReasonState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionResolutionReasonState {
    pub code: IssueDecisionResolutionCode,
    pub summary: String,
}

impl From<&IssueDecisionHistory> for IssueDecisionHistoryState {
    fn from(history: &IssueDecisionHistory) -> Self {
        Self {
            decision: (&history.decision).into(),
            request: history.request.as_ref().map(Into::into),
            resolution_reason: Some(IssueDecisionResolutionReasonState {
                code: history.resolution,
                summary: match history.resolution {
                    IssueDecisionResolutionCode::Answered => "이미 답변한 결정 요청입니다.",
                    IssueDecisionResolutionCode::Expired => {
                        "응답 기한이 지나 더 이상 답변할 수 없습니다."
                    }
                    IssueDecisionResolutionCode::Withdrawn => {
                        "요청이 철회되어 더 이상 답변할 수 없습니다."
                    }
                    IssueDecisionResolutionCode::BasisChanged => {
                        "이슈나 정책의 기준이 바뀌어 새 결정 요청이 필요합니다."
                    }
                }
                .to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueConversationState {
    Thread { thread_id: String },
    Draft { shared_copy: IssueSharedCopyState },
    Unavailable { reason: String },
}

impl Default for IssueConversationState {
    fn default() -> Self {
        Self::Unavailable {
            reason: "안전한 대화 연결을 확인할 수 없습니다.".to_owned(),
        }
    }
}

impl From<&IssueConversationTarget> for IssueConversationState {
    fn from(conversation: &IssueConversationTarget) -> Self {
        match conversation {
            IssueConversationTarget::Thread { thread_id } => Self::Thread {
                thread_id: thread_id.clone(),
            },
            IssueConversationTarget::Draft { shared_copy } => Self::Draft {
                shared_copy: IssueSharedCopyState {
                    title: shared_copy.title.clone(),
                    body: shared_copy.body.clone(),
                },
            },
            IssueConversationTarget::Unavailable => Self::default(),
        }
    }
}
