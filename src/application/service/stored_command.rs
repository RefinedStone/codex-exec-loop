//! 저장 명령을 해당 로컬 서비스로 실행한다. port는 `StoredCommandPort`다.

use anyhow::Result;
use std::sync::Arc;

use crate::application::port::inbound::issue_intervention_port::IssueInterventionPort;
use crate::application::port::inbound::stored_command_port::issue_confirmation::IssueConfirmationError;
use crate::application::port::inbound::stored_command_port::{
    StoredCommand, StoredCommandContext, StoredCommandOutcome, StoredCommandPort,
    StoredCommandRejection, StoredCommandResult, StoredTaskFields,
};
use crate::application::service::issue_collection::IssueCollectionService;
use crate::application::service::issue_decision::IssueDecisionService;
use crate::application::service::parallel_agent_profile::ParallelAgentProfileService;
use crate::application::service::planning::PlanningAdminFacadeService;
use crate::application::service::planning::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskMutationCommand, PlanningTaskUpdateInput,
};
use crate::application::service::task_evidence::TaskEvidenceService;
use crate::application::service::task_verification::TaskVerificationService;
use crate::domain::kernel_spec::ReasonCode;
use crate::domain::planning::PlanningMutationRefusal;

/*
 * 실행기의 구현이다. 사용자의 planning 변경을 맡는 admin facade 를 쓴다. 이 facade 는 커널의 조립으로 만든 것이어야
 * 한다 (직접 실행). 입구의 조립으로 만든 facade 는 변경을 커널 명령으로 보내므로, 그것을 쓰면 명령이 자기에게 돌아온다.
 */
pub struct StoredCommandService {
    planning: PlanningAdminFacadeService,
    issues: Option<Arc<IssueCollectionService>>,
    decisions: Option<Arc<IssueDecisionService>>,
    interventions: Option<Arc<dyn IssueInterventionPort>>,
    task_evidence: Option<Arc<TaskEvidenceService>>,
    task_verification: Option<Arc<TaskVerificationService>>,
    binding_pause: Option<Arc<super::issue_binding_pause::IssueBindingPauseService>>,
    parallel_agent_profiles: Option<ParallelAgentProfileService>,
}

impl StoredCommandService {
    pub fn new(planning: PlanningAdminFacadeService) -> Self {
        Self {
            planning,
            issues: None,
            decisions: None,
            interventions: None,
            task_evidence: None,
            task_verification: None,
            binding_pause: None,
            parallel_agent_profiles: None,
        }
    }

    pub fn with_parallel_agent_profiles(mut self, profiles: ParallelAgentProfileService) -> Self {
        self.parallel_agent_profiles = Some(profiles);
        self
    }

    pub fn with_binding_pause(
        mut self,
        service: Arc<super::issue_binding_pause::IssueBindingPauseService>,
    ) -> Self {
        self.binding_pause = Some(service);
        self
    }

    pub fn with_issue_collection(mut self, issues: Arc<IssueCollectionService>) -> Self {
        self.issues = Some(issues);
        self
    }

    pub fn with_issue_decisions(mut self, decisions: Arc<IssueDecisionService>) -> Self {
        self.decisions = Some(decisions);
        self
    }

    pub fn with_task_evidence(mut self, evidence: Arc<TaskEvidenceService>) -> Self {
        self.task_evidence = Some(evidence);
        self
    }

    pub fn with_task_verification(mut self, verification: Arc<TaskVerificationService>) -> Self {
        self.task_verification = Some(verification);
        self
    }

    pub fn with_issue_interventions(
        mut self,
        interventions: Arc<dyn IssueInterventionPort>,
    ) -> Self {
        self.interventions = Some(interventions);
        self
    }

