//! 작업별 개입을 영속 intent → 실행 권한 fence → 완료 영수증 순서로 확정한다.
use crate::application::port::inbound::issue_intervention_port::IssueInterventionPort;
use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionPort;
use crate::application::port::outbound::issue_work_steering_port::{
    IssueWorkSteerRequest, IssueWorkSteeringPort,
};
use crate::domain::issue_automation::coordinator::issue_work_order_id;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest, IssueSteerDelivery,
};
use crate::domain::issue_automation::source::{IssueSourceContent, IssueSourceRead};
use crate::domain::issue_automation::work_control::{
    IssueWorkControlAction, IssueWorkInstruction, IssueWorkInstructionAdmission,
};
use crate::domain::issue_work::{IssueReference, IssueSharedCopy};
use anyhow::{Result, bail};
use std::sync::{Arc, Mutex};

pub struct IssueInterventionService {
    workspace: String,
    store: Arc<dyn IssueAutomationStorePort>,
    execution: Arc<dyn IssueWorkExecutionPort>,
    steering: Arc<dyn IssueWorkSteeringPort>,
    command_gate: Mutex<()>,
    triage: Option<Arc<super::issue_triage::IssueTriageService>>,
    policy: Option<super::issue_policy::IssuePolicyService>,
}

impl IssueInterventionService {
    pub fn new(
        workspace: String,
        store: Arc<dyn IssueAutomationStorePort>,
        execution: Arc<dyn IssueWorkExecutionPort>,
        steering: Arc<dyn IssueWorkSteeringPort>,
    ) -> Self {
        Self {
            workspace,
            store,
            execution,
            steering,
            command_gate: Mutex::new(()),
            triage: None,
            policy: None,
        }
    }

    pub fn with_triage(mut self, triage: Arc<super::issue_triage::IssueTriageService>) -> Self {
        self.triage = Some(triage);
        self
    }
    pub fn with_policy(mut self, policy: super::issue_policy::IssuePolicyService) -> Self {
        self.policy = Some(policy);
        self
    }

    fn apply(&self, request: &IssueInterventionRequest) -> Result<IssueInterventionReceipt> {
        let work_order_id = issue_work_order_id(&request.issue);
        if matches!(request.action, IssueInterventionAction::Steer { .. }) {
            return self.steer(request);
        }
        let handoff = if matches!(request.action, IssueInterventionAction::TakeOver)
            && !self
                .execution
                .has_active_lease(&self.workspace, &work_order_id)?
        {
            current_shared_copy(
                self.store.as_ref(),
                &self.workspace,
                &request.issue,
                self.policy.as_ref(),
            )?
        } else {
            None
        };
        let receipt = apply_control(
            &self.workspace,
            self.store.as_ref(),
            self.execution.as_ref(),
            request,
            handoff,
        )?;
        if matches!(
            request.action,
            IssueInterventionAction::Retriage
                | IssueInterventionAction::Release
                | IssueInterventionAction::Track
        ) {
            self.retriage(&request.issue)?;
        }
        Ok(receipt)
    }

    fn completed_steer(
        &self,
        request: &IssueInterventionRequest,
    ) -> Result<Option<IssueInterventionReceipt>> {
        let Some(stored) = self
            .store
            .get_issue_intervention(&self.workspace, &request.command_id)?
        else {
            return Ok(None);
        };
        if stored.request != *request {
            bail!("issue intervention command_id was reused for different content");
        }
        Ok(stored.receipt.map(|mut receipt| {
            receipt.changed = false;
            receipt
        }))
    }

