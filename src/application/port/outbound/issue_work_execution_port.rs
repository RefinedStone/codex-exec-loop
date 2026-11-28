//! 이슈 자동화와 작업 실행기를 잇는 중립 경계다. 런타임 객체를 공유하지 않는다.
use crate::domain::issue_automation::coordinator::{IssueWorkBlockReason, IssueWorkOrder};
use crate::domain::issue_automation::decision::IssueDecisionBasis;
use crate::domain::issue_automation::delivery::IssueDeliveryDiff;
pub use crate::domain::issue_automation::execution::IssueWorkDeliveryAuthority;
use crate::domain::issue_automation::execution::{
    IssueExecutionQuestion, IssueExecutionTerminalClass, IssueExecutionTerminalRecord,
};
use crate::domain::issue_automation::work_control::{
    IssueWorkControl, IssueWorkControlAction, IssueWorkInstruction, IssueWorkInstructionAdmission,
};
use anyhow::Result;

pub struct ConfirmedIssueControl<'a> {
    pub work_order_id: &'a str,
    pub command_id: &'a str,
    pub action: crate::domain::issue_automation::work_control::IssueWorkControlAction,
    pub expected_epoch: u64,
    pub authorize: &'a dyn Fn() -> Result<bool>,
}
use sha2::{Digest, Sha256};

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
    /// Host가 실제 frozen 범위를 판정한 별도 Delivery 근거다.
    pub delivery_review: Option<IssueDecisionBasis>,
    /// Dispatch has no delivery authority; delivery explicitly selects Human or Auto.
    pub delivery_authority: Option<IssueWorkDeliveryAuthority>,
}

/// 이미 공유 정책을 통과한 답변이다. worker 원문이나 외부 확인 증거를 받지 않는다.
#[derive(Clone, PartialEq, Eq)]
pub struct IssueWorkAnswerRequest {
    pub authorization_epoch: u64,
    pub work_order_id: String,
    pub command_id: String,
    pub question: IssueExecutionQuestion,
    pub message: String,
}

