//! 저장된 이슈 사건의 최근 표시용 투영이다. 조회는 실행이나 원격 재확인을 시작하지 않는다.
use std::collections::BTreeMap;

use super::decision::IssueDecisionState;
use super::execution::{IssueExecutionProgress, IssueExecutionState, IssueExecutionTerminalClass};
use crate::domain::issue_work::IssueReference;
use crate::domain::kernel_event::{
    IssueClosureRecordOrigin, IssueDecisionGateRecord, IssueDecisionSubjectRecord,
    IssueEventReference, IssueFeedbackRecordAction, KernelEvent, RecordedKernelEvent,
};

pub const ISSUE_TIMELINE_LIMIT: usize = 20;
pub const ISSUE_TIMELINE_READ_LIMIT: usize = ISSUE_TIMELINE_LIMIT + 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTimelineProjection {
    Ready {
        entries: Vec<IssueTimelineEntry>,
        /// 현재 보존된 기록 중 표시 범위보다 앞선 사건이 더 있다.
        has_older: bool,
    },
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTimelineEntry {
    pub sequence: i64,
    pub recorded_at: String,
    pub kind: String,
    pub summary: String,
}

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

/// 저장이 아직 끝나지 않은 실제 runtime 기록도 합친다. 시각이 역전되어도 sequence 순서를 지킨다.
pub fn project_issue_timeline<'a>(
    issue: &IssueReference,
    stored: &'a [RecordedKernelEvent],
    recent: impl IntoIterator<Item = &'a RecordedKernelEvent>,
) -> IssueTimelineProjection {
    let mut records = BTreeMap::<i64, &RecordedKernelEvent>::new();
    for record in stored.iter().chain(recent) {
        if let Some(previous) = records.insert(record.sequence, record)
            && previous != record
        {
            return IssueTimelineProjection::Unavailable;
        }
    }
    let mut matching = records
        .into_values()
        .rev()
        .filter(|record| issue_event_matches(&record.event, issue));
    let mut entries = matching
        .by_ref()
        .take(ISSUE_TIMELINE_LIMIT)
        .map(|record| IssueTimelineEntry {
            sequence: record.sequence,
            recorded_at: record.recorded_at.clone(),
            kind: record.event.kind().to_string(),
            summary: event_summary(&record.event),
        })
        .collect::<Vec<_>>();
    let has_older = matching.next().is_some();
    entries.reverse();
    IssueTimelineProjection::Ready { entries, has_older }
}

