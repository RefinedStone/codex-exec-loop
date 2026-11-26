use super::PlanningWorkOrderRequest;
use crate::domain::planning::{TaskAcceptanceCriterion, normalize_task_acceptance};
use anyhow::{Result, bail};

/// 확인된 외부 승인으로부터 composition이 만드는 비직렬화 capability다.
/// 일반 System mutation의 사용자 기준 소유권 검사는 바꾸지 않는다.
pub(crate) struct VerifiedWorkOrderAcceptance {
    workspace: String,
    work_order_id: String,
    pub(super) receipt_digest: String,
    pub(super) criteria: Vec<TaskAcceptanceCriterion>,
}

impl VerifiedWorkOrderAcceptance {
    pub(crate) fn from_verified_receipt(
        workspace: &str,
        work_order_id: &str,
        receipt_digest: String,
        criteria: Vec<TaskAcceptanceCriterion>,
    ) -> Result<Self> {
        let normalized = normalize_task_acceptance(&criteria)?;
        if normalized.is_empty() || normalized != criteria || !valid_receipt(&receipt_digest) {
            bail!("invalid confirmed work-order acceptance");
        }
        Ok(Self {
            workspace: workspace.into(),
            work_order_id: work_order_id.into(),
            receipt_digest,
            criteria,
        })
    }

    pub(super) fn require_target(&self, request: &PlanningWorkOrderRequest) -> Result<()> {
        if self.workspace != request.workspace_directory
            || self.work_order_id != request.work_order_id
            || request.origin != "issue_automation"
            || request.input.acceptance.is_some()
        {
            bail!("confirmed acceptance belongs to another work-order admission");
        }
        Ok(())
    }
}

pub(crate) fn valid_receipt(receipt: &str) -> bool {
    receipt.len() == 64
        && receipt
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
