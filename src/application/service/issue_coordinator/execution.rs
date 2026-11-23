//! 저장된 작업의 로컬 실행 사실을 수집 주기와 독립적으로 반영한다.
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueExecutionSaveResult;
use crate::domain::issue_automation::execution::IssueExecutionRecord;

mod decisions;
mod recovery;

impl IssueCoordinatorService {
    /// 로컬 실행과 기한을 추적한다. 확정된 Default 시작만 기존 시작 경계로 전달한다.
    pub fn track_execution(
        &self,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        execution_enabled: bool,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        let _guard = self.lock_tick_gate()?;
        if cancel.is_cancelled() {
            return Ok(());
        }
        let report_once = work_event_reporter(committed);
        let committed: &Reporter<'_> = &report_once;
        let arbiter_error = self
            .process_arbiters(cancel, now_unix_ms, execution_enabled, committed)
            .err();
        let deadline_error = self
            .process_decision_deadlines(cancel, now_unix_ms, committed)
            .err();
        let feedback_error = self.reconcile_feedback(cancel).err();
        self.report_pending(committed)?;
        super::super::issue_intervention::recover_pending_controls(
            &self.workspace,
            self.store.as_ref(),
            self.execution.as_ref(),
        )?;
        let result = track(
            self,
            cancel,
            now_unix_ms,
            execution_enabled,
            committed,
            &mut std::collections::BTreeSet::new(),
        );
        result.and_then(|()| {
            feedback_error
                .or(deadline_error)
                .or(arbiter_error)
                .map_or(Ok(()), Err)
        })
    }
}

pub(super) fn track(
    service: &IssueCoordinatorService,
    cancel: &IssueCollectionCancellation,
    now_unix_ms: u64,
    execution_enabled: bool,
    committed: &Reporter<'_>,
    failed_work_orders: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    let budget = service.triage.execution_attempt_limit();
    let mut first_error = None;
    let mut after = None;
    'pages: loop {
        if cancel.is_cancelled() {
            break;
        }
        let batch = service
            .store
            .read_triage_batch(&service.workspace, after.as_ref(), 100)?;
        for item in batch.items {
            if cancel.is_cancelled() {
                break 'pages;
            }
            let result = (|| {
                let Some(work) = service
                    .store
                    .get_issue_work(&service.workspace, &item.issue)?
                else {
                    return Ok(());
                };
                track_one(
                    service,
                    TrackedWork {
                        checkpoint: &work,
                        binding_run_blocked: item.binding_run_blocked,
                    },
                    budget.as_ref().ok().copied(),
                    cancel,
                    now_unix_ms,
                    execution_enabled,
                    committed,
                )
            })()
            .and_then(|()| {
                if item.binding_run_blocked {
                    return Ok(());
                }
                super::closure::reconcile(service, &item.issue, cancel, now_unix_ms, committed)
            })
            .and_then(|()| {
                if item.binding_run_blocked {
                    return Ok(());
                }
                super::delivery::reconcile_pending(
                    service,
                    &item,
                    cancel,
                    now_unix_ms,
                    execution_enabled,
                    committed,
                )
            })
            .and_then(|()| {
                if item.binding_run_blocked {
                    return Ok(());
                }
                reconcile_timeout_start(
                    service,
                    &item,
                    cancel,
                    now_unix_ms,
                    execution_enabled,
                    committed,
                )
            });
            if let Err(error) = result {
                failed_work_orders.insert(issue_work_order_id(&item.issue));
                first_error.get_or_insert(error);
            }
            if !item.binding_run_blocked
                && let Err(error) =
                    service.sync_issue_feedback(&item.issue, now_unix_ms, cancel, committed)
            {
                first_error.get_or_insert(error);
            }
            // 앞 작업의 조회/회수 실패가 다음 작업의 사실 반영과 제어를 막지 않는다.
            if let Err(error) = service.report_pending(committed) {
                first_error.get_or_insert(error);
            }
        }
        after = batch.next_after;
        if after.is_none() {
            break;
        }
    }
    first_error.map_or(Ok(()), Err)
}

