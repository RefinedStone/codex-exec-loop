//! Resume a durable waiting session without ordinary queue selection or slot acquisition.
use super::super::turn::ParallelModeTurnService;
use super::super::{ParallelModeService, pool, slot_lifecycle};
use super::{
    ParallelDispatchWorkerRequest, ParallelModeOrchestratorLoopEvent,
    spawn_parallel_dispatch_worker,
};
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningWorkerContinuationClaimOutcome, PlanningWorkerContinuationDelivery,
    PlanningWorkerContinuationRequest,
};
use crate::application::service::planning::{PlanningServices, PlanningTaskHandoff};
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;

pub(super) struct ContinuationDispatchContext<'a> {
    pub workspace: &'a str,
    pub epoch_id: u64,
    pub planning: &'a PlanningServices,
    pub worker_port: &'a Arc<dyn ParallelAgentWorkerPort>,
    pub turn_service: &'a ParallelModeTurnService,
    pub event_sender: &'a Sender<ParallelModeOrchestratorLoopEvent>,
}

pub(super) fn dispatch_queued(
    service: &ParallelModeService,
    context: ContinuationDispatchContext<'_>,
) {
    let requests = match service
        .planning_authority
        .queued_worker_continuations(context.workspace)
    {
        Ok(requests) => requests,
        Err(_) => {
            let _ = context.event_sender.send(
                ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(
                    "대기 중인 답변 전달 요청을 읽지 못해 재개를 보류했습니다.".into(),
                ),
            );
            return;
        }
    };
    for request in requests {
        if !context
            .turn_service
            .automation_epoch_is_active(context.workspace, context.epoch_id)
        {
            break;
        }
        let claim = match claim_and_replace_mirror(service, context.workspace, &request) {
            Ok(PlanningWorkerContinuationClaimOutcome::Claimed(claim)) => claim,
            Ok(
                PlanningWorkerContinuationClaimOutcome::Waiting
                | PlanningWorkerContinuationClaimOutcome::Settled(_),
            ) => continue,
            Err(_) => {
                let _ = context.event_sender.send(
                    ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(
                        "답변 전달 경계를 확인하지 못해 기존 작업 공간을 보존했습니다.".into(),
                    ),
                );
                continue;
            }
        };
        let task = &claim.task;
        let worker_request = ParallelDispatchWorkerRequest {
            planning_workspace_directory: context.workspace.to_string(),
            worktree_directory: claim.lease.worktree_path.clone(),
            automation_epoch_id: context.epoch_id,
            prompt: claim.request.message.clone(),
            developer_instructions: String::new(),
            service_name: "parallel-answer".into(),
            handoff_task: PlanningTaskHandoff {
                task_id: task.id.clone(),
                task_title: task.title.clone(),
                direction_id: task.direction_id.clone(),
                combined_priority: task.base_priority + task.dynamic_priority_delta,
                updated_at: task.updated_at.clone(),
                status_label: "awaiting_user".into(),
            },
            expected_lease: claim.lease.clone(),
            continuation: Some(claim),
        };
        spawn_parallel_dispatch_worker(
            worker_request,
            context.worker_port.clone(),
            context.turn_service.clone(),
            context.planning.clone(),
            context.event_sender.clone(),
        );
    }
}

fn claim_and_replace_mirror(
    service: &ParallelModeService,
    workspace: &str,
    request: &PlanningWorkerContinuationRequest,
) -> Result<PlanningWorkerContinuationClaimOutcome, String> {
    let receipt = service
        .planning_authority
        .recheck_queued_worker_continuation(workspace, &request.command_id)
        .map_err(|_| "queued answer recheck failed".to_string())?;
    if receipt.delivery != PlanningWorkerContinuationDelivery::Queued {
        return Ok(PlanningWorkerContinuationClaimOutcome::Settled(receipt));
    }
    let lock = pool::acquire_pool_mutation_lock(
        service.planning_authority.as_ref(),
        service.parallel_runtime.as_ref(),
        workspace,
    )?;
    let resolution = pool::resolve_workspace_slot_lease_with_runtime(
        service.parallel_runtime.as_ref(),
        service.planning_authority.as_ref(),
        &request.question.worktree_path,
    )?
    .ok_or_else(|| "waiting worker lease is unavailable".to_string())?;
    lock.verify_pool_root(&resolution.context.pool_root)?;
    let relative = PathBuf::from(".leases").join(format!("{}.json", resolution.lease.slot_id));
    let observed = service
        .parallel_runtime
        .read_runtime_mirror_optional(&resolution.context.pool_root, &relative)?;
    if let Some(body) = observed.as_deref() {
        let mirror: ParallelModeSlotLeaseSnapshot = serde_json::from_str(body)
            .map_err(|_| "waiting lease mirror is invalid".to_string())?;
        if mirror != resolution.lease {
            return Err("waiting lease mirror changed".into());
        }
    }
    let generation = slot_lifecycle::new_slot_lease_generation()?;
    let outcome = service
        .planning_authority
        .claim_worker_continuation(
            workspace,
            &request.command_id,
            &generation,
            slot_lifecycle::current_lease_owner_process(service.parallel_runtime.as_ref()),
        )
        .map_err(|_| "worker continuation claim failed".to_string())?;
    if let PlanningWorkerContinuationClaimOutcome::Claimed(claim) = &outcome {
        if claim.previous_lease != resolution.lease {
            return Err("waiting lease changed before its continuation claim".into());
        }
        let body = serde_json::to_string_pretty(&claim.lease)
            .map_err(|_| "continuation lease could not be serialized".to_string())?;
        if !service
            .parallel_runtime
            .compare_and_swap_runtime_mirror_file(
                &resolution.context.pool_root,
                &relative,
                observed.as_deref(),
                Some(&body),
            )?
        {
            // The authority has sealed Unknown and a new generation. Never roll it back and
            // accidentally admit a late old-generation event or a retry of this answer.
            return Err("continuation lease mirror changed; delivery remains unknown".into());
        }
    }
    Ok(outcome)
}
