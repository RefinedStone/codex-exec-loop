//! 결정 요청의 공유본 조회와 메타데이터 변경 알림.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::decision::{
    IssueDecisionFact, IssueDecisionKind, IssueDecisionState, IssueDecisionTimeout,
    MAX_ISSUE_DECISION_ID_BYTES, MAX_ISSUE_DECISION_NOTE_BYTES, MAX_ISSUE_DECISION_OPTION_BYTES,
    StoredIssueDecision,
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
