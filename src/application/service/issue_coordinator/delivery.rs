//! 실제 diff 판정은 Start 승인과 분리하고, 정확한 commit 범위에만 전달 허가를 낸다.
use super::*;
use crate::domain::issue_automation::delivery::IssueDeliveryAssessment;
use sha2::{Digest, Sha256};

impl IssueCoordinatorService {
    pub(super) fn prepare_delivery(
        &self,
        work: &mut IssueWorkCheckpoint,
        source: &str,
        permit_floor: u64,
        now_unix_ms: u64,
        cancel: &IssueCollectionCancellation,
        committed: &Reporter<'_>,
    ) -> Result<Option<IssueDecisionBasis>> {
        // 이미 발급했다가 회수한 같은 source를 정책 읽기 회복만으로 새 판으로 승격하지 않는다.
        // 새 source 또는 명시적 제어 후의 새 authorization epoch가 기존 철회 경계를 넘는다.
        if work.delivery_assessment.is_none()
            && work.delivery_revision > 0
            && work.permit_source.as_deref() == Some(source)
            && work.permit_authorization_epoch
                == work.current_authorization_basis().authorization_epoch
        {
            return Ok(None);
        }
        let result = self.read_delivery_assessment(work, source, permit_floor, committed);
        let assessment = match result {
            Ok(assessment) => assessment,
            Err(error) => {
                self.invalidate_delivery(work, IssueWorkBlockReason::SourceUnavailable)?;
                return Err(error);
            }
        };
        if work.delivery_assessment.as_ref() != Some(&assessment) {
            self.execution
                .revoke(&self.workspace, &work.order.work_order_id)?;
            work.delivery_revision = assessment.basis.gate_revision;
            work.delivery_assessment = Some(assessment.clone());
            work.delivery_granted = false;
            work.blocked_reason = Some(if assessment.verdict.requires_human() {
                IssueWorkBlockReason::AwaitingDeliveryApproval
            } else {
                IssueWorkBlockReason::AwaitingSource
            });
            let (fence, _, current, _) = self.current_or_revoke(work, committed)?;
            let saved = cancel.commit_if_active(|| {
                self.store
                    .save_issue_work(&self.workspace, fence, &current, work)
            })?;
            let Some(IssueWorkSaveResult::Saved { record, .. }) = saved else {
                return Ok(None);
            };
            *work = *record;
        }
        if !assessment.verdict.requires_human() {
            return Ok(Some(assessment.basis));
        }
        let id = delivery_decision_id(&assessment.basis)?;
        let definition = delivery_definition(&work.order.issue, &assessment)?;
        let commit = cancel.commit_if_active(|| {
            self.decisions
                .ensure_request_at(id, definition, now_unix_ms)
        })?;
        let Some(commit) = commit else {
            return Ok(None);
        };
        if commit.changed {
            committed(vec![IssueCoordinatorFact::Decision {
                fact: commit.record.request.fact(),
                revision: commit.catalog_revision,
            }]);
        }
        Ok(human_delivery(&commit.record.request, &assessment).then_some(assessment.basis))
    }

    fn read_delivery_assessment(
        &self,
        work: &IssueWorkCheckpoint,
        source: &str,
        permit_floor: u64,
        committed: &Reporter<'_>,
    ) -> Result<IssueDeliveryAssessment> {
        let diff =
            self.execution
                .delivery_diff(&self.workspace, &work.order.work_order_id, source)?;
        diff.validate().map_err(|error| anyhow!(error))?;
        if diff.source_commit_sha != source {
            bail!("the delivery diff does not match the frozen source");
        }
        let (_, _, triage, start) = self.current_or_revoke(work, committed)?;
        if &start != work.current_authorization_basis() {
            bail!("the start approval changed before delivery assessment");
        }
        let verdict = self
            .triage
            .assess_delivery_diff_for_policy(&start.policy_version, &diff)?;
        let context = IssueDecisionDeliveryContext {
            base_commit_sha: diff.base_commit_sha,
            source_commit_sha: diff.source_commit_sha,
        };
        if let Some(existing) = &work.delivery_assessment
            && existing.matches_work(work)
            && existing.matches_triage(&triage, start.authorization_epoch)
            && existing.basis.delivery.as_ref() == Some(&context)
            && existing.verdict == verdict
            && existing.basis.gate_revision >= permit_floor
        {
            return Ok(existing.clone());
        }
        let mut basis = IssueDecisionBasis::from_assessment(
            &triage,
            IssueAutomationGate::Delivery,
            Some(work.order.work_order_id.clone()),
        )?;
        let triage_gate_revision = basis.gate_revision;
        basis.authorization_epoch = start.authorization_epoch;
        basis.gate_revision = work
            .delivery_revision
            .max(work.permit_revision)
            .max(permit_floor)
            .max(triage_gate_revision)
            .checked_add(1)
            .filter(|revision| {
                *revision <= crate::domain::issue_automation::MAX_ISSUE_AUTOMATION_REVISION
            })
            .ok_or_else(|| anyhow!("the delivery review revision is exhausted"))?;
        basis.delivery = Some(context);
        let assessment = IssueDeliveryAssessment {
            basis,
            triage_gate_revision,
            verdict,
        };
        assessment.validate().map_err(|error| anyhow!(error))?;
        Ok(assessment)
    }

