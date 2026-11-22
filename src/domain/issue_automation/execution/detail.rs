//! 저장된 실행 사실의 표시용 projection이다. 실행 권한이나 재시도 결정을 만들지 않는다.
use super::{
    IssueExecutionProgress, IssueExecutionRecord, IssueExecutionState, IssueExecutionTerminalClass,
};
use crate::domain::issue_automation::coordinator::{IssueWorkSummary, issue_work_order_id};
use crate::domain::issue_work::IssueReference;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueExecutionDetail {
    Ready(Box<IssueExecutionReadyDetail>),
    Unavailable {
        reason: IssueExecutionUnavailableReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueExecutionUnavailableReason {
    ReadFailed,
    WorkIdMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueExecutionReadyDetail {
    pub work_order_id: String,
    pub revision: u64,
    pub state: IssueExecutionState,
    pub state_label: String,
    pub execution_attempts: u32,
    /// 현재 정책을 읽지 못하면 None이다. 실행 차단용 제한값이나 기본값으로 대체하지 않는다.
    pub execution_attempt_limit: Option<u32>,
    pub verification_attempts: u32,
    pub review_pull_request_url: Option<String>,
    pub terminal_class: Option<IssueExecutionTerminalClass>,
    pub recovery: Option<IssueExecutionRecovery>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueExecutionRecovery {
    pub reason: IssueExecutionRecoveryReason,
    pub summary: String,
    pub next_summary: String,
    pub requires_human: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueExecutionRecoveryReason {
    AttemptsExhausted,
    ReworkNeeded,
    ObserveAgain,
    ContractRecovery,
    IdentityConflict,
    PullRequestClosed,
    QueueRecovery,
}

impl IssueExecutionDetail {
    pub fn from_record(
        issue: &IssueReference,
        work: Option<&IssueWorkSummary>,
        record: &IssueExecutionRecord,
        execution_attempt_limit: Option<u32>,
    ) -> Self {
        if record.work_order_id != issue_work_order_id(issue)
            || work.is_none_or(|work| work.work_order_id != record.work_order_id)
        {
            return Self::Unavailable {
                reason: IssueExecutionUnavailableReason::WorkIdMismatch,
            };
        }
        if record.validate().is_err() {
            return Self::Unavailable {
                reason: IssueExecutionUnavailableReason::ReadFailed,
            };
        }
        let progress = &record.progress;
        Self::Ready(Box::new(IssueExecutionReadyDetail {
            work_order_id: record.work_order_id.clone(),
            revision: progress.revision,
            state: progress.state,
            state_label: state_label(progress.state).into(),
            execution_attempts: progress.attempts.execution,
            execution_attempt_limit,
            verification_attempts: progress.attempts.verification,
            review_pull_request_url: progress.review_pull_request_url.clone(),
            terminal_class: progress
                .terminal
                .as_ref()
                .map(|terminal| terminal.classification),
            recovery: recovery(progress),
        }))
    }
}

fn state_label(state: IssueExecutionState) -> &'static str {
    match state {
        IssueExecutionState::Queued => "실행 대기",
        IssueExecutionState::Running => "실행 중",
        IssueExecutionState::InReview => "리뷰 중",
        IssueExecutionState::Done => "통합 완료",
        IssueExecutionState::Failed => "실행 실패",
    }
}

fn recovery(progress: &IssueExecutionProgress) -> Option<IssueExecutionRecovery> {
    let terminal = progress
        .terminal
        .as_ref()
        .map(|terminal| terminal_recovery(terminal.classification));
    if progress.attempts_exhausted {
        let exhausted = "허용된 실행 횟수를 모두 사용했습니다.";
        let handoff = "작업 결과를 확인하고 직접 맡기 또는 취소를 선택해 주세요.";
        return Some(IssueExecutionRecovery {
            reason: IssueExecutionRecoveryReason::AttemptsExhausted,
            summary: terminal
                .as_ref()
                .map(|terminal| format!("{} {exhausted}", terminal.summary))
                .unwrap_or_else(|| exhausted.into()),
            next_summary: terminal
                .as_ref()
                .map(|terminal| format!("{} {handoff}", terminal.next_summary))
                .unwrap_or_else(|| handoff.into()),
            requires_human: true,
        });
    }
    terminal
}

fn terminal_recovery(terminal: IssueExecutionTerminalClass) -> IssueExecutionRecovery {
    let (reason, summary) = match terminal {
        IssueExecutionTerminalClass::ReworkNeeded => (
            IssueExecutionRecoveryReason::ReworkNeeded,
            "PR 검증에서 보완할 내용이 확인되었습니다.",
        ),
        IssueExecutionTerminalClass::ObserveAgain => (
            IssueExecutionRecoveryReason::ObserveAgain,
            "검증 결과를 다시 확인해야 합니다.",
        ),
        IssueExecutionTerminalClass::ContractRecovery => (
            IssueExecutionRecoveryReason::ContractRecovery,
            "통합 증거 또는 필수 검증 설정에 복구가 필요합니다.",
        ),
        IssueExecutionTerminalClass::IdentityConflict => (
            IssueExecutionRecoveryReason::IdentityConflict,
            "검증 대상의 식별 정보가 일치하지 않습니다.",
        ),
        IssueExecutionTerminalClass::PullRequestClosed => (
            IssueExecutionRecoveryReason::PullRequestClosed,
            "PR이 통합되지 않은 채 닫혔습니다.",
        ),
        IssueExecutionTerminalClass::QueueRecovery => (
            IssueExecutionRecoveryReason::QueueRecovery,
            "보완 작업의 실행 큐를 복구해야 합니다.",
        ),
    };
    IssueExecutionRecovery {
        reason,
        summary: summary.into(),
        next_summary: terminal.recovery_summary().into(),
        requires_human: terminal.requires_human_recovery(),
    }
}

#[cfg(test)]
mod tests;