// A crash between Default commit and submission must not require another remote collection.
// The normal coordinator still performs resync, budget admission and permit checks.
fn reconcile_timeout_start(
    service: &IssueCoordinatorService,
    item: &IssueCollectionItem,
    cancel: &IssueCollectionCancellation,
    now_unix_ms: u64,
    execution_enabled: bool,
    committed: &Reporter<'_>,
) -> Result<()> {
    if item.binding_run_blocked || cancel.is_cancelled() {
        return Ok(());
    }
    let observing = service
        .store
        .get_collection(&service.workspace)?
        .is_some_and(|collection| collection.config.mode == IssueCollectionMode::Observe);
    if let Some(work) = service
        .store
        .get_issue_work(&service.workspace, &item.issue)?
        && matches!(
            work.current_start_authority(),
            IssueWorkStartAuthority::TimeoutDefault | IssueWorkStartAuthority::TimeoutArbiter
        )
    {
        if !observing {
            return fence_execution(service, &work);
        }
        return service.reconcile(
            item.clone(),
            cancel,
            now_unix_ms,
            execution_enabled,
            committed,
        );
    }
    if !execution_enabled || !observing {
        return Ok(());
    }
    let Some(triage) = &item.triage else {
        return Ok(());
    };
    if !triage.assessment.gates().iter().any(|gate| {
        gate.gate == IssueAutomationGate::Start
            && matches!(gate.action, crate::domain::issue_automation::policy::IssueAutomationGateAction::TimeoutDefault | crate::domain::issue_automation::policy::IssueAutomationGateAction::TimeoutArbiter)
    }) { return Ok(()); }
    let basis = service.approval_basis(&triage.assessment)?;
    let id = start_decision_id(&item.issue, &basis);
    let Some(decision) = service.store.get_issue_decision(&service.workspace, &id)? else {
        return Ok(());
    };
    if timeout_default_start(&triage.assessment, &decision.request)
        || timeout_arbiter_start(&triage.assessment, &decision.request)
    {
        service.reconcile(
            item.clone(),
            cancel,
            now_unix_ms,
            execution_enabled,
            committed,
        )?;
    }
    Ok(())
}

struct TrackedWork<'a> {
    checkpoint: &'a IssueWorkCheckpoint,
    binding_run_blocked: bool,
}

