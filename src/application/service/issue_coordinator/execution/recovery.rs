use super::*;
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkRecoveryOutcome, IssueWorkRecoveryRequest, issue_work_recovery_command_id,
};
use crate::domain::issue_automation::execution::{
    IssueExecutionTerminalClass, IssueExecutionTerminalRecord,
};
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionRequest,
};
use sha2::{Digest, Sha256};

pub(super) fn reconcile(
    service: &IssueCoordinatorService,
    work: &IssueWorkCheckpoint,
    record: &IssueExecutionRecord,
    cancel: &IssueCollectionCancellation,
    execution_enabled: bool,
    committed: &Reporter<'_>,
) -> Result<()> {
    let Some(terminal) = record.progress.terminal.as_ref() else {
        return Ok(());
    };
    if !current_record_matches(service, work, record)? {
        return Ok(());
    }
    let control = service
        .execution
        .control(&service.workspace, &work.order.work_order_id)?;
    if control.cancelled || !control.tracked || control.taken_over {
        return Ok(());
    }
    if record.progress.requires_human_recovery() {
        // 실행 추적은 이 분기 전에 한도를 닫고 기존 허가를 회수한다.
        if terminal.classification == IssueExecutionTerminalClass::IdentityConflict && !control.held
        {
            let id = format!(
                "issue-identity-hold-{:x}",
                Sha256::digest(serde_json::to_vec(&(
                    &work.order.work_order_id,
                    &terminal.record_id,
                    control.authorization_epoch,
                ))?),
            );
            intervene(service, work, id, IssueInterventionAction::Hold, cancel)?;
        }
        return ensure_recovery_decision(service, work, record, terminal, cancel, committed);
    }
    if terminal.classification == IssueExecutionTerminalClass::ReworkNeeded {
        // 기존 PR 검증 FSM이 원자 admission으로 보완 task를 계보에 붙인다.
        // 이 조정기는 별도의 구현 task를 만들지 않는다.
        return Ok(());
    }
    if !execution_enabled || !control.allows_automation() || cancel.is_cancelled() {
        return Ok(());
    }
    if record.progress.integration.is_some()
        && terminal.classification == IssueExecutionTerminalClass::QueueRecovery
    {
        return Ok(());
    }
    let (item, assessment) = current_assessment(service, &work.order.issue, committed)?;
    let basis = service.approval_basis(&assessment)?;
    let decision = match work.current_authorization_decision_id() {
        Some(id) => service.revoke_on_current_error(
            Some(work),
            service.store.get_issue_decision(&service.workspace, id),
        )?,
        None => None,
    };
    let authorization = match work.current_start_authority() {
        IssueWorkStartAuthority::Policy => Some(StartAuthorization::Policy(
            work.current_authorization_basis(),
        )),
        IssueWorkStartAuthority::Human => decision
            .as_ref()
            .map(|stored| StartAuthorization::Human(&stored.request)),
    };
    if work.current_authorization_basis() != &basis
        || !authorization.is_some_and(|authorization| {
            matches_approval(&item, &assessment, authorization, &basis)
        })
        || basis.authorization_epoch != control.authorization_epoch
        || service
            .store
            .pending_issue_interventions(&service.workspace)?
            .iter()
            .any(|intent| intent.request.issue == work.order.issue)
    {
        service
            .execution
            .revoke(&service.workspace, &work.order.work_order_id)?;
        return Ok(());
    }
    let request = IssueWorkRecoveryRequest {
        command_id: issue_work_recovery_command_id(&work.order.work_order_id, terminal),
        work_order_id: work.order.work_order_id.clone(),
        terminal: terminal.clone(),
        authorization_epoch: control.authorization_epoch,
        gate_revision: work.permit_revision,
        policy_version: basis.policy_version,
    };
    cancel.commit_if_active(|| {
        if !current_record_matches(service, work, record)? {
            return Ok(());
        }
        let receipt = service
            .execution
            .recover_execution(&service.workspace, &request)?;
        if receipt.command_id != request.command_id
            || receipt.terminal_record_id != terminal.record_id
        {
            bail!("the validation recovery receipt belongs to another terminal record");
        }
        match receipt.outcome {
            IssueWorkRecoveryOutcome::Registered {
                validation_record_key,
            } => {
                if validation_record_key.trim().is_empty() {
                    bail!("the validation recovery has no successor identity");
                }
            }
            IssueWorkRecoveryOutcome::Blocked { .. } => {}
        }
        Ok(())
    })?;
    Ok(())
}

