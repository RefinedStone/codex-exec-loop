//! Akra planning 등록/permit을 중립 이슈 실행 계약으로 옮긴다.
use crate::application::port::outbound::issue_work_execution_port::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControl, PlanningWorkControlAction, PlanningWorkDeliveryAuthority,
    PlanningWorkDeliveryReview, PlanningWorkInstruction, PlanningWorkInstructionAdmission,
    PlanningWorkLinkReference, PlanningWorkOrderRepositoryPort, PlanningWorkPermit,
    PlanningWorkPermitBasis, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::task_mutation::PlanningTaskCreateInput;
use crate::application::service::planning::{PlanningWorkOrderRequest, PlanningWorkOrderService};
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::work_control::{
    IssueWorkControl, IssueWorkControlAction, IssueWorkInstruction, IssueWorkInstructionAdmission,
};
use crate::domain::planning::PriorityQueueService;
use anyhow::{Result, anyhow, bail};
use std::sync::Arc;

mod answer;
mod conversation;
mod criteria;
mod delivery_authority;
mod observation;
mod recovery;
mod source;

pub(crate) struct AkraIssueWorkExecutionAdapter {
    repository: Arc<dyn PlanningWorkPermitRepositoryPort>,
    authority: Arc<dyn PlanningAuthorityPort>,
    work_orders: PlanningWorkOrderService,
    parallel: ParallelModeService,
    criteria_store: Option<Arc<dyn crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort>>,
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
            criteria_store: None,
        }
    }

    pub(crate) fn with_criteria_store(
        mut self,
        store: Arc<dyn crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort>,
    ) -> Self {
        self.criteria_store = Some(store);
        self
    }
}

