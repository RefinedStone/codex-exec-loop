//! 이슈 자동화와 작업 실행기를 잇는 중립 경계다. 런타임 객체를 공유하지 않는다.
use crate::domain::issue_automation::coordinator::{IssueWorkBlockReason, IssueWorkOrder};
use crate::domain::issue_automation::decision::IssueDecisionBasis;
use crate::domain::issue_automation::work_control::{
    IssueWorkControl, IssueWorkControlAction, IssueWorkInstruction, IssueWorkInstructionAdmission,
};
use anyhow::Result;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueWorkExecutionSnapshot {
    pub task_ids: Vec<String>,
    /// 실행기와 같은 remote/base 해석으로 새로 검증한 dispatch source다.
    pub dispatch_source: Option<String>,
    /// host가 완료 결과에 고정한 SHA다. 임의 branch HEAD로 대신하지 않는다.
    pub frozen_source: Option<String>,
    pub saved_source: Option<String>,
    pub saved_gate_revision: u64,
    pub saved_authorization_epoch: u64,
    pub blocked_reason: Option<IssueWorkBlockReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkPermitRequest {
    pub authorization_epoch: u64,
    pub work_order_id: String,
    pub approval: IssueDecisionBasis,
    pub source_sha: String,
    pub gate_revision: u64,
    pub delivery: bool,
}

pub trait IssueWorkExecutionPort: Send + Sync {
    fn control(&self, _workspace: &str, _work_order_id: &str) -> Result<IssueWorkControl> {
        Ok(IssueWorkControl::default())
    }
    fn apply_control(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _command_id: &str,
        _action: IssueWorkControlAction,
    ) -> Result<IssueWorkControl> {
        anyhow::bail!("work intervention control is unavailable")
    }
    fn queue_instruction(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _instruction: &IssueWorkInstruction,
    ) -> Result<IssueWorkInstructionAdmission> {
        anyhow::bail!("work instruction delivery is unavailable")
    }
    fn has_active_lease(&self, _workspace: &str, _work_order_id: &str) -> Result<bool> {
        Ok(false)
    }

    fn snapshot(&self, workspace: &str, work_order_id: &str) -> Result<IssueWorkExecutionSnapshot>;
    /// 기존 등록을 읽는다. 실제 adapter는 원격 fetch와 독립된 영속 조회를 구현한다.
    fn task_ids(&self, workspace: &str, work_order_id: &str) -> Result<Vec<String>> {
        Ok(self.snapshot(workspace, work_order_id)?.task_ids)
    }
    /// 영속 work_order_id로 멱등 등록한다. 반환 손실 뒤 재시도도 같은 task_ids다.
    fn submit(&self, workspace: &str, order: &IssueWorkOrder) -> Result<Vec<String>>;
    /// source/basis를 저장한 다음 단계별 허가를 만든다. integrate는 언제나 human review다.
    fn grant(&self, workspace: &str, request: &IssueWorkPermitRequest) -> Result<()>;
    fn revoke(&self, workspace: &str, work_order_id: &str) -> Result<()>;
}
