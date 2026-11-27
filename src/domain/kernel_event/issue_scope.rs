//! Kernel 사건의 이슈 좌표를 비교한다. 저장 adapter와 표시 투영이 같은 범위 검사를 사용한다.
use super::{IssueDecisionSubjectRecord, IssueEventReference, KernelEvent};
use crate::domain::issue_work::IssueReference;

/// 번호나 thread를 추정하지 않고 모든 안정 좌표를 비교한다.
pub fn issue_event_matches(event: &KernelEvent, issue: &IssueReference) -> bool {
    issue_event_reference(event).is_some_and(|reference| {
        reference.provider == issue.provider.into()
            && reference.server_id == issue.server_id
            && reference.repository_id == issue.repository_id
            && reference.issue_id == issue.issue_id
            && reference.project_scope.workspace_dir == issue.project_scope.workspace_dir
    })
}

fn issue_event_reference(event: &KernelEvent) -> Option<&IssueEventReference> {
    match event {
        KernelEvent::IssueTracked { issue, .. }
        | KernelEvent::IssueTriaged { issue, .. }
        | KernelEvent::IssueWorkSubmitted { issue, .. }
        | KernelEvent::IssueHeld { issue, .. }
        | KernelEvent::IssueReleased { issue, .. }
        | KernelEvent::IssueWorkPermitRevoked { issue, .. }
        | KernelEvent::IssueWorkPermitGranted { issue, .. }
        | KernelEvent::IssueExecutionProgressed { issue, .. }
        | KernelEvent::IssueIntegrated { issue, .. }
        | KernelEvent::IssueClosed { issue, .. }
        | KernelEvent::IssueFeedbackPosted { issue, .. } => Some(issue),
        KernelEvent::IssueDecisionRequested { decision, .. }
        | KernelEvent::IssueDecisionReminded { decision, .. }
        | KernelEvent::IssueDecisionArbiterFinished { decision, .. }
        | KernelEvent::IssueDecisionResolved { decision, .. } => match &decision.subject {
            IssueDecisionSubjectRecord::Issue { issue } => Some(issue),
        },
        _ => None,
    }
}
