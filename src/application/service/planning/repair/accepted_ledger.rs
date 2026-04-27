use anyhow::{Context, Result, anyhow};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::domain::planning::PlanningValidationResult;

use super::ledger_recovery::PlanningQueueProjectionAction;
use super::reconciliation::{PlanningExecutionSnapshot, execution_snapshot_to_workspace_record};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningAcceptedLedgerCommitOutcome {
    pub(super) queue_projection_action: PlanningQueueProjectionAction,
    pub(super) notices: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PlanningAcceptedLedgerCommitRequest<'a> {
    pub(super) workspace_dir: &'a str,
    pub(super) workspace_record: &'a PlanningWorkspaceLoadRecord,
    pub(super) execution_snapshot: &'a PlanningExecutionSnapshot,
    pub(super) validation_result: &'a PlanningValidationResult,
}

pub(super) fn commit_accepted_task_ledger(
    planning_workspace_port: &dyn PlanningWorkspacePort,
    planning_task_repository_port: &dyn PlanningTaskRepositoryPort,
    priority_queue_service: &PriorityQueueService,
    request: PlanningAcceptedLedgerCommitRequest<'_>,
) -> Result<PlanningAcceptedLedgerCommitOutcome> {
    let directions = request
        .validation_result
        .directions
        .as_ref()
        .ok_or_else(|| {
            anyhow!("planning validation reported success without parsed directions.toml")
        })?;
    let task_ledger = request
        .validation_result
        .task_ledger
        .as_ref()
        .ok_or_else(|| {
            anyhow!("planning validation reported success without parsed task-ledger.json")
        })?;
    let queue_projection = priority_queue_service
        .build_projection(directions, task_ledger)
        .map_err(|error| anyhow!("planning validation passed but queue build failed: {error}"))?;
    let queue_snapshot_json = serde_json::to_string_pretty(&queue_projection)
        .context("failed to serialize queue projection")?;

    let mut committed_record = execution_snapshot_to_workspace_record(request.execution_snapshot);
    committed_record.task_ledger_json = request.workspace_record.task_ledger_json.clone();
    committed_record.queue_snapshot_json = Some(queue_snapshot_json);
    planning_workspace_port
        .commit_planning_workspace_files(request.workspace_dir, &committed_record)?;
    planning_task_repository_port.commit_task_authority_snapshot(
        request.workspace_dir,
        PlanningTaskAuthorityCommit {
            observed_planning_revision: None,
            task_ledger,
            queue_projection: &queue_projection,
        },
    )?;

    Ok(PlanningAcceptedLedgerCommitOutcome {
        queue_projection_action: PlanningQueueProjectionAction::RebuiltFromAcceptedPlanning,
        notices: vec![
            "planning reconciliation accepted task-ledger.json and rebuilt queue.snapshot.json"
                .to_string(),
        ],
    })
}