    fn run(
        &self,
        command: StoredCommand,
        context: Option<&StoredCommandContext>,
    ) -> Result<StoredCommandResult> {
        match command {
            StoredCommand::RetryIssueAccountQuota {
                lane,
                expected_revision,
            } => {
                let coordinator = self
                    .issues
                    .as_ref()
                    .and_then(|issues| issues.coordinator_service())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "account quota retry requires the running repository kernel"
                        )
                    })?;
                Ok(StoredCommandResult::IssueAccountQuotaRetryRequested {
                    accepted: coordinator.retry_account_quota(lane, expected_revision)?,
                })
            }
            StoredCommand::ApplyIssueCommentRequest { key } => {
                let context = context.ok_or(IssueConfirmationError::Required)?;
                let receipt = self
                    .issues
                    .as_ref()
                    .and_then(|issues| issues.comment_command_service())
                    .ok_or_else(|| anyhow::anyhow!("comment requests are unavailable"))?
                    .confirm_request(&key, context)?;
                Ok(StoredCommandResult::IssueIntervened {
                    receipt: Box::new(receipt),
                })
            }
            StoredCommand::SetIssueBindingPaused {
                command_id,
                binding_id,
                paused,
                expected_revision,
            } => {
                let binding_id =
                    crate::domain::issue_automation::IssueAutomationBindingId::new(binding_id)
                        .map_err(anyhow::Error::msg)?;
                let command = self
                    .binding_pause
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!("binding pause requires the running repository kernel")
                    })?
                    .set(
                        command_id,
                        binding_id,
                        paused,
                        expected_revision,
                        context.and_then(|value| value.human_confirmation.as_ref()),
                    )?;
                Ok(StoredCommandResult::IssueBindingPauseRequested {
                    command: Box::new(command),
                })
            }
            StoredCommand::RequestTaskVerification { request } => {
                let context = context
                    .ok_or_else(|| anyhow::anyhow!("host verification requires server context"))?;
                let result = self
                    .task_verification
                    .as_ref()
                    .ok_or(crate::domain::planning::evidence::TaskEvidenceError::Unavailable)?
                    .request_verification(request, context)?;
                Ok(StoredCommandResult::TaskVerificationRequested {
                    attempt_id: result.stored.attempt.attempt_id,
                    changed: result.changed,
                })
            }
            StoredCommand::PrepareEvidence { request } => {
                let context = context
                    .ok_or_else(|| anyhow::anyhow!("task evidence requires server context"))?;
                let preparation = self
                    .task_evidence
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("task evidence is unavailable"))?
                    .prepare_evidence(request, context)?;
                Ok(StoredCommandResult::EvidencePrepared {
                    preparation: Box::new(preparation),
                })
            }
            StoredCommand::RecordEvidence { request } => {
                let context = context
                    .ok_or_else(|| anyhow::anyhow!("task evidence requires server context"))?;
                let result = self
                    .task_evidence
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("task evidence is unavailable"))?
                    .record_evidence(request, context)?;
                Ok(StoredCommandResult::CriterionEvidenceRecorded {
                    receipt: Box::new(result.value),
                    changed: result.changed,
                })
            }
            StoredCommand::WaiveCriterion { request } => {
                let context = context.ok_or(IssueConfirmationError::Required)?;
                let result = self
                    .task_evidence
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("task evidence is unavailable"))?
                    .waive_criterion(request, context)?;
                Ok(StoredCommandResult::CriterionWaived {
                    receipt: Box::new(result.value),
                    changed: result.changed,
                })
            }
            StoredCommand::InterveneIssue { request } => {
                let context = context
                    .ok_or_else(|| anyhow::anyhow!("issue interventions require server context"))?;
                let receipt = self
                    .interventions
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("issue interventions are unavailable"))?
                    .intervene(request, context)?;
                Ok(StoredCommandResult::IssueIntervened {
                    receipt: Box::new(receipt),
                })
            }
            StoredCommand::AnswerIssueDecision {
                criteria,
                decision_id,
                option,
                note,
            } => {
                let context = context.ok_or_else(|| {
                    anyhow::anyhow!("issue decision answers require server context")
                })?;
                let answer = self
                    .decisions
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("issue decisions are unavailable"))?
                    .answer_with_criteria(
                        &decision_id,
                        &option,
                        note.as_deref(),
                        criteria.as_deref(),
                        context,
                    )?;
                Ok(StoredCommandResult::IssueDecisionResolved {
                    fact: Box::new(answer.commit.record.request.fact()),
                    outcome: answer.outcome,
                    changed: answer.commit.changed,
                    catalog_revision: answer.commit.catalog_revision,
                })
            }
            StoredCommand::AskIssueDecision { request } => {
                let context = context
                    .ok_or_else(|| anyhow::anyhow!("agent questions require server context"))?;
                let commit = self
                    .decisions
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("issue decisions are unavailable"))?
                    .ask(request, context)?;
                Ok(StoredCommandResult::IssueDecisionRequested {
                    fact: Box::new(commit.record.request.fact()),
                    changed: commit.changed,
                    catalog_revision: commit.catalog_revision,
                })
            }
            StoredCommand::SetIssueAutomationMode { mode } => {
                let change = self
                    .issues
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("issue collection is unavailable"))?
                    .set_mode_with_confirmation(
                        mode,
                        context.and_then(|value| value.human_confirmation.as_ref()),
                    )?;
                Ok(StoredCommandResult::IssueAutomationModeChanged {
                    previous: change.previous,
                    projection: Box::new(change.projection),
                    changed: change.changed,
                })
            }
            StoredCommand::SetIssueAutonomy { command_id, level } => {
                let service = self
                    .issues
                    .as_ref()
                    .and_then(|issues| issues.autonomy_service())
                    .ok_or_else(|| anyhow::anyhow!("issue autonomy is unavailable"))?;
                let change = service.set(
                    command_id,
                    level,
                    context.and_then(|value| value.human_confirmation.as_ref()),
                )?;
                Ok(StoredCommandResult::IssueAutonomyChanged {
                    command_id: change.command_id,
                    changed: change.changed,
                    autonomy: Box::new(change.autonomy),
                })
            }
            StoredCommand::CreateTask { task } => {
                let change = self.planning.apply_operator_task_command(
                    PlanningTaskMutationCommand::CreateTask(create_input(task)),
                )?;
                Ok(StoredCommandResult::TaskCreated {
                    task_id: change.task_id,
                    planning_revision: change.planning_revision,
                })
            }
            StoredCommand::UpdateTask {
                task_id,
                task,
                expected_planning_revision,
            } => {
                let change = self.planning.apply_operator_task_command(
                    PlanningTaskMutationCommand::UpdateTask(update_input(
                        task_id,
                        task,
                        expected_planning_revision,
                    )),
                )?;
                Ok(StoredCommandResult::TaskUpdated {
                    task_id: change.task_id,
                    planning_revision: change.planning_revision,
                })
            }
            StoredCommand::DeleteTask {
                task_id,
                expected_planning_revision,
            } => {
                let planning_revision = self
                    .planning
                    .delete_operator_task(&task_id, Some(expected_planning_revision))?;
                Ok(StoredCommandResult::TaskDeleted {
                    task_id: task_id.trim().to_string(),
                    planning_revision,
                })
            }
            StoredCommand::CreateDirection { direction } => {
                let change = self.planning.create_operator_direction(direction)?;
                Ok(StoredCommandResult::DirectionCreated {
                    direction_id: change.direction_id,
                    planning_revision: change.planning_revision,
                })
            }
            StoredCommand::UpdateDirection {
                direction_id,
                direction,
                expected_planning_revision,
            } => {
                let change = self.planning.update_operator_direction(
                    &direction_id,
                    direction,
                    expected_planning_revision,
                )?;
                Ok(StoredCommandResult::DirectionUpdated {
                    direction_id: change.direction_id,
                    planning_revision: change.planning_revision,
                })
            }
            StoredCommand::DeleteDirection {
                direction_id,
                expected_planning_revision,
            } => {
                let deletion = self
                    .planning
                    .delete_operator_direction(&direction_id, expected_planning_revision)?;
                Ok(StoredCommandResult::DirectionDeleted {
                    direction_id: deletion.direction_id,
                    removed_task_count: deletion.removed_task_count,
                    default_retained: deletion.default_retained,
                    planning_revision: deletion.planning_revision,
                })
            }
            // 파일은 이 실행기의 작업 공간(커널의 대표 checkout)에 있다. 명령을 보낸 쪽의 폴더가 아니다.
            StoredCommand::ExportPlanningFiles => Ok(StoredCommandResult::PlanningFilesExported {
                directory: self.planning.workspace_dir().to_string(),
                paths: self.planning.export_operator_planning_files()?,
            }),
            StoredCommand::ApplyPlanningFiles => {
                let (paths, planning_revision) = self.planning.apply_operator_planning_files()?;
                Ok(StoredCommandResult::PlanningFilesApplied {
                    directory: self.planning.workspace_dir().to_string(),
                    paths,
                    planning_revision,
                })
            }
            StoredCommand::PromoteDraft {
                draft_name,
                kind,
                direction_id,
                expected_planning_revision,
            } => {
                let promotion = self.planning.promote_operator_draft(
                    &draft_name,
                    kind,
                    direction_id.as_deref(),
                    expected_planning_revision,
                )?;
                Ok(StoredCommandResult::DraftPromoted {
                    promoted_file_count: promotion.promoted_file_count,
                    is_valid: promotion.validation_report.is_valid(),
                    planning_revision: promotion.committed_planning_revision,
                })
            }
            // 프로필도 이 실행기의 작업 공간(커널의 대표 checkout)에 쓴다. 커널이 배분할 때 그 파일을 읽는다.
            StoredCommand::SaveParallelAgentProfiles { config } => {
                let directory = self.planning.workspace_dir().to_string();
                self.parallel_agent_profiles
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("parallel agent profiles are unavailable"))?
                    .save_config(&directory, &config)
                    .map_err(anyhow::Error::msg)?;
                Ok(StoredCommandResult::ParallelAgentProfilesSaved { directory })
            }
        }
    }
}