fn event_summary(event: &KernelEvent) -> String {
    let summary = match event {
        KernelEvent::IssueTracked { .. } => "이슈 추적을 시작했습니다.",
        KernelEvent::IssueTriaged { .. } => "이슈 정책 판정을 기록했습니다.",
        KernelEvent::IssueDecisionRequested { decision, .. } => match decision.gate {
            IssueDecisionGateRecord::Start => "작업 시작 결정을 요청했습니다.",
            IssueDecisionGateRecord::Spec => "요구사항 결정을 요청했습니다.",
            IssueDecisionGateRecord::Plan => "작업 계획 결정을 요청했습니다.",
            IssueDecisionGateRecord::Delivery => "변경 제출 결정을 요청했습니다.",
            IssueDecisionGateRecord::Merge => "변경 통합 결정을 요청했습니다.",
            IssueDecisionGateRecord::Close => "이슈 닫기 결정을 요청했습니다.",
        },
        KernelEvent::IssueDecisionResolved { decision, .. } => match decision.state {
            IssueDecisionState::Answered => "결정 답변을 기록했습니다.",
            IssueDecisionState::Expired => "결정 요청의 기한이 지났습니다.",
            IssueDecisionState::Withdrawn => "결정 요청을 철회했습니다.",
            IssueDecisionState::Superseded => "기준 변경으로 이전 결정 요청이 종료되었습니다.",
            IssueDecisionState::Open => "결정 요청의 상태를 기록했습니다.",
        },
        KernelEvent::IssueDecisionReminded { .. } => "결정 요청의 응답 기한이 다가옵니다.",
        KernelEvent::IssueDecisionArbiterFinished { outcome, .. } => match outcome {
            super::arbiter::IssueArbiterFinishedOutcome::Applied {} => {
                "중재자(arbiter)의 결정 답변을 기록했습니다."
            }
            super::arbiter::IssueArbiterFinishedOutcome::Waiting { .. } => {
                "중재 검토를 마치고 사람의 답변을 기다립니다."
            }
            super::arbiter::IssueArbiterFinishedOutcome::Superseded {} => {
                "기준 변경으로 이전 중재 요청이 종료되었습니다."
            }
            super::arbiter::IssueArbiterFinishedOutcome::Discarded {} => {
                "먼저 확정된 결정을 보존하고 중재 결과를 폐기했습니다."
            }
        },
        KernelEvent::IssueWorkSubmitted { .. } => "이슈를 구현 작업에 연결했습니다.",
        KernelEvent::IssueHeld { .. } => "이슈 작업을 보류했습니다.",
        KernelEvent::IssueReleased { .. } => "이슈 작업의 보류를 해제했습니다.",
        KernelEvent::IssueWorkPermitRevoked { .. } => "기존 작업 허가를 철회했습니다.",
        KernelEvent::IssueWorkPermitGranted { delivery: true, .. } => {
            "변경 제출 허가를 기록했습니다."
        }
        KernelEvent::IssueWorkPermitGranted {
            delivery: false, ..
        } => "구현 실행 허가를 기록했습니다.",
        KernelEvent::IssueExecutionProgressed { progress, .. } => {
            return execution_summary(progress);
        }
        KernelEvent::IssueIntegrated { .. } => "변경 통합을 확인했습니다.",
        KernelEvent::IssueClosed { origin, .. } => match origin {
            IssueClosureRecordOrigin::ConfirmedFeedback => "승인된 이슈 닫기 전송을 확인했습니다.",
            IssueClosureRecordOrigin::ProviderObservation => {
                "외부에서 이슈가 닫힌 상태를 관찰했습니다."
            }
        },
        KernelEvent::IssueFeedbackPosted { action, .. } => match action {
            IssueFeedbackRecordAction::CommentCreated => "상태 댓글 게시를 확인했습니다.",
            IssueFeedbackRecordAction::CommentUpdated => "상태 댓글 갱신을 확인했습니다.",
            IssueFeedbackRecordAction::LabelAdded => "상태 라벨 추가를 확인했습니다.",
            IssueFeedbackRecordAction::LabelRemoved => "상태 라벨 제거를 확인했습니다.",
            IssueFeedbackRecordAction::BoardFieldSet => "보드 필드 변경을 확인했습니다.",
        },
        // 좌표가 있는 지원 사건만 project_issue_timeline의 필터를 통과한다.
        _ => "이슈 기록을 확인했습니다.",
    };
    summary.to_string()
}

fn execution_summary(progress: &IssueExecutionProgress) -> String {
    let status = if progress.attempts_exhausted {
        "실행 시도 상한에 도달했습니다."
    } else if let Some(terminal) = &progress.terminal {
        match terminal.classification {
            IssueExecutionTerminalClass::ReworkNeeded => {
                "검증에서 보완 작업이 필요함을 확인했습니다."
            }
            IssueExecutionTerminalClass::ObserveAgain => "검증 결과를 다시 확인해야 합니다.",
            IssueExecutionTerminalClass::ContractRecovery => {
                "통합 증거나 검증 설정을 확인해야 합니다."
            }
            IssueExecutionTerminalClass::IdentityConflict => {
                "검증 대상의 식별 정보를 확인해야 합니다."
            }
            IssueExecutionTerminalClass::PullRequestClosed => "연결된 PR이 닫혔음을 확인했습니다.",
            IssueExecutionTerminalClass::QueueRecovery => "기존 보완 작업의 큐를 확인해야 합니다.",
        }
    } else {
        match progress.state {
            IssueExecutionState::Queued => "구현 작업이 대기 중입니다.",
            IssueExecutionState::Running => "구현 작업이 실행 중입니다.",
            IssueExecutionState::InReview => "PR 검증을 진행 중입니다.",
            IssueExecutionState::Done => "구현 작업의 통합 완료를 기록했습니다.",
            IssueExecutionState::Failed => "구현 작업의 중단을 기록했습니다.",
        }
    };
    format!(
        "{status} 실행 {}회 · 검증 {}회",
        progress.attempts.execution, progress.attempts.verification
    )
}

#[cfg(test)]
mod tests;