    fn steer(&self, request: &IssueInterventionRequest) -> Result<IssueInterventionReceipt> {
        if let Some(receipt) = self.completed_steer(request)? {
            return Ok(receipt);
        }
        let IssueInterventionAction::Steer { message } = &request.action else {
            bail!("a steering request is required");
        };
        let work_order_id = issue_work_order_id(&request.issue);
        let control = self.execution.control(&self.workspace, &work_order_id)?;
        if control.cancelled || control.taken_over {
            bail!("the work is no longer automatically owned");
        }
        let mut receipt = receipt(request, &work_order_id, control.authorization_epoch, true);
        receipt.delivery = Some(IssueSteerDelivery::Unknown);
        if !self
            .store
            .claim_issue_steer_delivery(&self.workspace, request, &receipt)?
        {
            return self
                .completed_steer(request)?
                .ok_or_else(|| anyhow::anyhow!("the claimed steering receipt is unavailable"));
        }
        // The intent and Unknown receipt are durable together before admission or external delivery.
        // A crash or an uncertain admission cannot leave a pending command that blocks later controls.
        let instruction = IssueWorkInstruction {
            command_id: request.command_id.clone(),
            message: message.clone(),
        };
        let admission = match self.execution.queue_instruction(
            &self.workspace,
            &work_order_id,
            &instruction,
        ) {
            Ok(admission) => admission,
            Err(_) => {
                tracing::warn!(command_id = %request.command_id, "issue steering admission was not confirmed; preserving Unknown without retry");
                return Ok(receipt);
            }
        };
        match admission {
            IssueWorkInstructionAdmission::Queued { changed } => {
                receipt.delivery = Some(IssueSteerDelivery::Queued);
                receipt.changed = changed;
                self.store
                    .finish_issue_intervention(&self.workspace, &receipt)?;
            }
            IssueWorkInstructionAdmission::Active => {
                let task_ids = match self.execution.task_ids(&self.workspace, &work_order_id) {
                    Ok(task_ids) => task_ids,
                    Err(_) => return Ok(receipt),
                };
                let result = self.steering.steer(&IssueWorkSteerRequest {
                    workspace_dir: self.workspace.clone(),
                    work_order_id,
                    task_ids,
                    command_id: request.command_id.clone(),
                    message: message.clone(),
                });
                if matches!(result, Ok(IssueSteerDelivery::Sent)) {
                    receipt.delivery = Some(IssueSteerDelivery::Sent);
                    self.store
                        .finish_issue_intervention(&self.workspace, &receipt)?;
                }
            }
        }
        Ok(receipt)
    }

    /// Called under the repository kernel lock before any automatic runtime is constructed.
    pub fn recover_before_runtime(&self) -> Result<()> {
        let mut receipts = Vec::new();
        for pending in self.store.pending_issue_interventions(&self.workspace)? {
            if !matches!(
                pending.request.action,
                IssueInterventionAction::Steer { .. }
            ) {
                receipts.push(fence_control(
                    &self.workspace,
                    self.execution.as_ref(),
                    &pending.request,
                    None,
                )?);
            }
        }
        // Every control fence must be durable before receipt recovery starts.
        for receipt in receipts {
            if self
                .store
                .finish_issue_intervention(&self.workspace, &receipt)
                .is_err()
            {
                tracing::warn!(command_id = %receipt.command_id, "issue control is fenced but its receipt remains pending");
            }
        }
        Ok(())
    }

    fn retriage(&self, issue: &IssueReference) -> Result<()> {
        let Some(triage) = &self.triage else {
            return Ok(());
        };
        let mut after = None;
        loop {
            let batch = self
                .store
                .read_triage_batch(&self.workspace, after.as_ref(), 100)?;
            let Some(collection) = batch.collection else {
                return Ok(());
            };
            if let Some(item) = batch.items.into_iter().find(|item| item.issue == *issue) {
                let assessment = triage.assess_current(&item)?;
                let fence=crate::application::port::outbound::issue_automation_store_port::IssueTriageFence{
                    binding_run_revision: item.binding_run_revision,
                    collection_epoch:collection.epoch,observation_revision:item.observation_revision,
                    tracked_revision:item.tracked_revision,assessment_revision:item.triage.as_ref().map(|record|record.revision),
                };
                self.store
                    .save_triage(&self.workspace, fence, &assessment)?;
                return Ok(());
            }
            after = batch.next_after;
            if after.is_none() {
                return Ok(());
            }
        }
    }
}

impl IssueInterventionPort for IssueInterventionService {
    fn intervene(
        &self,
        request: IssueInterventionRequest,
        _context: &StoredCommandContext,
    ) -> Result<IssueInterventionReceipt> {
        request.validate()?;
        if request
            .command_id
            .starts_with(crate::domain::issue_automation::autonomy::ISSUE_AUTONOMY_CONTROL_PREFIX)
            || request.command_id.starts_with(
                crate::domain::issue_automation::binding_pause::ISSUE_BINDING_PAUSE_CONTROL_PREFIX,
            )
        {
            bail!("the intervention command ID uses a reserved control namespace");
        }
        if request.issue.project_scope.workspace_dir != self.workspace {
            bail!("the issue intervention belongs to another workspace");
        }
        let _guard = self
            .command_gate
            .lock()
            .map_err(|_| anyhow::anyhow!("issue intervention command gate is unavailable"))?;
        recover_pending_controls(
            &self.workspace,
            self.store.as_ref(),
            self.execution.as_ref(),
        )?;
        if matches!(request.action, IssueInterventionAction::Steer { .. }) {
            return self.steer(&request);
        }
        let stored = self
            .store
            .begin_issue_intervention(&self.workspace, &request)?;
        if let Some(mut receipt) = stored.receipt {
            receipt.changed = false;
            return Ok(receipt);
        }
        self.apply(&request)
    }
}

