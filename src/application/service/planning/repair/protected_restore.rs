use anyhow::Result;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::shared::contract::{
    DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
};

use super::reconciliation::{PlanningChangeSet, PlanningExecutionSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProtectedFileRestoration {
    pub relative_path: &'static str,
    pub archived_candidate_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ReconciledPlanningWorkspaceFiles {
    pub(super) directions_toml: String,
    pub(super) task_ledger_schema_json: String,
    pub(super) result_output_markdown: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PlanningProtectedFileRestoreResult {
    pub(super) workspace_files: ReconciledPlanningWorkspaceFiles,
    pub(super) restorations: Vec<PlanningProtectedFileRestoration>,
    pub(super) notices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProtectedFileRestoreResult {
    body: String,
    restoration: Option<PlanningProtectedFileRestoration>,
    notices: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct ProtectedFileRestoreRequest<'a> {
    workspace_dir: &'a str,
    turn_id: &'a str,
    relative_path: &'static str,
    current_body: Option<&'a str>,
    execution_snapshot_body: Option<&'a str>,
    changed: bool,
}

pub(super) fn restore_protected_workspace_files(
    planning_workspace_port: &dyn PlanningWorkspacePort,
    workspace_dir: &str,
    turn_id: &str,
    workspace_record: &PlanningWorkspaceLoadRecord,
    execution_snapshot: &PlanningExecutionSnapshot,
    change_set: PlanningChangeSet,
) -> Result<PlanningProtectedFileRestoreResult> {
    let directions = restore_protected_file(
        planning_workspace_port,
        ProtectedFileRestoreRequest {
            workspace_dir,
            turn_id,
            relative_path: DIRECTIONS_FILE_PATH,
            current_body: workspace_record.directions_toml.as_deref(),
            execution_snapshot_body: execution_snapshot.directions_toml.as_deref(),
            changed: change_set.directions_changed,
        },
    )?;
    let task_ledger_schema = restore_protected_file(
        planning_workspace_port,
        ProtectedFileRestoreRequest {
            workspace_dir,
            turn_id,
            relative_path: TASK_LEDGER_SCHEMA_FILE_PATH,
            current_body: workspace_record.task_ledger_schema_json.as_deref(),
            execution_snapshot_body: execution_snapshot.task_ledger_schema_json.as_deref(),
            changed: change_set.task_ledger_schema_changed,
        },
    )?;
    let result_output = restore_protected_file(
        planning_workspace_port,
        ProtectedFileRestoreRequest {
            workspace_dir,
            turn_id,
            relative_path: RESULT_OUTPUT_FILE_PATH,
            current_body: workspace_record.result_output_markdown.as_deref(),
            execution_snapshot_body: execution_snapshot.result_output_markdown.as_deref(),
            changed: change_set.result_output_changed,
        },
    )?;

    let mut result = PlanningProtectedFileRestoreResult::default();
    let directions_toml = collect_restore_side_effects(&mut result, directions);
    let task_ledger_schema_json = collect_restore_side_effects(&mut result, task_ledger_schema);
    let result_output_markdown = collect_restore_side_effects(&mut result, result_output);
    result.workspace_files = ReconciledPlanningWorkspaceFiles {
        directions_toml,
        task_ledger_schema_json,
        result_output_markdown,
    };
    Ok(result)
}

fn collect_restore_side_effects(
    result: &mut PlanningProtectedFileRestoreResult,
    file_result: ProtectedFileRestoreResult,
) -> String {
    if let Some(restoration) = file_result.restoration {
        result.restorations.push(restoration);
    }
    result.notices.extend(file_result.notices);
    file_result.body
}

fn restore_protected_file(
    planning_workspace_port: &dyn PlanningWorkspacePort,
    request: ProtectedFileRestoreRequest<'_>,
) -> Result<ProtectedFileRestoreResult> {
    if !request.changed || request.current_body == request.execution_snapshot_body {
        return Ok(ProtectedFileRestoreResult {
            body: request
                .current_body
                .or(request.execution_snapshot_body)
                .unwrap_or_default()
                .to_string(),
            restoration: None,
            notices: Vec::new(),
        });
    }

    let archived_candidate_path = archive_changed_candidate(
        planning_workspace_port,
        request.workspace_dir,
        request.turn_id,
        request.relative_path,
        request.current_body,
        request.execution_snapshot_body,
    )?;

    let restoration = PlanningProtectedFileRestoration {
        relative_path: request.relative_path,
        archived_candidate_path: archived_candidate_path.clone(),
    };
    let mut notices = vec![format!(
        "planning reconciliation restored protected {}",
        request.relative_path
    )];
    if let Some(archived_candidate_path) = archived_candidate_path.as_deref() {
        notices.push(format!(
            "planning reconciliation archived protected-file candidate at {archived_candidate_path}"
        ));
    }

    Ok(ProtectedFileRestoreResult {
        body: request
            .execution_snapshot_body
            .unwrap_or_default()
            .to_string(),
        restoration: Some(restoration),
        notices,
    })
}

fn archive_changed_candidate(
    planning_workspace_port: &dyn PlanningWorkspacePort,
    workspace_dir: &str,
    turn_id: &str,
    relative_path: &str,
    current_body: Option<&str>,
    execution_snapshot_body: Option<&str>,
) -> Result<Option<String>> {
    let Some(current_body) = current_body else {
        return Ok(None);
    };
    if Some(current_body) == execution_snapshot_body {
        return Ok(None);
    }

    planning_workspace_port
        .archive_rejected_planning_file(workspace_dir, turn_id, relative_path, current_body)
        .map(Some)
}
