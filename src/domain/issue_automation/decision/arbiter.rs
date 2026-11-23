//! Arbiter 전이는 장부의 전용 finish 경계에서만 호출한다. 일반 답변 API에는 노출하지 않는다.
use chrono::{DateTime, SecondsFormat, Utc};

use super::{
    IssueDecisionAnswer, IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionError,
    IssueDecisionOutcome, IssueDecisionRequest, IssueDecisionResolution, IssueDecisionState,
};
use crate::domain::issue_automation::arbiter::{
    ISSUE_ARBITER_CHANNEL, IssueArbiterProposal, IssueArbiterSettings, arbiter_eligible,
};
use crate::domain::issue_automation::triage::IssueTriageAssessment;

impl IssueDecisionRequest {
    pub(crate) fn answer_arbiter(
        &self,
        current_basis: &IssueDecisionBasis,
        current: &IssueTriageAssessment,
        settings: &IssueArbiterSettings,
        proposal: &IssueArbiterProposal,
        now_unix_ms: u64,
    ) -> Result<IssueDecisionResolution, IssueDecisionError> {
        if self.state() != IssueDecisionState::Open {
            return Ok(IssueDecisionResolution {
                request: self.clone(),
                outcome: IssueDecisionOutcome::AlreadyResolved,
                changed: false,
            });
        }
        current_basis.validate()?;
        let mut record = self.to_record();
        if self.definition().basis != *current_basis {
            record.state = IssueDecisionState::Superseded;
            let request = Self::hydrate(record)?;
            request.validate_successor(self)?;
            return Ok(IssueDecisionResolution {
                request,
                outcome: IssueDecisionOutcome::Superseded,
                changed: true,
            });
        }
        settings
            .validate()
            .map_err(|_| IssueDecisionError::InvalidState)?;
        if !arbiter_eligible(self, current, now_unix_ms) || proposal.wait_reason(settings).is_some()
        {
            return Err(IssueDecisionError::UnsupportedAnswerOrigin);
        }
        let now = i64::try_from(now_unix_ms)
            .ok()
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .ok_or(IssueDecisionError::InvalidField { field: "now" })?;
        record.state = IssueDecisionState::Answered;
        record.answer = Some(IssueDecisionAnswer {
            option: proposal.decision.clone(),
            note: proposal.note.clone(),
            by: IssueDecisionAnswerBy::Arbiter,
            confirmation: None,
            caller: None,
            channel: ISSUE_ARBITER_CHANNEL.to_owned(),
            at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        });
        let request = Self::hydrate(record)?;
        request.validate_successor(self)?;
        Ok(IssueDecisionResolution {
            request,
            outcome: IssueDecisionOutcome::Answered,
            changed: true,
        })
    }
}
