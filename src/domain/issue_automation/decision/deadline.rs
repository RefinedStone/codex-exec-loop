//! 주입된 시각으로 결정 마감을 평가한다. 모델 호출이나 실행 권한은 만들지 않는다.
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::{
    IssueAutomationGateAction, IssueDecisionAnswer, IssueDecisionAnswerBy, IssueDecisionBasis,
    IssueDecisionError, IssueDecisionFact, IssueDecisionKind, IssueDecisionOutcome,
    IssueDecisionRequest, IssueDecisionResolution, IssueDecisionState, IssueDecisionTimeout,
    StoredIssueDecision,
};

pub const ISSUE_DECISION_TIMEOUT_CHANNEL: &str = "issue-timeout";

/// 요청 판은 알림 처리와 독립적이다. 알림이 와도 작성 중인 사람 답변의 판은 유지된다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDecisionDeadline {
    pub record: StoredIssueDecision,
    pub remind_at: Option<String>,
    pub reminded: bool,
}

/// 결정별 영속 알림 사실이다. 질문, 답변 note, caller 등 공유 본문을 복사하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDecisionReminder {
    pub event_id: String,
    pub fact: IssueDecisionFact,
    pub due_at: String,
    pub reminded_at: String,
    pub revision: u64,
}

impl IssueDecisionRequest {
    /// None은 변경할 일이 없는 wait이다. 이미 확정된 답과 낡은 기준을 먼저 처리한다.
    pub(crate) fn timeout_for_gate(
        &self,
        current_basis: &IssueDecisionBasis,
        gate_action: IssueAutomationGateAction,
        now_unix_ms: u64,
    ) -> Result<Option<IssueDecisionResolution>, IssueDecisionError> {
        if self.state() != IssueDecisionState::Open {
            return Ok(Some(IssueDecisionResolution {
                request: self.clone(),
                outcome: IssueDecisionOutcome::AlreadyResolved,
                changed: false,
            }));
        }
        current_basis.validate()?;
        if self.definition().basis != *current_basis {
            let mut record = self.to_record();
            record.state = IssueDecisionState::Superseded;
            let request = Self::hydrate(record)?;
            request.validate_successor(self)?;
            return Ok(Some(IssueDecisionResolution {
                request,
                outcome: IssueDecisionOutcome::Superseded,
                changed: true,
            }));
        }
        let definition = self.definition();
        let requires_human = definition.is_delivery_risk_exception()
            || definition.kind == IssueDecisionKind::CloseApproval
            || (definition.basis.work_order_id.is_some()
                && matches!(
                    definition.kind,
                    IssueDecisionKind::AgentQuestion | IssueDecisionKind::FailureRecovery
                ));
        if requires_human
            || gate_action != IssueAutomationGateAction::TimeoutDefault
            || definition.on_timeout != IssueDecisionTimeout::Default
        {
            return Ok(None);
        }
        let Some(due_at) = definition.due_at.as_deref() else {
            return Ok(None);
        };
        let Some(recommended) = definition.recommended.as_ref() else {
            return Ok(None);
        };
        let now = i64::try_from(now_unix_ms)
            .ok()
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .ok_or(IssueDecisionError::InvalidField { field: "now" })?;
        let due = DateTime::parse_from_rfc3339(due_at)
            .map_err(|_| IssueDecisionError::InvalidField { field: "due_at" })?;
        if now < due {
            return Ok(None);
        }
        let mut record = self.to_record();
        record.state = IssueDecisionState::Answered;
        record.answer = Some(IssueDecisionAnswer {
            option: recommended.clone(),
            note: None,
            by: IssueDecisionAnswerBy::Default,
            confirmation: None,
            caller: None,
            channel: ISSUE_DECISION_TIMEOUT_CHANNEL.to_owned(),
            at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        });
        let request = Self::hydrate(record)?;
        request.validate_successor(self)?;
        Ok(Some(IssueDecisionResolution {
            request,
            outcome: IssueDecisionOutcome::Answered,
            changed: true,
        }))
    }
}