impl std::fmt::Debug for IssueWorkAnswerRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssueWorkAnswerRequest")
            .field("work_order_id", &self.work_order_id)
            .field("command_id", &self.command_id)
            .field("question_id", &self.question.question_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkAnswerReceipt {
    pub command_id: String,
    pub question_id: String,
    pub delivery: IssueWorkAnswerDelivery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueWorkAnswerDelivery {
    /// 아직 외부 action을 claim하지 않은 영속 대기열이다.
    Queued,
    /// 외부 action을 claim했다. 확인 ACK가 없으며 자동 재전송하지 않는다.
    Unknown,
    /// exact thread의 turn/start ACK와 새 lease generation을 영속화했다.
    Accepted {
        thread_id: String,
        turn_id: String,
        lease_generation: String,
    },
    /// 외부 action 전의 확정 거절이다. reason은 고정 사유 코드이며 RPC 원문을 담지 않는다.
    RejectedBeforeStart { reason: String },
}

/// 이미 관측한 종료 사실을 기존 작업의 검증 후속 기록으로 복구하는 요청이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkRecoveryRequest {
    pub command_id: String,
    pub work_order_id: String,
    pub terminal: IssueExecutionTerminalRecord,
    pub authorization_epoch: u64,
    pub gate_revision: u64,
    pub policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkRecoveryReceipt {
    pub command_id: String,
    pub terminal_record_id: String,
    pub outcome: IssueWorkRecoveryOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueWorkRecoveryOutcome {
    Registered {
        validation_record_key: String,
    },
    /// 고정 사유 코드다. 사전 거절은 외부 action이나 성공 영수증을 만들지 않는다.
    Blocked {
        reason: String,
    },
}

/// 동일 work/종료 사실/복구 종류는 프로세스 재시작 뒤에도 같은 명령이다.
pub fn issue_work_recovery_command_id(
    work_order_id: &str,
    terminal: &IssueExecutionTerminalRecord,
) -> String {
    let classification = match terminal.classification {
        IssueExecutionTerminalClass::ReworkNeeded => "rework_needed",
        IssueExecutionTerminalClass::ObserveAgain => "observe_again",
        IssueExecutionTerminalClass::ContractRecovery => "contract_recovery",
        IssueExecutionTerminalClass::IdentityConflict => "identity_conflict",
        IssueExecutionTerminalClass::PullRequestClosed => "pull_request_closed",
        IssueExecutionTerminalClass::QueueRecovery => "queue_recovery",
    };
    let mut digest = Sha256::new();
    for part in [work_order_id, terminal.record_id.as_str(), classification] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("issue-validation-recovery-v1-{:x}", digest.finalize())
}

impl IssueWorkRecoveryRequest {
    pub fn validate(&self) -> Result<()> {
        if self.command_id != issue_work_recovery_command_id(&self.work_order_id, &self.terminal)
            || self.work_order_id.trim().is_empty()
            || self.work_order_id.len() > 512
            || self.terminal.record_id.trim().is_empty()
            || self.terminal.record_id.len() > 512
            || self.policy_version.trim().is_empty()
            || self.policy_version.len() > 512
            || [
                self.work_order_id.as_str(),
                self.terminal.record_id.as_str(),
                self.policy_version.as_str(),
            ]
            .iter()
            .any(|value| value.chars().any(char::is_control))
        {
            anyhow::bail!("invalid work validation recovery identity");
        }
        Ok(())
    }
}

pub trait IssueWorkExecutionPort: Send + Sync {
    /// 기존 종료를 변경하지 않고 후속 검증과 영속 영수증을 한 번만 등록한다.
    fn recover_execution(
        &self,
        _workspace: &str,
        _request: &IssueWorkRecoveryRequest,
    ) -> Result<IssueWorkRecoveryReceipt> {
        anyhow::bail!("work validation recovery is unavailable")
    }

    /// 동일 command_id의 payload는 불변이다. 반환 손실 후에는 receipt 조회만 허용한다.
    fn answer_question(
        &self,
        _workspace: &str,
        _request: &IssueWorkAnswerRequest,
    ) -> Result<IssueWorkAnswerReceipt> {
        anyhow::bail!("work question continuation is unavailable")
    }

    /// Queued/Unknown을 포함한 기존 영수증만 조회한다. 실행이나 자동 재전송을 시작하지 않는다.
    fn question_answer_receipt(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _command_id: &str,
    ) -> Result<Option<IssueWorkAnswerReceipt>> {
        Ok(None)
    }

    /// 실행 한도를 영속 실행 admission에 적용한다. 지원하지 않는 실행기는 진행하지 않는다.
    fn execution_limit(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _max_attempts: u32,
    ) -> Result<()> {
        anyhow::bail!("work execution limits are unavailable")
    }

    fn permit_gate_revision(&self, _workspace: &str, _work_order_id: &str) -> Result<u64> {
        Ok(0)
    }

    /// Read-only execution facts. None means this executor has no tracking projection.
    fn observe_execution(
        &self,
        _workspace: &str,
        _work_order_id: &str,
    ) -> Result<Option<crate::domain::issue_automation::execution::IssueWorkExecutionObservation>>
    {
        Ok(None)
    }

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
    fn control_command_result(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _command_id: &str,
        _action: IssueWorkControlAction,
    ) -> Result<Option<IssueWorkControl>> {
        anyhow::bail!("work intervention control history is unavailable")
    }
    fn apply_confirmed_control(
        &self,
        _workspace: &str,
        _request: &ConfirmedIssueControl<'_>,
    ) -> Result<Option<IssueWorkControl>> {
        anyhow::bail!("confirmed issue control is unavailable")
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

    /// 이 branch 에서 지금 실행 중인 이슈 작업의 (work_order_id, 좌표)이다.
    /// lease·thread 가 하나가 아니거나 이미 대기 중이면 None 이다.
    fn asking_worker(
        &self,
        _workspace: &str,
        _branch: &str,
    ) -> Result<
        Option<(
            String,
            crate::domain::issue_automation::agent_question::IssueAgentAsk,
        )>,
    > {
        Ok(None)
    }

    /// 그 좌표의 task 를 awaiting_user 로 둔다. 턴이 끝나도 thread 와 worktree 가 남는다.
    /// lease generation 이 바뀌었으면 false 이다. 이미 기다리는 task 는 true 이다.
    fn await_question(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _ask: &crate::domain::issue_automation::agent_question::IssueAgentAsk,
    ) -> Result<bool> {
        anyhow::bail!("agent questions are unavailable")
    }

    /// Read-only, uniquely attributable worker conversation. Never creates or resumes a turn.
    fn conversation_thread_id(
        &self,
        _workspace: &str,
        _work_order_id: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    fn snapshot(&self, workspace: &str, work_order_id: &str) -> Result<IssueWorkExecutionSnapshot>;
    /// 로컬 queue에 이미 고정한 source만 읽는다. 원격 base fetch나 worker 시작은 없다.
    fn frozen_delivery_source(
        &self,
        _workspace: &str,
        _work_order_id: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }
    /// 현재 작업/lease에 속한 고정 base부터 source까지 빠짐없이 읽는다.
    fn delivery_diff(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _expected_source_sha: &str,
    ) -> Result<IssueDeliveryDiff> {
        anyhow::bail!("work delivery diff inspection is unavailable")
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_command_identity_is_stable_and_binds_work_terminal_and_kind() {
        let terminal = IssueExecutionTerminalRecord {
            record_id: "terminal-1".into(),
            classification: IssueExecutionTerminalClass::ObserveAgain,
        };
        let first = issue_work_recovery_command_id("work-1", &terminal);
        assert_eq!(
            first,
            issue_work_recovery_command_id("work-1", &terminal.clone())
        );
        assert_ne!(first, issue_work_recovery_command_id("work-2", &terminal));
        let mut changed = terminal.clone();
        changed.record_id = "terminal-2".into();
        assert_ne!(first, issue_work_recovery_command_id("work-1", &changed));
        changed = terminal.clone();
        changed.classification = IssueExecutionTerminalClass::QueueRecovery;
        assert_ne!(first, issue_work_recovery_command_id("work-1", &changed));
        let mut request = IssueWorkRecoveryRequest {
            command_id: first,
            work_order_id: "work-1".into(),
            terminal,
            authorization_epoch: 0,
            gate_revision: 1,
            policy_version: "policy".into(),
        };
        request.validate().unwrap();
        request.terminal = changed;
        assert!(request.validate().is_err());
    }
}
