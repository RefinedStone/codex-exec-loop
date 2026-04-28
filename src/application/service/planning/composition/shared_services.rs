use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
use crate::domain::planning::PriorityQueueService;

use super::super::authoring::directions::PlanningDirectionsService;
use super::super::repair::reconciliation::PlanningReconciliationService;
use super::super::runtime::facade::PlanningRuntimeFacadeService;
use super::super::runtime::policy::PlanningRuntimePolicyService;
use super::super::runtime::prompt::PlanningPromptService;
use super::super::runtime::validation::PlanningValidationService;
use super::PlanningFeaturePorts;

pub(super) struct PlanningSharedServices {
    pub(super) validation: PlanningValidationService,
    pub(super) priority_queue: PriorityQueueService,
    pub(super) directions: PlanningDirectionsService,
    pub(super) prompt: PlanningPromptService,
    pub(super) runtime_facade: PlanningRuntimeFacadeService,
}

impl PlanningSharedServices {
    pub(super) fn new(ports: &PlanningFeaturePorts) -> Self {
        let validation = PlanningValidationService::new();
        let priority_queue = PriorityQueueService::new();
        let directions =
            PlanningDirectionsService::new(ports.workspace.clone(), validation.clone());
        let prompt = PlanningPromptService::with_task_repository(
            ports.workspace.clone(),
            validation.clone(),
            priority_queue.clone(),
            ports.task_repository.clone(),
        );
        let reconciliation = PlanningReconciliationService::with_task_repository(
            ports.workspace.clone(),
            validation.clone(),
            priority_queue.clone(),
            ports.task_repository.clone(),
        );
        let runtime_facade = PlanningRuntimeFacadeService::new(
            prompt.clone(),
            reconciliation,
            PlanningRuntimePolicyService::new(),
            TurnPromptAssemblyService::new(),
        );

        Self {
            validation,
            priority_queue,
            directions,
            prompt,
            runtime_facade,
        }
    }
}