fn track_one(
    service: &IssueCoordinatorService,
    tracked: TrackedWork<'_>,
    max_attempts: Option<u32>,
    cancel: &IssueCollectionCancellation,
    now_unix_ms: u64,
    execution_enabled: bool,
    committed: &Reporter<'_>,
) -> Result<()> {
    let TrackedWork {
        checkpoint: work,
        binding_run_blocked,
    } = tracked;
    let mut first_error = None;
    let mut fenced = false;
    if max_attempts.is_none() {
        first_error = Some(anyhow!(
            "the current execution attempt policy is unavailable"
        ));
        if let Err(error) = fence_execution(service, work) {
            let context = format!(
                "the current execution attempt policy is unavailable; execution could not be fenced: {error}"
            );
            first_error = Some(error.context(context));
        }
        fenced = true;
    }
    if let Err(error) = decisions::poll_pending_answers(service, work, cancel) {
        first_error.get_or_insert(error);
    }
    let previous = match service
        .store
        .get_issue_execution(&service.workspace, &work.order.issue)
    {
        Ok(value) => value,
        Err(error) => {
            if !fenced {
                fence_execution(service, work)?;
            }
            return Err(first_error.unwrap_or(error));
        }
    };
    let observation = match service
        .execution
        .observe_execution(&service.workspace, &work.order.work_order_id)
    {
        Ok(value) => value,
        Err(error) => {
            if !fenced {
                fence_execution(service, work)?;
            }
            return Err(first_error.unwrap_or(error));
        }
    };
    let Some(observation) = observation else {
        return first_error.map_or(Ok(()), Err);
    };
    let already_integrated = previous
        .as_ref()
        .is_some_and(|record| record.progress.integration.is_some());
    let needs_human_recovery = previous
        .as_ref()
        .is_some_and(|record| record.progress.attempts_exhausted)
        || observation.current_terminal_id.as_ref().is_some_and(|id| {
            observation.terminals.iter().any(|terminal| {
                &terminal.record_id == id && terminal.classification.requires_human_recovery()
            })
        });
    if !fenced && needs_human_recovery {
        if let Err(error) = fence_execution(service, work) {
            first_error.get_or_insert(error);
        }
        fenced = true;
    }
    if !fenced && !binding_run_blocked {
        if let Err(error) = service.execution.execution_limit(
            &service.workspace,
            &work.order.work_order_id,
            if already_integrated || observation.integration.is_some() || needs_human_recovery {
                0
            } else {
                max_attempts.unwrap_or(0)
            },
        ) {
            first_error.get_or_insert(error);
        }
        if first_error.is_some()
            && let Err(error) = fence_execution(service, work)
        {
            first_error.get_or_insert(error);
        }
    }
    let result = match cancel.commit_if_active(|| {
        service.store.apply_issue_execution(
            &service.workspace,
            work,
            previous
                .as_ref()
                .map_or(0, |record| record.progress.revision),
            &observation,
            max_attempts,
        )
    }) {
        Ok(result) => result,
        Err(error) => {
            if !fenced {
                fence_execution(service, work)?;
            }
            return Err(first_error.unwrap_or(error));
        }
    };
    let record = match result {
        Some(IssueExecutionSaveResult::Saved { record, .. }) => record,
        Some(IssueExecutionSaveResult::Conflict) => {
            // 낡은 관측으로 새 실행을 열지 않는다. 다음 tick이 work와 실행기를 함께 새로 읽는다.
            if let Err(error) = fence_execution(service, work) {
                first_error.get_or_insert(error);
            }
            return Err(first_error.unwrap_or_else(|| {
                anyhow!("the issue execution changed concurrently; a fresh observation is required")
            }));
        }
        None => return first_error.map_or(Ok(()), Err),
    };
    // 알려진 복구 대기는 후속 알림이나 저장소 조회가 실패해도 실행을 다시 열지 않는다.
    if !fenced
        && record.progress.requires_human_recovery()
        && let Err(error) = fence_execution(service, work)
    {
        first_error.get_or_insert(error);
    }
    service.report_pending(committed)?;
    if binding_run_blocked {
        // Preserve terminal/Unknown observations and existing human answer reconciliation,
        // but do not create new decisions, recovery, or execution permissions during Pause.
        return first_error.map_or(Ok(()), Err);
    }
    let Some(current_work) = service
        .store
        .get_issue_work(&service.workspace, &work.order.issue)?
    else {
        return first_error.map_or(Ok(()), Err);
    };
    if record.progress.integration.is_some() && !already_integrated {
        // 완료 사실 뒤에는 보완 구현을 새로 시작하지 않는다. 검증 기록은 계속 읽는다.
        if let Err(error) =
            service
                .execution
                .execution_limit(&service.workspace, &work.order.work_order_id, 0)
        {
            first_error.get_or_insert(error);
        }
    }
    if first_error.is_none() && !cancel.is_cancelled() {
        if let Err(error) = decisions::reconcile(
            service,
            &current_work,
            &record,
            cancel,
            now_unix_ms,
            execution_enabled,
            committed,
        ) {
            first_error.get_or_insert(error);
        }
        if let Err(error) = recovery::reconcile(
            service,
            &current_work,
            &record,
            cancel,
            now_unix_ms,
            execution_enabled,
            committed,
        ) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// 최초 승인이나 새 source만으로 사람의 복구 대기를 우회하지 않는다.
pub(super) fn submission_blocked(
    service: &IssueCoordinatorService,
    work: &IssueWorkCheckpoint,
) -> Result<bool> {
    let record = match service
        .store
        .get_issue_execution(&service.workspace, &work.order.issue)
    {
        Ok(record) => record,
        Err(error) => {
            fence_execution(service, work)?;
            return Err(error);
        }
    };
    let blocked = record.is_some_and(|record| record.progress.requires_human_recovery());
    if blocked {
        fence_execution(service, work)?;
    }
    Ok(blocked)
}

/// 한 경계의 실패가 다른 경계의 차단 시도를 생략하지 않도록 둘 다 호출한다.
fn fence_execution(service: &IssueCoordinatorService, work: &IssueWorkCheckpoint) -> Result<()> {
    let limit = service
        .execution
        .execution_limit(&service.workspace, &work.order.work_order_id, 0);
    let revoke = if work.delivery_assessment.is_some() || work.delivery_granted {
        service.invalidate_delivery(&mut work.clone(), IssueWorkBlockReason::ApprovalChanged)
    } else {
        service
            .execution
            .revoke(&service.workspace, &work.order.work_order_id)
    };
    match (limit, revoke) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(revoke)) => {
            Err(error.context(format!("work permits could not be revoked: {revoke}")))
        }
    }
}

