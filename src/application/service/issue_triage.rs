//! 저장된 현재 관찰을 순수 판정에 넘기고 CAS로 저장한다. 원격 호출이나 실행 허가는 없다.
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueTriageFence, IssueTriageSaveResult,
};
use crate::domain::issue_automation::collection::{
    IssueCollectionDetail, IssueCollectionItem, IssueCollectionMode, IssueCollectionPage,
    IssueCollectionQuery, IssueTriageObserveReport,
};
use crate::domain::issue_automation::source::IssueSourceRead;
use crate::domain::issue_automation::triage::{
    IssueObservationDisposition, IssueTriageAssessment, IssueTriageInput, triage_issue,
};
use crate::domain::issue_automation::{IssueAutomationBindingId, IssueAutomationState};
use crate::domain::issue_validation::IssueRequirements;
use crate::domain::issue_work::{IssueFieldHash, IssueProvider, IssueReference};

use super::issue_automation_policy::{
    IssueAutomationPolicyInspection, IssueAutomationPolicyService, ResolvedIssueAutomationPolicy,
};
use super::issue_collection::IssueCollectionCancellation;
use super::issue_policy::{IssuePolicyService, issue_policy_version};

const TRIAGE_BATCH_SIZE: u16 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTriagedFact {
    pub issue: IssueReference,
    pub binding_id: IssueAutomationBindingId,
    pub revision: u64,
    pub assessment_revision: u64,
    pub gate_revision: u64,
    pub policy_version: String,
    pub field_hash: Option<IssueFieldHash>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueTriageRunOutcome {
    pub inspected: usize,
    pub changed: usize,
    pub discarded: bool,
    pub policy_blocked: bool,
}

struct TriageContext {
    automation: ResolvedIssueAutomationPolicy,
    runtime_selection_revision: u64,
    requirements: IssueRequirements,
    validation_policy_version: String,
}

impl TriageContext {
    fn assess(&self, item: &IssueCollectionItem) -> Result<IssueTriageAssessment> {
        triage_issue(IssueTriageInput {
            runtime_selection_revision: self.runtime_selection_revision,
            issue: &item.issue,
            current: item.current.as_ref(),
            policy: &self.automation.policy,
            automation_policy_version: &self.automation.version,
            requirements: &self.requirements,
            validation_policy_version: &self.validation_policy_version,
            held: item
                .tracked
                .as_ref()
                .is_some_and(|tracked| tracked.flags.held),
            pinned_source_commit: None,
            previous: item.triage.as_ref().map(|triage| &triage.assessment),
        })
        .map_err(|_| anyhow!("the issue triage input could not be assessed"))
    }
}

pub struct IssueTriageService {
    workspace_dir: String,
    store: Arc<dyn IssueAutomationStorePort>,
    automation_policy: IssueAutomationPolicyService,
    validation_policy: IssuePolicyService,
}

impl IssueTriageService {
    pub fn new(
        workspace_dir: String,
        store: Arc<dyn IssueAutomationStorePort>,
        automation_policy: IssueAutomationPolicyService,
        validation_policy: IssuePolicyService,
    ) -> Self {
        Self {
            workspace_dir,
            store,
            automation_policy,
            validation_policy,
        }
    }

    /// 장부와 공급자에 접근하지 않고 대표 workspace의 정책 파일만 읽는다.
    pub fn policy_inspection(&self) -> IssueAutomationPolicyInspection {
        self.automation_policy.inspect(&self.workspace_dir)
    }

    /// 실행 허가 전에 현재 사용자·프로젝트 상한을 읽는다. 표시용 문자열은 재해석하지 않는다.
    pub(crate) fn execution_attempt_limit(&self) -> Result<u32> {
        self.automation_policy
            .resolve(&self.workspace_dir)
            .map(|resolved| resolved.policy.budget.max_attempts)
            .map_err(|_| anyhow!("the current execution attempt policy is unavailable"))
    }

    /// 실행 뒤 diff에도 Start와 같은 합성 정책 판의 보호 경로를 적용한다.
    pub(crate) fn assess_delivery_diff_for_policy(
        &self,
        expected_version: &str,
        diff: &crate::domain::issue_automation::delivery::IssueDeliveryDiff,
    ) -> Result<crate::domain::issue_automation::delivery::IssueDeliveryDiffVerdict> {
        let context = self
            .context()
            .ok_or_else(|| anyhow!("the current delivery protection policy is unavailable"))?;
        if context.automation.version != expected_version {
            bail!("the delivery protection policy changed");
        }
        crate::domain::issue_automation::delivery::assess_delivery_diff(
            &context.automation.policy,
            diff,
        )
        .map_err(|error| anyhow!(error))
    }

    /// 중재자 입력도 현재 판정과 같은 효과 정책만 사용한다.
    pub(crate) fn automation_policy_for_version(
        &self,
        expected_version: &str,
    ) -> Result<crate::domain::issue_automation::policy::IssueAutomationPolicy> {
        let context = self
            .context()
            .ok_or_else(|| anyhow!("the current issue automation policy is unavailable"))?;
        if context.automation.version != expected_version {
            bail!("the issue automation policy changed");
        }
        Ok(context.automation.policy)
    }

    /// 요청과 같은 정책 판의 기한 설정만 사용한다.
    pub(crate) fn decision_timeouts_for_policy(
        &self,
        expected_version: &str,
    ) -> Result<crate::domain::issue_automation::policy::IssueAutomationTimeouts> {
        let context = self
            .context()
            .ok_or_else(|| anyhow!("the current decision deadline policy could not be assessed"))?;
        if context.automation.version != expected_version {
            bail!("the decision deadline policy changed")
        }
        Ok(context.automation.policy.timeouts.clone())
    }

    /// Admission consumes only the budget from the policy revision just assessed.
    pub(crate) fn start_budget_for_policy(
        &self,
        expected_version: &str,
    ) -> Result<crate::domain::issue_automation::policy::IssueAutomationBudget> {
        let context = self
            .context()
            .ok_or_else(|| anyhow!("the current issue start budget could not be assessed"))?;
        if context.automation.version != expected_version {
            return Err(anyhow!("the approved issue start policy changed"));
        }
        Ok(context.automation.policy.budget.clone())
    }

    /// Freeze only the close policy represented by the already approved policy revision.
    pub(crate) fn provider_link_for_policy(&self, expected_version: &str) -> Result<bool> {
        let context = self
            .context()
            .ok_or_else(|| anyhow!("the current issue policy could not be assessed"))?;
        if context.automation.version != expected_version {
            return Err(anyhow!("the approved issue close policy changed"));
        }
        Ok(context.automation.policy.close.when
            == crate::domain::issue_automation::policy::IssueAutomationCloseWhen::ProviderLink)
    }

    /// Close는 실행 전 triage의 disabled gate와 별개인 통합 후 사람 승인이다.
    /// 현재 사용자 동의와 런타임 상한이 같은 정책 판에 남아 있어야 요청을 만든다.
    pub(crate) fn integration_close_allowed(&self, expected_policy_version: &str) -> Result<bool> {
        use crate::domain::issue_automation::IssueAutomationAutonomy;
        use crate::domain::issue_automation::policy::IssueAutomationCloseWhen;
        let (resolved, _) = super::issue_autonomy::resolve_runtime_policy(
            &self.workspace_dir,
            self.store.as_ref(),
            &self.automation_policy,
        )?;
        Ok(resolved.version == expected_policy_version
            && resolved.policy.selected_mode() >= IssueAutomationAutonomy::Propose
            && resolved.policy.feedback.transition
            && resolved.policy.close.when == IssueAutomationCloseWhen::Integration)
    }

    /// 결정 답을 적용하기 직전에 현재 관찰과 현재 정책으로 다시 판정한다.
    /// 저장된 gate revision만 믿지 않으며, 정책을 읽지 못하면 기존 판으로 대신하지 않는다.
    pub(crate) fn assess_current(
        &self,
        item: &IssueCollectionItem,
    ) -> Result<IssueTriageAssessment> {
        self.validate_reference(&item.issue)?;
        self.context()
            .ok_or_else(|| anyhow!("the current issue policy could not be assessed"))?
            .assess(item)
    }
    /// callback은 수집과 같은 commit gate 안에서 확정 사실만 전달한다.
    /// 취소/후속 실패가 발생해도 이미 보낸 fact는 취소하지 않는다.
    pub fn triage_all(
        &self,
        cancel: &IssueCollectionCancellation,
        committed: &(dyn Fn(Vec<IssueTriagedFact>) + Send + Sync),
    ) -> Result<IssueTriageRunOutcome> {
        let mut outcome = IssueTriageRunOutcome::default();
        if cancel.is_cancelled() {
            outcome.discarded = true;
            return Ok(outcome);
        }
        let state = self
            .store
            .get_collection(&self.workspace_dir)
            .map_err(ledger_unavailable)?;
        let Some(state) = state.filter(|state| state.config.mode == IssueCollectionMode::Observe)
        else {
            outcome.discarded = true;
            return Ok(outcome);
        };
        let Some(context) = self.context() else {
            outcome.policy_blocked = true;
            return Ok(outcome);
        };
        let epoch = state.epoch;
        let mut after = None;
        loop {
            if cancel.is_cancelled() {
                outcome.discarded = true;
                return Ok(outcome);
            }
            let batch = self
                .store
                .read_triage_batch(&self.workspace_dir, after.as_ref(), TRIAGE_BATCH_SIZE)
                .map_err(ledger_unavailable)?;
            if !batch.collection.as_ref().is_some_and(|current| {
                current.config.mode == IssueCollectionMode::Observe && current.epoch == epoch
            }) {
                outcome.discarded = true;
                return Ok(outcome);
            }
            for item in batch.items {
                if cancel.is_cancelled() {
                    outcome.discarded = true;
                    return Ok(outcome);
                }
                let assessment = context.assess(&item)?;
                let expected = IssueTriageFence {
                    binding_run_revision: item.binding_run_revision,
                    collection_epoch: epoch,
                    observation_revision: item.observation_revision,
                    tracked_revision: item.tracked_revision,
                    assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
                };
                let result = cancel.commit_if_active(|| {
                    let result = self
                        .store
                        .save_triage(&self.workspace_dir, expected, &assessment)
                        .map_err(ledger_unavailable)?;
                    if let IssueTriageSaveResult::Saved {
                        record,
                        binding_id,
                        catalog_revision,
                        changed: true,
                    } = &result
                    {
                        let gate_revision = record
                            .assessment
                            .gates()
                            .iter()
                            .map(|gate| gate.gate_revision)
                            .max()
                            .ok_or_else(|| {
                                anyhow!("the committed issue assessment has no gates")
                            })?;
                        committed(vec![IssueTriagedFact {
                            issue: item.issue.clone(),
                            binding_id: binding_id.clone(),
                            revision: *catalog_revision,
                            assessment_revision: record.revision,
                            gate_revision,
                            policy_version: record
                                .assessment
                                .basis()
                                .automation_policy_version
                                .clone(),
                            field_hash: record.assessment.basis().field_hash.clone(),
                        }]);
                    }
                    Ok(result)
                })?;
                let Some(IssueTriageSaveResult::Saved { changed, .. }) = result else {
                    outcome.discarded = true;
                    return Ok(outcome);
                };
                outcome.inspected += 1;
                outcome.changed += usize::from(changed);
            }
            match batch.next_after {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        Ok(outcome)
    }

    pub fn issues(&self, query: &IssueCollectionQuery) -> Result<IssueCollectionPage> {
        query.validate()?;
        if let Some(after) = &query.after {
            self.validate_reference(after)?;
        }
        let mut items = Vec::new();
        let maximum = usize::from(query.limit) + 1;
        let report = self.scan_projected(query.binding_id.as_ref(), &mut |item| {
            let after = query
                .after
                .as_ref()
                .is_none_or(|after| issue_key(&item.issue) > issue_key(after));
            if after && items.len() < maximum {
                items.push(item);
            }
            Ok(())
        })?;
        let has_more = items.len() > usize::from(query.limit);
        items.truncate(usize::from(query.limit));
        let next_after = has_more
            .then(|| items.last().map(|item| item.issue.clone()))
            .flatten();
        Ok(IssueCollectionPage {
            revision: report.revision,
            items,
            next_after,
            report: Some(report),
        })
    }

    pub fn issue(&self, reference: &IssueReference) -> Result<IssueCollectionDetail> {
        self.validate_reference(reference)?;
        let mut item = None;
        let report = self.scan_projected(None, &mut |candidate| {
            if candidate.issue == *reference {
                item = Some(candidate);
            }
            Ok(())
        })?;
        Ok(IssueCollectionDetail {
            arbiter_waits: Vec::new(),
            timeline: None,
            revision: report.revision,
            item,
            report: Some(report),
            interaction: None,
            execution: None,
            feedback: None,
        })
    }

    pub fn report(
        &self,
        binding_id: Option<&IssueAutomationBindingId>,
    ) -> Result<IssueTriageObserveReport> {
        self.scan_projected(binding_id, &mut |_| Ok(()))
    }

    fn scan_projected(
        &self,
        binding_id: Option<&IssueAutomationBindingId>,
        visitor: &mut dyn FnMut(IssueCollectionItem) -> Result<()>,
    ) -> Result<IssueTriageObserveReport> {
        // All file reads precede the DB snapshot. The visitor only performs pure evaluation.
        let context = self.context();
        let mut report = IssueTriageObserveReport {
            binding_id: binding_id.cloned(),
            policy_version: context
                .as_ref()
                .map(|context| context.automation.version.clone()),
            assumed_mode: context
                .as_ref()
                .map(|context| context.automation.policy.authority.autonomy_cap),
            policy_blocked: context.is_none(),
            ..IssueTriageObserveReport::default()
        };
        let revision = self
            .store
            .scan_collection_issues(&self.workspace_dir, binding_id, &mut |mut item| {
                let current = context
                    .as_ref()
                    .and_then(|context| context.assess(&item).ok());
                if let Some(stored) = &mut item.triage {
                    stored.stale |= current.as_ref().is_none_or(|current| {
                        current.input_fingerprint() != stored.assessment.input_fingerprint()
                    });
                }
                count(&mut report, &item)?;
                visitor(item)
            })
            .map_err(ledger_unavailable)?;
        report.revision = revision;
        Ok(report)
    }

    pub fn autonomy_projection(
        &self,
    ) -> crate::domain::issue_automation::autonomy::IssueAutonomyProjection {
        super::issue_autonomy::projection(
            &self.workspace_dir,
            self.store.as_ref(),
            &self.automation_policy,
        )
    }

    fn context(&self) -> Option<TriageContext> {
        let (automation, runtime_selection_revision) =
            super::issue_autonomy::resolve_runtime_policy(
                &self.workspace_dir,
                self.store.as_ref(),
                &self.automation_policy,
            )
            .ok()?;
        let validation = self.validation_policy.resolve(&self.workspace_dir).ok()?;
        Some(TriageContext {
            automation,
            runtime_selection_revision,
            requirements: IssueRequirements {
                template_fields: Vec::new(),
                required_sections: validation.convention.required_sections.clone(),
            },
            validation_policy_version: issue_policy_version(&validation),
        })
    }

    fn validate_reference(&self, reference: &IssueReference) -> Result<()> {
        if reference.project_scope.workspace_dir != self.workspace_dir
            || [
                &reference.server_id,
                &reference.repository_id,
                &reference.issue_id,
            ]
            .iter()
            .any(|value| value.trim().is_empty() || value.contains('\0'))
        {
            bail!("the issue query reference does not match its project scope")
        }
        Ok(())
    }
}

fn count(report: &mut IssueTriageObserveReport, item: &IssueCollectionItem) -> Result<()> {
    increment(&mut report.total)?;
    if report.policy_blocked {
        return increment(&mut report.blocked);
    }
    if matches!(item.current, None | Some(IssueSourceRead::Unknown { .. })) {
        return increment(&mut report.unknown);
    }
    let Some(triage) = &item.triage else {
        return increment(&mut report.untriaged);
    };
    if triage.stale {
        return increment(&mut report.stale);
    }
    if let Some(tracked) = &item.tracked {
        match tracked.state {
            IssueAutomationState::Done | IssueAutomationState::Cancelled => {
                return increment(&mut report.excluded);
            }
            IssueAutomationState::Queued
            | IssueAutomationState::Running
            | IssueAutomationState::InReview
            | IssueAutomationState::AwaitingDecision
            | IssueAutomationState::Failed => return increment(&mut report.deferred),
            _ => {}
        }
    }
    if item.start_budget_wait.is_some()
        || triage.assessment.reasons().contains(
            &crate::domain::issue_automation::triage::IssueTriageReasonCode::BudgetExhausted,
        )
    {
        return increment(&mut report.deferred);
    }
    match triage.assessment.observation().disposition {
        IssueObservationDisposition::AutomaticStart => increment(&mut report.automatic_start),
        IssueObservationDisposition::HumanApproval => increment(&mut report.human_approval),
        IssueObservationDisposition::Deferred => increment(&mut report.deferred),
        IssueObservationDisposition::Excluded => increment(&mut report.excluded),
        IssueObservationDisposition::NeedsInfo => increment(&mut report.needs_info),
        IssueObservationDisposition::Unknown => increment(&mut report.unknown),
        IssueObservationDisposition::ObservationOnly => increment(&mut report.observation_only),
    }
}

fn increment(value: &mut u64) -> Result<()> {
    *value = value
        .checked_add(1)
        .ok_or_else(|| anyhow!("the issue observation count overflowed"))?;
    Ok(())
}

fn issue_key(issue: &IssueReference) -> (&str, &str, &str, &str, &str) {
    let provider = match issue.provider {
        IssueProvider::GitHub => "github",
        IssueProvider::Jira => "jira",
        IssueProvider::Linear => "linear",
    };
    (
        provider,
        &issue.server_id,
        &issue.repository_id,
        &issue.issue_id,
        &issue.project_scope.workspace_dir,
    )
}

fn ledger_unavailable(_: anyhow::Error) -> anyhow::Error {
    anyhow!("the issue automation ledger is unavailable")
}

#[cfg(test)]
mod tests;
