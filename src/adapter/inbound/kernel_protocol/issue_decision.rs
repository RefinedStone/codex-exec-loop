//! 결정 요청의 공유본 조회와 메타데이터 변경 알림.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::application::port::inbound::stored_command_port::{
    AskIssueDecisionOption, AskIssueDecisionRequest,
};
use crate::domain::issue_automation::agent_question::{
    MAX_AGENT_QUESTION_BYTES, WORKER_REPLY_OPTION,
};
use crate::domain::issue_automation::arbiter::IssueArbiterWaitReason;
use crate::domain::issue_automation::decision::{
    IssueDecisionFact, IssueDecisionKind, IssueDecisionState, IssueDecisionTimeout,
    MAX_ISSUE_DECISION_ID_BYTES, MAX_ISSUE_DECISION_NOTE_BYTES, MAX_ISSUE_DECISION_OPTION_BYTES,
    MAX_ISSUE_DECISION_OPTIONS, StoredIssueDecision,
};
use crate::domain::issue_automation::delivery::{
    IssueDeliveryDiffVerdict, IssueDeliveryRiskReason,
};
use crate::domain::kernel_event::{
    IssueDecisionBasisRecord, IssueDecisionFactRecord, IssueDecisionGateRecord,
    IssueDecisionSubjectRecord,
};

use super::{KernelProtocolError, ReasonCode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionRequestState {
    pub id: String,
    pub subject: IssueDecisionSubjectRecord,
    pub gate: IssueDecisionGateRecord,
    pub kind: IssueDecisionKind,
    /// 정책 경계에서 이미 가린 공유 문장이다.
    pub question: String,
    pub options: Vec<IssueDecisionOptionState>,
    pub recommended: Option<String>,
    pub basis: IssueDecisionBasisRecord,
    /// 이 결정에 고정된 실제 변경 범위의 판정이다. 최신 실행 상태로 덮어쓰지 않는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_review: Option<IssueDeliveryReviewState>,
    /// 결정의 정의나 판을 바꾸지 않는 현재 중재 대기 표시다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arbiter_wait: Option<IssueArbiterWaitState>,
    pub on_timeout: IssueDecisionTimeout,
    pub due_at: Option<String>,
    pub state: IssueDecisionState,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionOptionState {
    pub id: String,
    pub label: String,
    pub effect: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueArbiterWaitState {
    pub reason: IssueArbiterWaitReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDeliveryReviewState {
    pub changed_files: u64,
    /// 바이너리가 포함된 변경의 전체 줄 수는 미확인이다.
    pub changed_lines: Option<u64>,
    pub reasons: Vec<IssueDeliveryRiskReason>,
}

impl From<&IssueDeliveryDiffVerdict> for IssueDeliveryReviewState {
    fn from(review: &IssueDeliveryDiffVerdict) -> Self {
        Self {
            changed_files: review.changed_files,
            changed_lines: review.changed_lines,
            reasons: review.reasons.clone(),
        }
    }
}

impl From<&StoredIssueDecision> for IssueDecisionRequestState {
    fn from(stored: &StoredIssueDecision) -> Self {
        let request = &stored.request;
        let definition = request.definition();
        Self {
            id: request.id().as_str().to_owned(),
            subject: (&definition.subject).into(),
            gate: definition.gate.into(),
            kind: definition.kind,
            question: definition.question.as_str().to_owned(),
            options: definition
                .options
                .iter()
                .map(|option| IssueDecisionOptionState {
                    id: option.id.clone(),
                    label: option.label.as_str().to_owned(),
                    effect: option.effect.as_str().to_owned(),
                })
                .collect(),
            recommended: definition.recommended.clone(),
            basis: (&definition.basis).into(),
            delivery_review: definition.delivery_review.as_ref().map(Into::into),
            arbiter_wait: None,
            on_timeout: definition.on_timeout,
            due_at: definition.due_at.clone(),
            state: request.state(),
            revision: stored.revision,
        }
    }
}

/// commit에서 돌려받은 좌표를 알린다. 수신자는 같은 catalog 판으로 issue를 다시 읽는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueDecisionChangedState {
    pub decision: IssueDecisionFactRecord,
    pub revision: u64,
    pub reason: Option<ReasonCode>,
}

impl IssueDecisionChangedState {
    pub fn from_fact(fact: &IssueDecisionFact, revision: u64) -> Self {
        Self {
            decision: fact.into(),
            revision,
            reason: (fact.state == IssueDecisionState::Superseded)
                .then_some(ReasonCode::DecisionSuperseded),
        }
    }
}

pub(super) fn validate_answer_input(
    decision_id: &str,
    option: &str,
    note: Option<&str>,
) -> Result<(), KernelProtocolError> {
    // 입력 원문을 오류 설명에 넣지 않는다. 장부는 이 검사를 신뢰하지 않고 다시 검증한다.
    // JSON Schema length는 글자 수이므로 여기서는 UTF-8 바이트 상한도 검사한다.
    if !valid_identifier(decision_id, MAX_ISSUE_DECISION_ID_BYTES)
        || !valid_identifier(option, MAX_ISSUE_DECISION_OPTION_BYTES)
        || note.is_some_and(|value| {
            value.len() > MAX_ISSUE_DECISION_NOTE_BYTES || value.contains('\0')
        })
    {
        return Err(KernelProtocolError::invalid_params(
            "invalid issue decision answer",
        ));
    }
    Ok(())
}

/// 에이전트 질문의 입력 모양이다. 글의 공유본 처리는 서비스가 한다. 원문을 오류에 넣지 않는다.
pub(super) fn ask_input(
    branch: String,
    question: String,
    options: Vec<IssueQuestionOptionFields>,
    recommended: Option<String>,
) -> Result<AskIssueDecisionRequest, KernelProtocolError> {
    let invalid = || KernelProtocolError::invalid_params("invalid issue question");
    let mut ids = std::collections::BTreeSet::new();
    if !valid_identifier(&branch, 256)
        || !valid_text(&question, MAX_AGENT_QUESTION_BYTES)
        || !(2..=MAX_ISSUE_DECISION_OPTIONS).contains(&options.len())
        || options.iter().any(|option| {
            !valid_identifier(&option.id, MAX_ISSUE_DECISION_OPTION_BYTES)
                || option.id == WORKER_REPLY_OPTION
                || !valid_text(&option.label, 256)
                || !ids.insert(option.id.as_str())
        })
        || recommended.as_deref().is_some_and(|id| !ids.contains(id))
    {
        return Err(invalid());
    }
    Ok(AskIssueDecisionRequest {
        branch,
        question,
        options: options
            .into_iter()
            .map(|option| AskIssueDecisionOption {
                id: option.id,
                label: option.label,
            })
            .collect(),
        recommended,
    })
}

/// 에이전트 질문의 선택지 하나이다. 효과는 서버가 고정한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueQuestionOptionFields {
    #[schemars(length(min = 1, max = 128), regex(pattern = super::NON_BLANK))]
    pub id: String,
    #[schemars(length(min = 1, max = 256), regex(pattern = super::NON_BLANK))]
    pub label: String,
}

fn valid_text(value: &str, maximum_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_bytes
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn valid_identifier(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value.trim() == value
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

#[cfg(test)]
mod tests;
