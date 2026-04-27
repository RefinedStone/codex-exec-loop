use anyhow::Result;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::shared::contract::TASK_LEDGER_FILE_PATH;
use crate::domain::planning::PlanningValidationResult;

use super::protected_restore::ReconciledPlanningWorkspaceFiles;
use super::reconciliation::{
    PlanningExecutionSnapshot, PlanningRepairRequest, execution_snapshot_to_workspace_record,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueProjectionAction {
    RebuiltFromAcceptedPlanning,
    RestoredFromExecutionSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct QueueProjectionRecoveryOutcome {
    pub(super) action: Option<PlanningQueueProjectionAction>,
    pub(super) notices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningLedgerRejectionOutcome {
    pub(super) rejected_archive_path: Option<String>,
    pub(super) queue_projection_action: Option<PlanningQueueProjectionAction>,
    pub(super) repair_request: PlanningRepairRequest,
    pub(super) notices: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PlanningLedgerRejectionRequest<'a> {
    pub(super) workspace_dir: &'a str,
    pub(super) turn_id: &'a str,
    pub(super) workspace_record: &'a PlanningWorkspaceLoadRecord,
    pub(super) execution_snapshot: &'a PlanningExecutionSnapshot,
    pub(super) reconciled_workspace: &'a ReconciledPlanningWorkspaceFiles,
    pub(super) validation_result: &'a PlanningValidationResult,
}

pub(super) fn reject_invalid_task_ledger(
    planning_workspace_port: &dyn PlanningWorkspacePort,
    request: PlanningLedgerRejectionRequest<'_>,
) -> Result<PlanningLedgerRejectionOutcome> {
    let rejected_archive_path = archive_rejected_task_ledger(planning_workspace_port, request)?;
    let queue_recovery = recover_queue_projection(
        request.workspace_record.queue_snapshot_json.as_deref(),
        request.execution_snapshot,
    );
    planning_workspace_port.commit_planning_workspace_files(
        request.workspace_dir,
        &execution_snapshot_to_workspace_record(request.execution_snapshot),
    )?;

    let validation_errors = validation_error_summaries(request.validation_result);
    let failure_summary = validation_errors
        .first()
        .cloned()
        .unwrap_or_else(|| "unknown validation failure".to_string());
    let repair_request = PlanningRepairRequest {
        failure_summary: failure_summary.clone(),
        validation_errors,
        directions_toml: request.reconciled_workspace.directions_toml.clone(),
        task_ledger_schema_json: request.reconciled_workspace.task_ledger_schema_json.clone(),
        accepted_task_ledger_json: request
            .execution_snapshot
            .task_ledger_json
            .clone()
            .unwrap_or_default(),
        rejected_task_ledger_json: request.workspace_record.task_ledger_json.clone(),
        rejected_archive_path: rejected_archive_path.clone(),
    };

    let mut notices = queue_recovery.notices;
    notices.push(format!(
        "planning reconciliation rejected task-ledger.json and restored the last accepted ledger ({failure_summary})"
    ));
    if let Some(rejected_archive_path) = rejected_archive_path.as_deref() {
        notices.push(format!(
            "planning reconciliation archived rejected task-ledger at {rejected_archive_path}"
        ));
    }

    Ok(PlanningLedgerRejectionOutcome {
        rejected_archive_path,
        queue_projection_action: queue_recovery.action,
        repair_request,
        notices,
    })
}

pub(super) fn recover_queue_projection(
    current_queue_snapshot_json: Option<&str>,
    execution_snapshot: &PlanningExecutionSnapshot,
) -> QueueProjectionRecoveryOutcome {
    if current_queue_snapshot_json == execution_snapshot.queue_snapshot_json.as_deref() {
        return QueueProjectionRecoveryOutcome::default();
    }

    QueueProjectionRecoveryOutcome {
        action: Some(PlanningQueueProjectionAction::RestoredFromExecutionSnapshot),
        notices: vec![
            "planning reconciliation restored queue.snapshot.json to the last accepted state"
                .to_string(),
        ],
    }
}

fn archive_rejected_task_ledger(
    planning_workspace_port: &dyn PlanningWorkspacePort,
    request: PlanningLedgerRejectionRequest<'_>,
) -> Result<Option<String>> {
    let Some(task_ledger_json) = request.workspace_record.task_ledger_json.as_deref() else {
        return Ok(None);
    };

    planning_workspace_port
        .archive_rejected_planning_file(
            request.workspace_dir,
            request.turn_id,
            TASK_LEDGER_FILE_PATH,
            task_ledger_json,
        )
        .map(Some)
}

fn validation_error_summaries(validation_result: &PlanningValidationResult) -> Vec<String> {
    validation_result
        .report
        .errors()
        .into_iter()
        .map(|issue| issue.message.clone())
        .collect()
}