    pub(super) fn delivery_review_current(
        &self,
        work: &IssueWorkCheckpoint,
        basis: &IssueDecisionBasis,
        triage: &IssueTriageAssessment,
    ) -> Result<bool> {
        let Some(assessment) = work.delivery_assessment.as_ref().filter(|assessment| {
            assessment.basis == *basis
                && assessment.matches_work(work)
                && assessment.matches_triage(
                    triage,
                    work.current_authorization_basis().authorization_epoch,
                )
        }) else {
            return Ok(false);
        };
        if self
            .store
            .get_issue_work(&self.workspace, &work.order.issue)?
            .as_ref()
            != Some(work)
        {
            return Ok(false);
        }
        if !assessment.verdict.requires_human() {
            return Ok(true);
        }
        let id = delivery_decision_id(basis)?;
        Ok(self
            .store
            .get_issue_decision(&self.workspace, &id)?
            .is_some_and(|stored| human_delivery(&stored.request, assessment)))
    }

    /// 발급 응답만 유실되었고 현재 범위·승인이 그대로면 같은 판의 멱등 재시도를 보존한다.
    /// 현재성을 읽을 수 없거나 source가 바뀐 실패는 기존 허가도 회수한다.
    pub(super) fn delivery_grant_retryable(
        &self,
        work: &IssueWorkCheckpoint,
        committed: &Reporter<'_>,
    ) -> Result<bool> {
        let Some(review) = &work.delivery_assessment else {
            return Ok(false);
        };
        let range = review
            .basis
            .delivery
            .as_ref()
            .ok_or_else(|| anyhow!("missing delivery range"))?;
        let diff = self.execution.delivery_diff(
            &self.workspace,
            &work.order.work_order_id,
            &range.source_commit_sha,
        )?;
        if diff.base_commit_sha != range.base_commit_sha
            || diff.source_commit_sha != range.source_commit_sha
        {
            return Ok(false);
        }
        let (_, _, current, start) = self.current_or_revoke(work, committed)?;
        Ok(&start == work.current_authorization_basis()
            && self.delivery_review_current(work, &review.basis, &current)?
            && self
                .triage
                .assess_delivery_diff_for_policy(&start.policy_version, &diff)?
                == review.verdict)
    }

    /// 정책 파일이나 수집 상태에 의존하지 않는 감권 전용 CAS로 판정을 폐기한다.
    /// revision floor는 남겨 같은 범위로 회복해도 과거 사람 답을 재사용하지 않는다.
    pub(super) fn invalidate_delivery(
        &self,
        work: &mut IssueWorkCheckpoint,
        reason: IssueWorkBlockReason,
    ) -> Result<()> {
        self.execution
            .revoke(&self.workspace, &work.order.work_order_id)?;
        for _ in 0..3 {
            let Some(latest) = self
                .store
                .get_issue_work(&self.workspace, &work.order.issue)?
            else {
                return Ok(());
            };
            match self.store.clear_issue_delivery(
                &self.workspace,
                &latest.order.issue,
                latest.revision,
                reason,
            )? {
                IssueWorkSaveResult::Saved { record, .. } => {
                    *work = *record;
                    return Ok(());
                }
                IssueWorkSaveResult::Conflict => continue,
            }
        }
        bail!("the issue delivery checkpoint changed while revoking its permit")
    }
}

pub(super) fn delivery_decision_id(basis: &IssueDecisionBasis) -> Result<IssueDecisionId> {
    let digest = Sha256::digest(serde_json::to_vec(basis)?);
    Ok(IssueDecisionId::new(format!(
        "issue-delivery-v1-{digest:x}"
    ))?)
}

