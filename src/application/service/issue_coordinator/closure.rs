//! 실행 완료와 별개인 외부 이슈 닫기 승인을 준비한다 (#2553).
use super::{IssueCoordinatorFact, IssueCoordinatorService, Reporter, execution};
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::domain::issue_automation::IssueAutomationState;
use crate::domain::issue_automation::closure::{close_decision_evidence, close_decision_id};
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::decision::{
    IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionKind, IssueDecisionOption,
    IssueDecisionSharedText, IssueDecisionSubject, IssueDecisionTimeout,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::{IssueRemoteState, IssueSourceRead};
use crate::domain::issue_work::IssueReference;
use anyhow::Result;

/// 승인 요청만 생성한다. 실행 permit·재구현·provider 쓰기를 수행하지 않는다.
pub(super) fn reconcile(
    service: &IssueCoordinatorService,
    issue: &IssueReference,
    cancel: &IssueCollectionCancellation,
    now_unix_ms: u64,
    committed: &Reporter<'_>,
) -> Result<()> {
    if cancel.is_cancelled()
        || !service
            .store
            .get_collection(&service.workspace)?
            .is_some_and(|collection| collection.config.mode == IssueCollectionMode::Observe)
    {
        return Ok(());
    }
    let Some(work) = service.store.get_issue_work(&service.workspace, issue)? else {
        return Ok(());
    };
    let Some(record) = service
        .store
        .get_issue_execution(&service.workspace, issue)?
    else {
        return Ok(());
    };
    let Some(evidence) = close_decision_evidence(&record) else {
        return Ok(());
    };
    if record.work_order_id != work.order.work_order_id {
        return Ok(());
    }
    if let Some(feedback) = &service.feedback
        && feedback
            .snapshot(issue)?
            .is_some_and(|snapshot| snapshot.state.close_proof.is_some())
    {
        // 확인된 닫기는 영구 이력이다. 사람이 다시 연 이슈에 새 닫기 카드를 만들지 않는다.
        return Ok(());
    }
    let (item, assessment) = execution::current_assessment(service, issue, committed)?;
    let Some(tracked) = &item.tracked else {
        return Ok(());
    };
    if tracked.state != IssueAutomationState::Done
        || tracked.flags.held
        || tracked.flags.stale
        || tracked.flags.unknown
        || !matches!(&item.current, Some(IssueSourceRead::Observed(observation))
            if observation.remote_state == IssueRemoteState::Open)
        || !service
            .triage
            .integration_close_allowed(&assessment.basis().automation_policy_version)?
        || service
            .store
            .pending_issue_interventions(&service.workspace)?
            .iter()
            .any(|pending| pending.request.issue == *issue)
    {
        return Ok(());
    }
    let epoch = service
        .store
        .issue_authorization_epoch(&service.workspace, issue)?;
    let control = service
        .execution
        .control(&service.workspace, &work.order.work_order_id)?;
    if !control.allows_automation() || control.authorization_epoch != epoch {
        return Ok(());
    }
    let mut basis = IssueDecisionBasis::from_assessment(
        &assessment,
        IssueAutomationGate::Close,
        Some(work.order.work_order_id.clone()),
    )?;
    basis.authorization_epoch = epoch;
    let id = close_decision_id(&work, &record, &basis)?;
    let text = |value: &str| IssueDecisionSharedText::new_shared(value);
    let definition = IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(issue.clone()),
        gate: IssueAutomationGate::Close,
        kind: IssueDecisionKind::CloseApproval,
        question: text("작업의 통합이 확인됐습니다. 외부 이슈도 닫을까요?")?,
        options: vec![
            IssueDecisionOption {
                id: "close".into(),
                label: text("이슈 닫기")?,
                effect: text("현재 공유본과 통합 근거를 다시 확인한 뒤 외부 이슈를 닫습니다.")?,
            },
            IssueDecisionOption {
                id: "wait".into(),
                label: text("열린 상태 유지")?,
                effect: text("외부 이슈를 열어 둡니다. 완료된 작업은 다시 실행하지 않습니다.")?,
            },
        ],
        recommended: None,
        basis,
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "issue-coordinator".into(),
        evidence,
    };
    let Some(commit) = cancel.commit_if_active(|| {
        service
            .decisions
            .ensure_request_at(id, definition, now_unix_ms)
    })?
    else {
        return Ok(());
    };
    if commit.changed {
        committed(vec![IssueCoordinatorFact::Decision {
            fact: commit.record.request.fact(),
            revision: commit.catalog_revision,
        }]);
    }
    Ok(())
}
