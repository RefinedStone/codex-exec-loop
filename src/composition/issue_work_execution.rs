//! Akra planning 등록/permit을 중립 이슈 실행 계약으로 옮긴다.
use crate::application::port::outbound::issue_work_execution_port::*;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthoritySnapshot;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControl, PlanningWorkControlAction, PlanningWorkInstruction,
    PlanningWorkInstructionAdmission, PlanningWorkOrderRepositoryPort, PlanningWorkPermit,
    PlanningWorkPermitBasis, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::task_mutation::PlanningTaskCreateInput;
use crate::application::service::planning::{PlanningWorkOrderRequest, PlanningWorkOrderService};
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::work_control::{
    IssueWorkControl, IssueWorkControlAction, IssueWorkInstruction, IssueWorkInstructionAdmission,
};
use crate::domain::planning::{PriorityQueueService, TaskStatus};
use anyhow::{Result, anyhow, bail};
use std::sync::Arc;

pub(crate) struct AkraIssueWorkExecutionAdapter {
    repository: Arc<dyn PlanningWorkPermitRepositoryPort>,
    authority: Arc<dyn PlanningAuthorityPort>,
    work_orders: PlanningWorkOrderService,
    parallel: ParallelModeService,
}

impl AkraIssueWorkExecutionAdapter {
    pub(crate) fn new(
        repository: Arc<dyn PlanningWorkPermitRepositoryPort>,
        authority: Arc<dyn PlanningAuthorityPort>,
        parallel: ParallelModeService,
    ) -> Self {
        let orders: Arc<dyn PlanningWorkOrderRepositoryPort> = repository.clone();
        Self {
            repository,
            authority,
            work_orders: PlanningWorkOrderService::new(orders, PriorityQueueService::new()),
            parallel,
        }
    }
}

impl IssueWorkExecutionPort for AkraIssueWorkExecutionAdapter {
    fn control(&self, workspace: &str, work_order_id: &str) -> Result<IssueWorkControl> {
        self.repository
            .load_work_order_control(workspace, work_order_id)
            .map(issue_control)
    }
    fn apply_control(
        &self,
        workspace: &str,
        work_order_id: &str,
        command_id: &str,
        action: IssueWorkControlAction,
    ) -> Result<IssueWorkControl> {
        self.repository
            .apply_work_order_control(
                workspace,
                work_order_id,
                command_id,
                planning_control_action(action),
            )
            .map(issue_control)
    }
    fn queue_instruction(
        &self,
        workspace: &str,
        work_order_id: &str,
        instruction: &IssueWorkInstruction,
    ) -> Result<IssueWorkInstructionAdmission> {
        let instruction = PlanningWorkInstruction {
            command_id: instruction.command_id.clone(),
            message: instruction.message.clone(),
        };
        self.repository
            .append_work_order_instruction(workspace, work_order_id, &instruction)
            .map(issue_instruction_admission)
    }
    fn has_active_lease(&self, workspace: &str, work_order_id: &str) -> Result<bool> {
        let ids = self.task_ids(workspace, work_order_id)?;
        Ok(self
            .authority
            .load_runtime_projections(workspace)?
            .slot_leases
            .values()
            .any(|lease| ids.contains(&lease.task_id)))
    }

    fn snapshot(&self, workspace: &str, work_order_id: &str) -> Result<IssueWorkExecutionSnapshot> {
        let order = self.repository.load_work_order(workspace, work_order_id)?;
        let task_ids = order
            .as_ref()
            .map(|order| order.task_ids.clone())
            .unwrap_or_default();
        let basis = self
            .repository
            .load_work_order_permit_basis(workspace, work_order_id)?;
        let runtime = self.authority.load_runtime_projections(workspace)?;
        execution_snapshot(
            task_ids,
            basis.as_ref(),
            &runtime,
            || self.repository.load_task_authority_snapshot(workspace),
            || {
                self.parallel
                    .work_order_dispatch_source_sha(workspace)
                    .map_err(|_| anyhow!("the configured work dispatch source is unavailable"))
            },
        )
    }

    fn task_ids(&self, workspace: &str, work_order_id: &str) -> Result<Vec<String>> {
        Ok(self
            .repository
            .load_work_order(workspace, work_order_id)?
            .map(|order| order.task_ids)
            .unwrap_or_default())
    }

    fn submit(&self, workspace: &str, order: &IssueWorkOrder) -> Result<Vec<String>> {
        if workspace != order.scope || workspace != order.issue.project_scope.workspace_dir {
            bail!("the issue work belongs to another workspace");
        }
        // 선택적 hint/profile은 이 첫 slice에서 지정하지 않는다. 실제 지원 없이 조용히 무시하지 않는다.
        if order.branch_hint.is_some() || order.profile.is_some() {
            bail!("issue work branch/profile hints are not supported");
        }
        let description = format!(
            "{}

Issue: {}
Work order: {}",
            order.handoff.body,
            order
                .link_reference
                .as_deref()
                .unwrap_or(&order.issue.issue_id),
            order.work_order_id
        );
        let admission = self
            .work_orders
            .register_work_order(PlanningWorkOrderRequest {
                workspace_directory: workspace.into(),
                work_order_id: order.work_order_id.clone(),
                origin: "issue_automation".into(),
                input: PlanningTaskCreateInput {
                    direction_id: None,
                    direction_relation_note: Some("사용자가 이슈 작업 시작을 승인했습니다.".into()),
                    title: order.title.clone(),
                    description: Some(description),
                    status: None,
                    base_priority: Some(order.priority),
                    dynamic_priority_delta: None,
                    priority_reason: Some("issue start approval".into()),
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                },
            })?;
        Ok(admission.work_order.task_ids)
    }