impl IssueWorkExecutionPort for AkraIssueWorkExecutionAdapter {
    fn frozen_delivery_source(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Option<String>> {
        source::frozen_delivery_source(self, workspace, work_order_id)
    }

    fn delivery_diff(
        &self,
        workspace: &str,
        work_order_id: &str,
        expected_source_sha: &str,
    ) -> Result<crate::domain::issue_automation::delivery::IssueDeliveryDiff> {
        source::delivery_diff(self, workspace, work_order_id, expected_source_sha)
    }

    fn recover_execution(
        &self,
        workspace: &str,
        request: &IssueWorkRecoveryRequest,
    ) -> Result<IssueWorkRecoveryReceipt> {
        recovery::recover(self, workspace, request)
    }

    fn execution_limit(
        &self,
        workspace: &str,
        work_order_id: &str,
        max_attempts: u32,
    ) -> Result<()> {
        self.repository
            .set_work_order_execution_limit(workspace, work_order_id, max_attempts)
    }

    fn permit_gate_revision(&self, workspace: &str, work_order_id: &str) -> Result<u64> {
        let basis = self
            .repository
            .load_work_order_permit_basis(workspace, work_order_id)?;
        Ok(basis.map_or(0, |basis| basis.gate_revision))
    }

    fn answer_question(
        &self,
        workspace: &str,
        request: &IssueWorkAnswerRequest,
    ) -> Result<IssueWorkAnswerReceipt> {
        answer::enqueue(self, workspace, request)
    }

    fn question_answer_receipt(
        &self,
        workspace: &str,
        work_order_id: &str,
        command_id: &str,
    ) -> Result<Option<IssueWorkAnswerReceipt>> {
        answer::receipt(self, workspace, work_order_id, command_id)
    }

    fn observe_execution(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Option<crate::domain::issue_automation::execution::IssueWorkExecutionObservation>>
    {
        observation::observe(self, workspace, work_order_id)
    }

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
    fn control_command_result(
        &self,
        workspace: &str,
        work_order_id: &str,
        command_id: &str,
        action: IssueWorkControlAction,
    ) -> Result<Option<IssueWorkControl>> {
        self.repository
            .load_work_order_control_command(
                workspace,
                work_order_id,
                command_id,
                planning_control_action(action),
            )
            .map(|result| result.map(issue_control))
    }
    fn apply_confirmed_control(
        &self,
        workspace: &str,
        request: &crate::application::port::outbound::issue_work_execution_port::ConfirmedIssueControl<'_>,
    ) -> Result<Option<IssueWorkControl>> {
        self.repository.apply_confirmed_work_order_control(
            workspace,
            &crate::application::port::outbound::planning_work_order_repository_port::ConfirmedWorkControl {
                work_order_id: request.work_order_id, command_id: request.command_id,
                action: planning_control_action(request.action), expected_epoch: request.expected_epoch,
                authorize: request.authorize,
            },
        ).map(|result| result.map(issue_control))
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

    fn conversation_thread_id(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Option<String>> {
        conversation::thread_id(self, workspace, work_order_id)
    }

    fn asking_worker(
        &self,
        workspace: &str,
        branch: &str,
    ) -> Result<
        Option<(
            String,
            crate::domain::issue_automation::agent_question::IssueAgentAsk,
        )>,
    > {
        conversation::asking_worker(self, workspace, branch)
    }

    fn await_question(
        &self,
        workspace: &str,
        work_order_id: &str,
        ask: &crate::domain::issue_automation::agent_question::IssueAgentAsk,
    ) -> Result<bool> {
        conversation::await_question(self, workspace, work_order_id, ask)
    }

    fn snapshot(&self, workspace: &str, work_order_id: &str) -> Result<IssueWorkExecutionSnapshot> {
        source::snapshot(self, workspace, work_order_id)
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
        // profile은 아직 지원하지 않는다. branch hint는 Linear의 branchName만 받는다.
        // 실제 지원 없이 조용히 무시하지 않는다.
        if order.profile.is_some()
            || (order.branch_hint.is_some()
                && order.issue.provider != crate::domain::issue_work::IssueProvider::Linear)
        {
            bail!("issue work branch/profile hints are not supported");
        }
        if order.branch_hint.is_some() && order.link_reference.is_none() {
            bail!("a branch hint requires the immutable issue link");
        }
        let link_reference = order
            .link_reference
            .as_ref()
            .map(|url| PlanningWorkLinkReference {
                url: url.clone(),
                close_on_merge: order.provider_link,
                branch_hint: order.branch_hint.clone(),
            });
        if order.provider_link && link_reference.is_none() {
            bail!("close.when: provider_link requires an issue link");
        }
        if let Some(reference) = &link_reference {
            self.parallel
                .validate_work_link_reference(workspace, reference)
                .map_err(anyhow::Error::msg)?;
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
        let acceptance = criteria::verified(self, workspace, order)?;
        let request = PlanningWorkOrderRequest {
            workspace_directory: workspace.into(),
            work_order_id: order.work_order_id.clone(),
            origin: "issue_automation".into(),
            link_reference,
            input: PlanningTaskCreateInput {
                acceptance: None,
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
        };
        let admission = match acceptance {
            Some(acceptance) => self
                .work_orders
                .register_work_order_with_acceptance(request, acceptance)?,
            None => self.work_orders.register_work_order(request)?,
        };
        Ok(admission.work_order.task_ids)
    }

    fn grant(&self, workspace: &str, request: &IssueWorkPermitRequest) -> Result<()> {
        request.approval.validate()?;
        if request.authorization_epoch != request.approval.authorization_epoch
            || request.gate_revision < request.approval.gate_revision
            || request.approval.delivery.is_some()
        {
            bail!("the work permit does not match its approved authorization epoch");
        }
        let hash = request
            .approval
            .field_hash
            .as_ref()
            .ok_or_else(|| anyhow!("the work approval has no field hash"))?;
        let delivery_review = if request.delivery {
            let review = request
                .delivery_review
                .as_ref()
                .ok_or_else(|| anyhow!("the delivery permit requires a host diff review"))?;
            review.validate()?;
            let range = review
                .delivery
                .as_ref()
                .ok_or_else(|| anyhow!("the delivery review has no frozen commit range"))?;
            if review.work_order_id.as_deref() != Some(request.work_order_id.as_str())
                || review.authorization_epoch != request.authorization_epoch
                || review.policy_version != request.approval.policy_version
                || review.field_hash != request.approval.field_hash
                || review.gate_revision != request.gate_revision
                || range.source_commit_sha != request.source_sha
            {
                bail!("the delivery review does not match the work permit");
            }
            source::verify_delivery_context(
                self,
                workspace,
                &request.work_order_id,
                &range.base_commit_sha,
                &range.source_commit_sha,
            )?;
            Some(PlanningWorkDeliveryReview {
                base_commit_sha: range.base_commit_sha.clone(),
                source_commit_sha: range.source_commit_sha.clone(),
                gate_revision: review.gate_revision,
                authority: Some(delivery_authority::validate(request, review)?),
            })
        } else {
            if request.delivery_review.is_some() || request.delivery_authority.is_some() {
                bail!("dispatch cannot consume a delivery review or authority");
            }
            None
        };
        let basis = PlanningWorkPermitBasis {
            authorization_epoch: request.authorization_epoch,
            policy_version: request.approval.policy_version.clone(),
            field_hash: format!("v{}:{}", hash.version, hash.sha256),
            gate_revision: request.gate_revision,
            source_sha: request.source_sha.clone(),
            delivery_review,
        };
        let auto_delivery = basis.delivery_review.as_ref().is_some_and(|review| {
            matches!(
                review.authority,
                Some(PlanningWorkDeliveryAuthority::Auto { .. })
            )
        });
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
                    requires_human_review: *stage == PlanningWorkPermitStage::Integrate
                        && !auto_delivery,
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
