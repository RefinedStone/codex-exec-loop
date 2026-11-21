//! 이미 관측한 종료 사실을 기존 작업의 검증 후속 기록으로 복구하는 경계다.
use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::domain::issue_automation::execution::{
    IssueExecutionTerminalClass, IssueExecutionTerminalRecord,
};

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