    fn grant(&self, workspace: &str, request: &IssueWorkPermitRequest) -> Result<()> {
        request.approval.validate()?;
        if request.authorization_epoch != request.approval.authorization_epoch
            || request.gate_revision < request.approval.gate_revision
        {
            bail!("the work permit does not match its approved authorization epoch");
        }
        let hash = request
            .approval
            .field_hash
            .as_ref()
            .ok_or_else(|| anyhow!("the work approval has no field hash"))?;
        let basis = PlanningWorkPermitBasis {
            authorization_epoch: request.authorization_epoch,
            policy_version: request.approval.policy_version.clone(),
            field_hash: format!("v{}:{}", hash.version, hash.sha256),
            gate_revision: request.gate_revision,
            source_sha: request.source_sha.clone(),
        };
        self.repository
            .set_work_order_permit_basis(workspace, &request.work_order_id, &basis)?;
        let stages: &[PlanningWorkPermitStage] = if request.delivery {
            &[
                PlanningWorkPermitStage::Push,
                PlanningWorkPermitStage::PullRequest,
                PlanningWorkPermitStage::Integrate,
            ]
        } else {
            &[PlanningWorkPermitStage::Dispatch]
        };
        for stage in stages {
            self.repository.grant_work_order_permit(
                workspace,
                &PlanningWorkPermit {
                    work_order_id: request.work_order_id.clone(),
                    stage: *stage,
                    basis: basis.clone(),
                    requires_human_review: *stage == PlanningWorkPermitStage::Integrate,
                },
            )?;
        }
        Ok(())
    }

    fn revoke(&self, workspace: &str, work_order_id: &str) -> Result<()> {
        if self
            .repository
            .load_work_order(workspace, work_order_id)?
            .is_none()
        {
            return Ok(());
        }
        self.repository.revoke_work_order_permits(
            workspace,
            work_order_id,
            &PlanningWorkPermitStage::ALL,
            "issue_approval_changed",
        )?;
        Ok(())
    }
}

fn issue_control(control: PlanningWorkControl) -> IssueWorkControl {
    let PlanningWorkControl {
        held,
        taken_over,
        cancelled,
        tracked,
        authorization_epoch,
    } = control;
    IssueWorkControl {
        held,
        taken_over,
        cancelled,
        tracked,
        authorization_epoch,
    }
}

fn planning_control_action(action: IssueWorkControlAction) -> PlanningWorkControlAction {
    match action {
        IssueWorkControlAction::Hold => PlanningWorkControlAction::Hold,
        IssueWorkControlAction::Release => PlanningWorkControlAction::Release,
        IssueWorkControlAction::TakeOver => PlanningWorkControlAction::TakeOver,
        IssueWorkControlAction::Cancel => PlanningWorkControlAction::Cancel,
        IssueWorkControlAction::Track => PlanningWorkControlAction::Track,
        IssueWorkControlAction::Untrack => PlanningWorkControlAction::Untrack,
        IssueWorkControlAction::Retriage => PlanningWorkControlAction::Retriage,
    }
}

fn issue_instruction_admission(
    admission: PlanningWorkInstructionAdmission,
) -> IssueWorkInstructionAdmission {
    match admission {
        PlanningWorkInstructionAdmission::Queued { changed } => {
            IssueWorkInstructionAdmission::Queued { changed }
        }
        PlanningWorkInstructionAdmission::Active => IssueWorkInstructionAdmission::Active,
    }
}

fn execution_snapshot(
    task_ids: Vec<String>,
    basis: Option<&PlanningWorkPermitBasis>,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
    load_tasks: impl FnOnce() -> Result<Option<PlanningTaskAuthoritySnapshot>>,
    fetch_dispatch_source: impl FnOnce() -> Result<String>,
) -> Result<IssueWorkExecutionSnapshot> {
    let mut sources: Vec<String> = runtime
        .distributor_queue_records
        .iter()
        .filter(|record| task_ids.contains(&record.task_id))
        .map(|record| record.effective_source_commit_sha())
        .filter(|source| !source.is_empty())
        .collect();
    sources.sort();
    sources.dedup();
    if sources.len() > 1 {
        bail!("the work has ambiguous frozen source commits");
    }
    let frozen_source = sources.pop();
    // Leased 단계도 배정이 끝난 상태다. 실제 실행/전달 source를 새 integration HEAD로 바꾸지 않는다.
    let pending_dispatch = if frozen_source.is_some()
        || runtime
            .slot_leases
            .values()
            .any(|lease| task_ids.contains(&lease.task_id))
    {
        false
    } else if task_ids.is_empty() {
        true
    } else {
        let tasks =
            load_tasks()?.ok_or_else(|| anyhow!("the planning execution tasks are unavailable"))?;
        let mut pending = true;
        for task_id in &task_ids {
            let task = tasks
                .task_authority
                .tasks
                .iter()
                .find(|task| &task.id == task_id)
                .ok_or_else(|| anyhow!("the work task is missing from planning authority"))?;
            pending &= matches!(task.status, TaskStatus::Ready | TaskStatus::Proposed);
        }
        pending
    };
    // 슬롯 대기 중에는 저장된 permit basis가 있어도 실제 dispatch와 같은 configured remote/base를 읽는다.
    let dispatch_source = if pending_dispatch {
        Some(fetch_dispatch_source()?)
    } else {
        None
    };
    Ok(IssueWorkExecutionSnapshot {
        task_ids,
        dispatch_source,
        frozen_source,
        saved_source: basis.map(|basis| basis.source_sha.clone()),
        saved_gate_revision: basis.map(|basis| basis.gate_revision).unwrap_or(0),
        saved_authorization_epoch: basis.map(|basis| basis.authorization_epoch).unwrap_or(0),
        blocked_reason: None,
    })
}

#[cfg(test)]
mod tests;
