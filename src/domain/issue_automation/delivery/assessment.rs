//! 전달 판정은 Start 승인을 바꾸지 않고 work와 실제 commit 범위에 묶인다.
use serde::{Deserialize, Serialize};

use super::IssueDeliveryDiffVerdict;
use crate::domain::issue_automation::MAX_ISSUE_AUTOMATION_REVISION;
use crate::domain::issue_automation::coordinator::{IssueWorkCheckpoint, issue_work_order_id};
use crate::domain::issue_automation::decision::{IssueDecisionBasis, IssueDecisionDefinition};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::triage::IssueTriageAssessment;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDeliveryAssessment {
    pub basis: IssueDecisionBasis,
    /// 실행 범위별 gate revision과 별개인 현재 triage의 메타데이터 fence다.
    pub triage_gate_revision: u64,
    pub verdict: IssueDeliveryDiffVerdict,
}

impl IssueDeliveryAssessment {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.basis
            .validate()
            .map_err(|_| "invalid delivery assessment basis")?;
        if self.basis.work_order_id.is_none()
            || self.basis.field_hash.is_none()
            || self.basis.delivery.is_none()
            || self.triage_gate_revision == 0
            || self.triage_gate_revision > MAX_ISSUE_AUTOMATION_REVISION
            || self.basis.gate_revision <= self.triage_gate_revision
        {
            return Err("invalid delivery assessment binding");
        }
        self.verdict
            .validate()
            .map_err(|_| "invalid delivery diff verdict")
    }

    pub fn matches_work(&self, work: &IssueWorkCheckpoint) -> bool {
        self.validate().is_ok()
            && self.basis.work_order_id.as_deref() == Some(&work.order.work_order_id)
            && self.basis.gate_revision == work.delivery_revision
            && self.basis.field_hash.as_ref() == Some(&work.order.field_hash)
            && self.basis.authorization_epoch
                == work.current_authorization_basis().authorization_epoch
    }

    pub fn matches_triage(&self, triage: &IssueTriageAssessment, authorization_epoch: u64) -> bool {
        let Ok(mut current) = IssueDecisionBasis::from_assessment(
            triage,
            IssueAutomationGate::Delivery,
            self.basis.work_order_id.clone(),
        ) else {
            return false;
        };
        if self.validate().is_err()
            || current.gate_revision != self.triage_gate_revision
            || self.basis.work_order_id.as_deref()
                != Some(issue_work_order_id(&triage.basis().issue).as_str())
        {
            return false;
        }
        current.authorization_epoch = authorization_epoch;
        current.gate_revision = self.basis.gate_revision;
        current.delivery = self.basis.delivery.clone();
        current == self.basis
    }

    pub fn matches_decision(&self, definition: &IssueDecisionDefinition) -> bool {
        definition.is_delivery_risk_exception()
            && self.verdict.requires_human()
            && self.basis == definition.basis
            && definition.delivery_review.as_ref() == Some(&self.verdict)
    }
}
