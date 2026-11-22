//! Akra planning 등록/permit을 중립 이슈 실행 계약으로 옮긴다.
use crate::application::port::outbound::issue_work_execution_port::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControl, PlanningWorkControlAction, PlanningWorkInstruction,
    PlanningWorkInstructionAdmission, PlanningWorkOrderRepositoryPort, PlanningWorkPermit,
    PlanningWorkPermitBasis, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::task_mutation::PlanningTaskCreateInput;
use crate::application::service::planning::{PlanningWorkOrderRequest, PlanningWorkOrderService};
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::planning::PriorityQueueService;
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
    fn control(&self, workspace: &str, work_order_id: &str) -> Result<PlanningWorkControl> {
        self.repository
            .load_work_order_control(workspace, work_order_id)
    }
    fn apply_control(
        &self,
        workspace: &str,
        work_order_id: &str,
        command_id: &str,
        action: PlanningWorkControlAction,
    ) -> Result<PlanningWorkControl> {
        self.repository
            .apply_work_order_control(workspace, work_order_id, command_id, action)
    }
    fn queue_instruction(
        &self,
        workspace: &str,
        work_order_id: &str,
        instruction: &PlanningWorkInstruction,
    ) -> Result<PlanningWorkInstructionAdmission> {
        self.repository
            .append_work_order_instruction(workspace, work_order_id, instruction)
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
        let mut sources: Vec<String> = self
            .authority
            .load_runtime_projections(workspace)?
            .distributor_queue_records
            .into_iter()
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
        // 실제 dispatch와 동일한 configured remote/base를 새로 fetch/검증한다. checkout HEAD는 쓰지 않는다.
        let dispatch_source = if basis.is_none() && frozen_source.is_none() {
            Some(
                self.parallel
                    .work_order_dispatch_source_sha(workspace)
                    .map_err(|_| anyhow!("the configured work dispatch source is unavailable"))?,
            )
        } else {
            None
        };
        Ok(IssueWorkExecutionSnapshot {
            task_ids,
            dispatch_source,
            frozen_source,
            saved_source: basis.as_ref().map(|basis| basis.source_sha.clone()),
            saved_gate_revision: basis.as_ref().map(|basis| basis.gate_revision).unwrap_or(0),
            saved_authorization_epoch: basis
                .as_ref()
                .map(|basis| basis.authorization_epoch)
                .unwrap_or(0),
            blocked_reason: None,
        })
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