fn ensure_recovery_decision(
    service: &IssueCoordinatorService,
    work: &IssueWorkCheckpoint,
    record: &IssueExecutionRecord,
    terminal: &IssueExecutionTerminalRecord,
    cancel: &IssueCollectionCancellation,
    committed: &Reporter<'_>,
) -> Result<()> {
    let (_, assessment) = current_assessment(service, &work.order.issue, committed)?;
    let mut basis = IssueDecisionBasis::from_assessment(
        &assessment,
        IssueAutomationGate::Spec,
        Some(work.order.work_order_id.clone()),
    )?;
    basis.authorization_epoch = service
        .store
        .issue_authorization_epoch(&service.workspace, &work.order.issue)?;
    let id = IssueDecisionId::new(format!(
        "issue-recovery-{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &work.order.work_order_id,
            terminal,
            record.progress.attempts_exhausted,
            &basis,
        ))?),
    ))?;
    let text = |value: &str| IssueDecisionSharedText::new_shared(value);
    let option = |id: &str, label: &str, effect: &str| -> Result<IssueDecisionOption> {
        Ok(IssueDecisionOption {
            id: id.to_string(),
            label: text(label)?,
            effect: text(effect)?,
        })
    };
    let (question, mut options) = if record.progress.attempts_exhausted {
        (
            "구현 시도 한도에 도달했습니다. 작업을 어떻게 정리할까요?",
            vec![option(
                "inspect",
                "실패와 예산 확인",
                "현재 실패와 실행 정책을 확인할 방향을 기록합니다. 자동으로 새 구현을 시작하지 않습니다.",
            )?],
        )
    } else if terminal.classification == IssueExecutionTerminalClass::PullRequestClosed {
        (
            "통합되지 않고 닫힌 PR을 어떻게 복구할까요?",
            vec![
                option(
                    "reopen",
                    "기존 PR 다시 열기",
                    "기존 PR을 다시 열기로 결정한 방향을 기록합니다. 해당 PR을 복구하면 기존 검증이 상태를 다시 확인합니다.",
                )?,
                option(
                    "new_pull_request",
                    "새 PR 연결",
                    "검증 대상에 맞는 새 PR을 준비할 방향을 기록합니다. 이 선택만으로 PR이나 구현 작업을 만들지 않습니다.",
                )?,
            ],
        )
    } else if terminal.classification == IssueExecutionTerminalClass::IdentityConflict {
        (
            "작업과 검증 대상의 식별 정보가 달라 보류했습니다. 어떻게 처리할까요?",
            vec![option(
                "inspect",
                "대상과 소유권 확인",
                "PR과 작업의 식별 정보를 확인합니다. 보류를 해제하기 전까지 자동 실행과 전달은 차단됩니다.",
            )?],
        )
    } else {
        (
            "통합 증거 또는 필수 검증 설정을 확인해야 합니다. 어떻게 처리할까요?",
            vec![option(
                "inspect",
                "검증 설정과 증거 확인",
                "기존 검증의 복구 안내에 따라 설정과 증거를 확인합니다. 이미 통합한 구현은 다시 실행하지 않습니다.",
            )?],
        )
    };
    options.push(option(
        "wait",
        "대기",
        "현재 결과를 유지하며 사람이 복구할 때까지 기다립니다.",
    )?);
    options.push(option(
        "cancel",
        "작업 취소",
        "같은 작업 지시의 최초 작업과 모든 보완 작업의 자동 실행 및 전달을 취소합니다.",
    )?);
    let definition = IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(work.order.issue.clone()),
        gate: IssueAutomationGate::Spec,
        kind: IssueDecisionKind::FailureRecovery,
        question: text(question)?,
        options,
        recommended: None,
        basis,
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "issue-coordinator".into(),
        evidence: vec![format!("terminal:{}", terminal.record_id)],
    };
    let commit = cancel.commit_if_active(|| service.decisions.ensure_request(id, definition))?;
    let Some(commit) = commit else {
        return Ok(());
    };
    if commit.changed {
        committed(vec![IssueCoordinatorFact::Decision {
            fact: commit.record.request.fact(),
            revision: commit.catalog_revision,
        }]);
    }
    if commit.record.request.state() == IssueDecisionState::Answered
        && commit.record.request.answer().is_some_and(|answer| {
            answer.by == IssueDecisionAnswerBy::Human && answer.option == "cancel"
        })
        && current_record_matches(service, work, record)?
    {
        let id = format!(
            "issue-recovery-cancel-{:x}",
            Sha256::digest(commit.record.request.id().as_str().as_bytes()),
        );
        intervene(
            service,
            work,
            id,
            IssueInterventionAction::Cancel { confirm: true },
            cancel,
        )?;
    }
    Ok(())
}

fn intervene(
    service: &IssueCoordinatorService,
    work: &IssueWorkCheckpoint,
    command_id: String,
    action: IssueInterventionAction,
    cancel: &IssueCollectionCancellation,
) -> Result<()> {
    let request = IssueInterventionRequest {
        command_id,
        issue: work.order.issue.clone(),
        action,
    };
    if cancel
        .commit_if_active(|| {
            service
                .store
                .begin_issue_intervention(&service.workspace, &request)
        })?
        .is_some()
    {
        super::super::super::issue_intervention::recover_pending_controls(
            &service.workspace,
            service.store.as_ref(),
            service.execution.as_ref(),
        )?;
    }
    Ok(())
}
