use std::collections::BTreeMap;

use crate::application::service::planning::{
    DirectionsMaintenanceSummary, PlanningDoctorReport, PlanningRuntimeSnapshot,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PlanningFileKind, PlanningValidationReport,
    PlanningValidationSeverity, PriorityQueueProjection, TaskLedgerDocument,
};

use super::{
    PlanningAdminDirectionManagementView, PlanningAdminDirectionSummaryView,
    PlanningAdminDirectionsSummaryView, PlanningAdminDoctorSummary, PlanningAdminManagementView,
    PlanningAdminQueueHeadView, PlanningAdminQueuePreview, PlanningAdminQueueTaskView,
    PlanningAdminRuntimeSummary, PlanningAdminTaskManagementView, PlanningAdminValidationIssueView,
    PlanningAdminValidationView,
};

pub(super) fn map_management_view(
    directions: &DirectionCatalogDocument,
    task_ledger: &TaskLedgerDocument,
    default_direction_id: &str,
) -> PlanningAdminManagementView {
    let mut task_counts = BTreeMap::<String, usize>::new();
    for task in &task_ledger.tasks {
        *task_counts
            .entry(task.direction_id.trim().to_string())
            .or_default() += 1;
    }

    PlanningAdminManagementView {
        default_direction_id: default_direction_id.to_string(),
        directions: directions
            .directions
            .iter()
            .map(|direction| PlanningAdminDirectionManagementView {
                id: direction.id.clone(),
                title: direction.title.clone(),
                summary: direction.summary.clone(),
                success_criteria_text: direction.success_criteria.join("\n"),
                scope_hints_text: direction.scope_hints.join("\n"),
                detail_doc_path: direction.detail_doc_path.clone(),
                state: direction_state_label(direction.state).to_string(),
                task_count: task_counts
                    .get(direction.id.trim())
                    .copied()
                    .unwrap_or_default(),
            })
            .collect(),
        tasks: task_ledger
            .tasks
            .iter()
            .map(|task| PlanningAdminTaskManagementView {
                id: task.id.clone(),
                direction_id: task.direction_id.clone(),
                title: task.title.clone(),
                description: task.description.clone(),
                status: task.status.label().to_string(),
                base_priority: task.base_priority,
                dynamic_priority_delta: task.dynamic_priority_delta,
                priority_reason: task.priority_reason.clone(),
                depends_on_text: task.depends_on.join("\n"),
                blocked_by_text: task.blocked_by.join("\n"),
                updated_at: task.updated_at.clone(),
            })
            .collect(),
    }
}

pub(super) fn map_doctor_report(report: &PlanningDoctorReport) -> PlanningAdminDoctorSummary {
    PlanningAdminDoctorSummary {
        planning_state: report.planning_state().label().to_string(),
        queue_idle_policy: report.queue_idle_policy().map(str::to_string),
        queue_summary: report.queue_summary().map(str::to_string),
        proposal_summary: report.proposal_summary().map(str::to_string),
        health: report.health().map(str::to_string),
        issue: report.issue().map(str::to_string),
        note: report.note().map(str::to_string),
    }
}

pub(super) fn map_runtime_snapshot(
    snapshot: &PlanningRuntimeSnapshot,
) -> PlanningAdminRuntimeSummary {
    let queue_preview = snapshot.queue_projection().cloned().map(map_queue_preview);
    PlanningAdminRuntimeSummary {
        workspace_present: snapshot.workspace_present(),
        preview_status_label: snapshot.preview_status_label().to_string(),
        preview_detail: snapshot.preview_detail().map(str::to_string),
        queue_head: queue_preview
            .as_ref()
            .and_then(|preview| preview.queue_head.clone()),
        visible_tasks: queue_preview
            .as_ref()
            .map(|preview| preview.visible_tasks.clone())
            .unwrap_or_default(),
        proposed_tasks: queue_preview
            .as_ref()
            .map(|preview| preview.proposed_tasks.clone())
            .unwrap_or_default(),
    }
}

pub(super) fn map_directions_summary(
    summary: DirectionsMaintenanceSummary,
) -> PlanningAdminDirectionsSummaryView {
    PlanningAdminDirectionsSummaryView {
        missing_detail_doc_count: summary.missing_detail_doc_count,
        broken_detail_doc_count: summary.broken_detail_doc_count,
        queue_idle_policy: summary.queue_idle_policy.label().to_string(),
        queue_idle_prompt_path: summary.queue_idle_prompt_path,
        queue_idle_prompt_status: summary.queue_idle_prompt_status.label().to_string(),
        parse_error: summary.parse_error,
        directions: summary
            .directions
            .into_iter()
            .map(|direction| PlanningAdminDirectionSummaryView {
                id: direction.id,
                title: direction.title,
                detail_doc_path: direction.detail_doc_path,
                detail_doc_status: direction.detail_doc_status.label().to_string(),
                needs_attention: direction.detail_doc_status.needs_attention(),
            })
            .collect(),
    }
}

pub(super) fn map_validation_report(
    report: &PlanningValidationReport,
) -> PlanningAdminValidationView {
    let error_count = report.errors().len();
    let warning_count = report
        .issues
        .iter()
        .filter(|issue| issue.severity != PlanningValidationSeverity::Error)
        .count();
    PlanningAdminValidationView {
        is_valid: report.is_valid(),
        error_count,
        warning_count,
        issues: report
            .issues
            .iter()
            .map(|issue| PlanningAdminValidationIssueView {
                severity: match issue.severity {
                    PlanningValidationSeverity::Error => "error".to_string(),
                    PlanningValidationSeverity::Warning => "warning".to_string(),
                },
                file_kind: match issue.file_kind {
                    PlanningFileKind::Directions => "directions".to_string(),
                    PlanningFileKind::TaskLedger => "task_ledger".to_string(),
                    PlanningFileKind::TaskLedgerSchema => "task_ledger_schema".to_string(),
                    PlanningFileKind::ResultOutput => "result_output".to_string(),
                },
                code: issue.code.clone(),
                message: issue.message.clone(),
            })
            .collect(),
    }
}

pub(super) fn map_queue_preview(snapshot: PriorityQueueProjection) -> PlanningAdminQueuePreview {
    PlanningAdminQueuePreview {
        queue_summary: match snapshot.next_task.as_ref() {
            Some(task) => format!("now: {}", task.task_title.trim()),
            None => "next task: none".to_string(),
        },
        proposal_summary: snapshot
            .proposed_tasks
            .first()
            .map(|task| task.task_title.trim().to_string()),
        queue_head: snapshot
            .next_task
            .as_ref()
            .map(|task| PlanningAdminQueueHeadView {
                task_id: task.task_id.clone(),
                task_title: task.task_title.clone(),
                direction_id: task.direction_id.clone(),
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at.clone(),
                rank_reasons: task.rank_reasons.clone(),
            }),
        visible_tasks: snapshot
            .visible_tasks(5)
            .into_iter()
            .map(|task| PlanningAdminQueueTaskView {
                task_id: task.task_id,
                task_title: task.task_title,
                direction_id: task.direction_id,
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at,
            })
            .collect(),
        proposed_tasks: snapshot
            .visible_proposed_tasks(5)
            .into_iter()
            .map(|task| PlanningAdminQueueTaskView {
                task_id: task.task_id,
                task_title: task.task_title,
                direction_id: task.direction_id,
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at,
            })
            .collect(),
    }
}

fn direction_state_label(state: DirectionState) -> &'static str {
    match state {
        DirectionState::Active => "active",
        DirectionState::Paused => "paused",
        DirectionState::Done => "done",
    }
}