/// 재시작 뒤 미완료 제어를 먼저 확정한다. steer의 외부 송신은 여기에서 재시도하지 않는다.
pub(crate) fn recover_pending_controls(
    workspace: &str,
    store: &dyn IssueAutomationStorePort,
    execution: &dyn IssueWorkExecutionPort,
) -> Result<()> {
    for pending in store.pending_issue_interventions(workspace)? {
        if matches!(
            pending.request.action,
            IssueInterventionAction::Steer { .. }
        ) {
            continue;
        }
        if apply_control(workspace, store, execution, &pending.request, None).is_err() {
            tracing::warn!(command_id = %pending.request.command_id, "issue control remains pending; unrelated issues may continue");
        }
    }
    Ok(())
}

fn apply_control(
    workspace: &str,
    store: &dyn IssueAutomationStorePort,
    execution: &dyn IssueWorkExecutionPort,
    request: &IssueInterventionRequest,
    handoff: Option<IssueSharedCopy>,
) -> Result<IssueInterventionReceipt> {
    let receipt = fence_control(workspace, execution, request, handoff)?;
    store.finish_issue_intervention(workspace, &receipt)?;
    Ok(receipt)
}

fn fence_control(
    workspace: &str,
    execution: &dyn IssueWorkExecutionPort,
    request: &IssueInterventionRequest,
    handoff: Option<IssueSharedCopy>,
) -> Result<IssueInterventionReceipt> {
    request.validate()?;
    let work_order_id = issue_work_order_id(&request.issue);
    let action = match request.action {
        IssueInterventionAction::Hold => IssueWorkControlAction::Hold,
        IssueInterventionAction::Release => IssueWorkControlAction::Release,
        IssueInterventionAction::TakeOver => IssueWorkControlAction::TakeOver,
        IssueInterventionAction::Retriage => IssueWorkControlAction::Retriage,
        IssueInterventionAction::Cancel { .. } => IssueWorkControlAction::Cancel,
        IssueInterventionAction::Track => IssueWorkControlAction::Track,
        IssueInterventionAction::Untrack => IssueWorkControlAction::Untrack,
        IssueInterventionAction::Steer { .. } => {
            bail!("a steering message is not a control mutation")
        }
    };
    let previous = execution.control(workspace, &work_order_id)?;
    let control =
        execution.apply_control(workspace, &work_order_id, &request.command_id, action)?;
    let mut receipt = receipt(
        request,
        &work_order_id,
        control.authorization_epoch,
        previous != control,
    );
    receipt.handoff = handoff;
    Ok(receipt)
}

fn receipt(
    request: &IssueInterventionRequest,
    work_order_id: &str,
    epoch: u64,
    changed: bool,
) -> IssueInterventionReceipt {
    IssueInterventionReceipt {
        issue: request.issue.clone(),
        command_id: request.command_id.clone(),
        action: request.action.label().into(),
        work_order_id: Some(work_order_id.into()),
        authorization_epoch: epoch,
        changed,
        delivery: None,
        handoff: None,
    }
}

fn current_shared_copy(
    store: &dyn IssueAutomationStorePort,
    workspace: &str,
    issue: &IssueReference,
    policy: Option<&super::issue_policy::IssuePolicyService>,
) -> Result<Option<IssueSharedCopy>> {
    let Some(policy) = policy else {
        return Ok(None);
    };
    let policy_version = super::issue_policy::issue_policy_version(&policy.resolve(workspace)?);
    let mut after = None;
    loop {
        let batch = store.read_triage_batch(workspace, after.as_ref(), 100)?;
        for item in batch.items {
            if item.issue == *issue {
                return Ok(match item.current {
                    Some(IssueSourceRead::Observed(observation)) => match observation.content {
                        IssueSourceContent::Shared(snapshot)
                            if snapshot.policy_version == policy_version =>
                        {
                            Some(snapshot.shared_copy)
                        }
                        IssueSourceContent::Shared(_) | IssueSourceContent::Withheld { .. } => None,
                    },
                    _ => None,
                });
            }
        }
        after = batch.next_after;
        if after.is_none() {
            return Ok(None);
        }
    }
}

#[cfg(test)]
mod tests;