/// 새 완료 source나 사람의 Delivery 답은 수집 주기를 기다리지 않고 같은 제출 경계로 보낸다.
pub(super) fn reconcile_pending(
    service: &IssueCoordinatorService,
    item: &IssueCollectionItem,
    cancel: &IssueCollectionCancellation,
    now_unix_ms: u64,
    execution_enabled: bool,
    committed: &Reporter<'_>,
) -> Result<()> {
    let Some(mut work) = service
        .store
        .get_issue_work(&service.workspace, &item.issue)?
    else {
        return Ok(());
    };
    if cancel.is_cancelled()
        || item.tracked.as_ref().is_none_or(|tracked| {
            matches!(
                tracked.state,
                IssueAutomationState::Done
                    | IssueAutomationState::Cancelled
                    | IssueAutomationState::Failed
            )
        })
    {
        return Ok(());
    }
    if !service
        .store
        .get_collection(&service.workspace)?
        .is_some_and(|collection| collection.config.mode == IssueCollectionMode::Observe)
    {
        if work.delivery_assessment.is_some() {
            service.invalidate_delivery(&mut work, IssueWorkBlockReason::ApprovalChanged)?;
        }
        return Ok(());
    }
    // PR 생성 전의 Queued 상태도 포함한다. 로컬 queue만 읽고 원격 fetch는 하지 않는다.
    let source = match service
        .execution
        .frozen_delivery_source(&service.workspace, &work.order.work_order_id)
    {
        Ok(source) => source,
        Err(error) => {
            service.invalidate_delivery(&mut work, IssueWorkBlockReason::SourceUnavailable)?;
            return Err(error);
        }
    };
    let Some(source) = source else {
        if work.delivery_assessment.is_some() {
            service.invalidate_delivery(&mut work, IssueWorkBlockReason::SourceUnavailable)?;
        }
        return Ok(());
    };
    let triage =
        service.revoke_on_current_error(Some(&work), service.triage.assess_current(item))?;
    let unchanged = work.delivery_assessment.as_ref().is_some_and(|assessment| {
        assessment.matches_triage(
            &triage,
            work.current_authorization_basis().authorization_epoch,
        ) && assessment
            .basis
            .delivery
            .as_ref()
            .is_some_and(|range| range.source_commit_sha == source)
    });
    let answered = if unchanged && !work.delivery_granted {
        let assessment = work
            .delivery_assessment
            .as_ref()
            .expect("matching delivery assessment");
        !assessment.verdict.requires_human()
            || service
                .store
                .get_issue_decision(
                    &service.workspace,
                    &delivery_decision_id(&assessment.basis)?,
                )?
                .is_some_and(|stored| human_delivery(&stored.request, assessment))
    } else {
        false
    };
    if !unchanged || answered {
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

fn human_delivery(request: &IssueDecisionRequest, assessment: &IssueDeliveryAssessment) -> bool {
    assessment.matches_decision(request.definition())
        && request.state() == IssueDecisionState::Answered
        && request.answer().is_some_and(|answer| {
            answer.option == "deliver"
                && answer.by == IssueDecisionAnswerBy::Human
                && answer.confirmation.is_some()
        })
}

fn delivery_definition(
    issue: &IssueReference,
    assessment: &IssueDeliveryAssessment,
) -> Result<IssueDecisionDefinition> {
    let text = |value: &str| IssueDecisionSharedText::new_shared(value);
    Ok(IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(issue.clone()),
        gate: IssueAutomationGate::Delivery,
        kind: IssueDecisionKind::RiskException,
        question: text("이 변경 결과를 제출할까요?")?,
        options: vec![
            IssueDecisionOption {
                id: "deliver".into(),
                label: text("이 범위 제출 승인")?,
                effect: text(
                    "표시된 고정 커밋 범위의 push와 PR 생성을 허용합니다. 통합에는 별도 사람 리뷰가 필요합니다.",
                )?,
            },
            IssueDecisionOption {
                id: "hold".into(),
                label: text("제출 보류")?,
                effect: text("이 변경 결과의 push와 PR 생성을 허용하지 않습니다.")?,
            },
        ],
        recommended: None,
        basis: assessment.basis.clone(),
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "issue-coordinator".into(),
        evidence: Vec::new(),
        delivery_review: Some(assessment.verdict.clone()),
    })
}