/// 수집 Off에서도 기존 작업의 결정에 필요한 현재 로컬 판정을 고정한다.
pub(super) fn current_assessment(
    service: &IssueCoordinatorService,
    issue: &IssueReference,
    committed: &Reporter<'_>,
) -> Result<(IssueCollectionItem, IssueTriageAssessment)> {
    for _ in 0..3 {
        let mut after = None;
        loop {
            let batch = service
                .store
                .read_triage_batch(&service.workspace, after.as_ref(), 100)?;
            let control = batch
                .collection
                .ok_or_else(|| anyhow!("the issue collection is unavailable"))?;
            if let Some(mut item) = batch.items.into_iter().find(|item| item.issue == *issue) {
                let assessment = service.triage.assess_current(&item)?;
                let fence = IssueTriageFence {
                    binding_run_revision: item.binding_run_revision,
                    collection_epoch: control.epoch,
                    observation_revision: item.observation_revision,
                    tracked_revision: item.tracked_revision,
                    assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
                };
                if let IssueTriageSaveResult::Saved {
                    record,
                    binding_id,
                    catalog_revision,
                    changed,
                } = service
                    .store
                    .save_triage(&service.workspace, fence, &assessment)?
                {
                    if changed {
                        committed(vec![IssueCoordinatorFact::Triaged(IssueTriagedFact {
                            issue: item.issue.clone(),
                            binding_id,
                            revision: catalog_revision,
                            assessment_revision: record.revision,
                            gate_revision: record
                                .assessment
                                .gates()
                                .iter()
                                .map(|gate| gate.gate_revision)
                                .max()
                                .ok_or_else(|| {
                                    anyhow!("the committed issue assessment has no gates")
                                })?,
                            policy_version: record
                                .assessment
                                .basis()
                                .automation_policy_version
                                .clone(),
                            field_hash: record.assessment.basis().field_hash.clone(),
                        })]);
                    }
                    item.triage = Some(*record);
                    return Ok((item, assessment));
                }
                break;
            }
            after = batch.next_after;
            if after.is_none() {
                bail!("the tracked issue is unavailable");
            }
        }
    }
    bail!("the local issue assessment changed concurrently")
}

fn current_record_matches(
    service: &IssueCoordinatorService,
    work: &IssueWorkCheckpoint,
    record: &IssueExecutionRecord,
) -> Result<bool> {
    Ok(service
        .store
        .get_issue_work(&service.workspace, &work.order.issue)?
        .as_ref()
        == Some(work)
        && service
            .store
            .get_issue_execution(&service.workspace, &work.order.issue)?
            .as_ref()
            == Some(record))
}