impl StoredCommandService {
    fn execute_internal(
        &self,
        command: StoredCommand,
        context: Option<&StoredCommandContext>,
    ) -> Result<StoredCommandOutcome> {
        match self.run(command, context) {
            Ok(result) => Ok(StoredCommandOutcome::Done(result)),
            // 낡은 revision 과 잡힌 guard 는 실패가 아니라 거절이다. 아무것도 바뀌지 않았다.
            Err(error) => {
                if let Some(rejection) = error.downcast_ref::<StoredCommandRejection>() {
                    return Ok(StoredCommandOutcome::Rejected(rejection.clone()));
                }
                if error.downcast_ref::<crate::domain::planning::evidence::TaskEvidenceError>()
                    == Some(&crate::domain::planning::evidence::TaskEvidenceError::HumanConfirmationRequired)
                {
                    return Ok(StoredCommandOutcome::Rejected(StoredCommandRejection {
                        reason: ReasonCode::HumanConfirmationRequired,
                        message: IssueConfirmationError::Required.to_string(),
                        planning_revision: None,
                    }));
                }
                if let Some(reason) = error.downcast_ref::<IssueConfirmationError>() {
                    return Ok(StoredCommandOutcome::Rejected(StoredCommandRejection {
                        reason: ReasonCode::HumanConfirmationRequired,
                        message: reason.to_string(),
                        planning_revision: None,
                    }));
                }
                match PlanningMutationRefusal::find_in(&error) {
                    Some(refusal) => Ok(StoredCommandOutcome::Rejected(
                        StoredCommandRejection::from(refusal),
                    )),
                    None => Err(error),
                }
            }
        }
    }
}

