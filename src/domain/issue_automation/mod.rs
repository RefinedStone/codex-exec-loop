//! 이슈 자동화 장부의 값과 전이 규칙이다 (#2536, plan 28 IA1-1).
//! 실행 허가, 수집, 판정 알고리즘은 여기서 만들지 않는다.
//! 원본 필드의 해시와 정책을 지난 공유본만 보관한다.
mod binding;
// Observe 수집 설정과 현재 관찰 투영이다 (#2538).
pub mod autonomy;
pub mod collection;
pub mod coordinator;
pub mod interaction;
// 판에 묶인 결정 요청과 답이다 (#2544).
pub mod decision;
// 수집/실행과 독립된 자동화 정책의 typed 효과값이다 (#2540).
pub mod policy;
// 현재 관찰과 정책의 결정적 판정이다. 실행 허가는 만들지 않는다 (#2541).
mod state;
mod tracked;
pub mod triage;
// 출처의 공유 관찰 값과 페이지 커서다 (#2537).
pub mod source;

pub use binding::{
    IssueAutomationAutonomy, IssueAutomationBinding, IssueAutomationBindingId,
    PersistedIssueAutomationBinding,
};
pub use state::{
    IssueAutomationFlag, IssueAutomationFlags, IssueAutomationState, IssueDecisionResumePhase,
    IssueTransition,
};
pub use tracked::{IssueAutomationSnapshot, PersistedTrackedIssue, TrackedIssue};

/// 판정 알고리즘의 구현이 아니라, 그 결과로 기록할 준비 상태만 표현한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueTriageResult {
    Ineligible,
    NeedsInfo,
    Ready,
}

/// SQLite INTEGER에 정확히 저장할 수 있는 장부 판의 상한이다.
pub const MAX_ISSUE_AUTOMATION_REVISION: u64 = i64::MAX as u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAutomationError {
    InvalidValue {
        field: &'static str,
    },
    InvalidRevision,
    InvalidDecisionResume,
    IdentityChanged,
    BindingChanged,
    RevisionChanged,
    InvalidTransition {
        from: IssueAutomationState,
        transition: IssueTransition,
    },
    InvalidStateReplacement {
        from: IssueAutomationState,
        to: IssueAutomationState,
    },
}

impl std::fmt::Display for IssueAutomationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidValue { field } => {
                write!(formatter, "invalid issue automation field: {field}")
            }
            Self::InvalidRevision => {
                formatter.write_str("invalid persisted issue automation revision")
            }
            Self::InvalidDecisionResume => {
                formatter.write_str("invalid issue decision resume phase")
            }
            Self::IdentityChanged => {
                formatter.write_str("the stable issue reference cannot change")
            }
            Self::BindingChanged => {
                formatter.write_str("the issue automation binding cannot change")
            }
            Self::RevisionChanged => {
                formatter.write_str("the store owns the issue automation revision")
            }
            Self::InvalidTransition { from, transition } => {
                write!(
                    formatter,
                    "issue transition {transition:?} is not allowed from {from:?}"
                )
            }
            Self::InvalidStateReplacement { from, to } => {
                write!(
                    formatter,
                    "issue state replacement from {from:?} to {to:?} is not allowed"
                )
            }
        }
    }
}

impl std::error::Error for IssueAutomationError {}

fn validate_text(value: &str, field: &'static str) -> Result<(), IssueAutomationError> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(IssueAutomationError::InvalidValue { field });
    }
    Ok(())
}

fn validate_reference(
    reference: &crate::domain::issue_work::IssueReference,
) -> Result<(), IssueAutomationError> {
    validate_text(&reference.server_id, "reference.server_id")?;
    validate_text(&reference.repository_id, "reference.repository_id")?;
    validate_text(&reference.issue_id, "reference.issue_id")?;
    validate_text(
        &reference.project_scope.workspace_dir,
        "reference.project_scope",
    )
}

#[cfg(test)]
mod tests;

pub mod intervention;
pub mod work_control;
