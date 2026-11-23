//! 통합 사실에 묶인 별도 사람 승인이다. 시작 승인은 이슈 닫기 권한을 만들지 않는다.
use sha2::{Digest, Sha256};

use super::coordinator::IssueWorkCheckpoint;
use super::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionError,
    IssueDecisionId, IssueDecisionKind, IssueDecisionRequest, IssueDecisionState,
    IssueDecisionTimeout,
};
use super::execution::{IssueExecutionRecord, IssueExecutionState};
use super::policy::IssueAutomationGate;

/// 현재 통합 결과의 식별자와 두 SHA 모두를 승인 근거에 고정한다.
pub fn close_decision_evidence(execution: &IssueExecutionRecord) -> Option<Vec<String>> {
    execution.validate().ok()?;
    if execution.progress.state != IssueExecutionState::Done
        || execution.progress.requires_human_recovery()
    {
        return None;
    }
    let integration = execution.progress.integration.as_ref()?;
    Some(vec![
        format!("integration:{}", integration.record_id),
        format!("source:{}", integration.source_sha),
        format!("integrated:{}", integration.integration_sha),
    ])
}

/// 정책·공유본·인가 epoch·관문·통합 결과가 바뀌면 이전 결정 ID를 재사용하지 않는다.
pub fn close_decision_id(
    work: &IssueWorkCheckpoint,
    execution: &IssueExecutionRecord,
    basis: &IssueDecisionBasis,
) -> Result<IssueDecisionId, IssueDecisionError> {
    work.validate()
        .map_err(|_| IssueDecisionError::InvalidBasis)?;
    basis.validate()?;
    if execution.work_order_id != work.order.work_order_id
        || basis.work_order_id.as_deref() != Some(work.order.work_order_id.as_str())
        || basis.field_hash.is_none()
    {
        return Err(IssueDecisionError::InvalidBasis);
    }
    let evidence = close_decision_evidence(execution).ok_or(IssueDecisionError::InvalidBasis)?;
    let identity = serde_json::to_vec(&(
        "issue-close-approval-v1",
        &work.order.issue,
        &work.order.work_order_id,
        evidence,
        basis,
    ))
    .map_err(|_| IssueDecisionError::InvalidBasis)?;
    IssueDecisionId::new(format!("issue-close-{:x}", Sha256::digest(identity)))
}

/// service의 사전검사와 SQLite 결정 저장 transaction이 같은 통합 근거를 검사한다.
pub fn close_work_context_matches(
    definition: &IssueDecisionDefinition,
    work: &IssueWorkCheckpoint,
    execution: &IssueExecutionRecord,
) -> bool {
    definition.kind == IssueDecisionKind::CloseApproval
        && definition.gate == IssueAutomationGate::Close
        && definition.subject.issue() == &work.order.issue
        && definition.basis.work_order_id.as_deref() == Some(work.order.work_order_id.as_str())
        && execution.work_order_id == work.order.work_order_id
        && definition.basis.field_hash.is_some()
        && definition.on_timeout == IssueDecisionTimeout::Wait
        && definition
            .options
            .iter()
            .map(|option| option.id.as_str())
            .eq(["close", "wait"])
        && close_decision_evidence(execution)
            .is_some_and(|evidence| definition.evidence == evidence)
}

/// 현재 판에 대한 명시적 사람의 close 답만 외부 닫기의 근거로 쓴다.
pub fn close_approval_matches(
    request: &IssueDecisionRequest,
    work: &IssueWorkCheckpoint,
    execution: &IssueExecutionRecord,
    current_basis: &IssueDecisionBasis,
) -> bool {
    request.state() == IssueDecisionState::Answered
        && request.definition().basis == *current_basis
        && close_work_context_matches(request.definition(), work, execution)
        && close_decision_id(work, execution, current_basis)
            .is_ok_and(|expected| request.id() == &expected)
        && request.answer().is_some_and(|answer| {
            answer.by == IssueDecisionAnswerBy::Human
                && answer.option == "close"
                && answer.confirmation.is_some()
        })
}