impl StoredCommandPort for StoredCommandService {
    fn execute(&self, command: StoredCommand) -> Result<StoredCommandOutcome> {
        self.execute_internal(command, None)
    }

    fn execute_with_context(
        &self,
        command: StoredCommand,
        context: &StoredCommandContext,
    ) -> Result<StoredCommandOutcome> {
        self.execute_internal(command, Some(context))
    }
}

fn create_input(task: StoredTaskFields) -> PlanningTaskCreateInput {
    PlanningTaskCreateInput {
        direction_id: task.direction_id,
        direction_relation_note: None,
        title: task.title,
        description: task.description,
        acceptance: task.acceptance,
        status: task.status,
        base_priority: task.base_priority,
        dynamic_priority_delta: task.dynamic_priority_delta,
        priority_reason: task.priority_reason,
        depends_on: task.depends_on,
        blocked_by: task.blocked_by,
    }
}

/*
 * 고치기는 작업의 필드 전체를 바꾼다 (admin 의 폼과 같은 뜻이다). 그래서 이유와 참조는 값이 없어도 "비운다"로
 * 보낸다. 방향, 설명, 상태, 우선순위, 완료 기준은 값이 없으면 그대로 둔다. 서비스의 고치기 입력이 그 필드들에서 "없음"을
 * "그대로"로 읽기 때문이다.
 */
fn update_input(
    task_id: String,
    task: StoredTaskFields,
    expected_planning_revision: i64,
) -> PlanningTaskUpdateInput {
    PlanningTaskUpdateInput {
        expected_planning_revision: Some(expected_planning_revision),
        task_id,
        direction_id: task.direction_id,
        direction_relation_note: None,
        title: Some(task.title),
        description: task.description,
        acceptance: task.acceptance,
        status: task.status,
        base_priority: task.base_priority,
        dynamic_priority_delta: task.dynamic_priority_delta,
        priority_reason: Some(task.priority_reason.unwrap_or_default()),
        depends_on: Some(task.depends_on),
        blocked_by: Some(task.blocked_by),
    }
}

#[cfg(test)]
mod tests {
    use super::{create_input, update_input};
    use crate::application::port::inbound::stored_command_port::StoredTaskFields;
    use crate::domain::planning::{TaskAcceptanceChannel, TaskAcceptanceCriterion};

    #[test]
    fn stored_task_acceptance_preserves_patch_presence_and_update_revision() {
        for acceptance in [
            None,
            Some(Vec::new()),
            Some(vec![TaskAcceptanceCriterion {
                id: "observable-result".into(),
                scenario: "The expected result is observable.".into(),
                channel: TaskAcceptanceChannel::Command,
                essential: true,
                verification_recipe: None,
            }]),
        ] {
            let fields = StoredTaskFields {
                direction_id: None,
                title: "Task".into(),
                description: None,
                acceptance: acceptance.clone(),
                status: None,
                base_priority: None,
                dynamic_priority_delta: None,
                priority_reason: None,
                depends_on: Vec::new(),
                blocked_by: Vec::new(),
            };
            assert_eq!(create_input(fields.clone()).acceptance, acceptance);
            let update = update_input("task-1".into(), fields, 7);
            assert_eq!(update.acceptance, acceptance);
            assert_eq!(update.expected_planning_revision, Some(7));
        }
    }
}
