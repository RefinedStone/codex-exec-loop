use std::collections::BTreeMap;

use super::{
    PlanningAdminDirectionManagementView, PlanningAdminDirectionSummaryView,
    PlanningAdminDirectionsSummaryView, PlanningAdminDoctorSummary, PlanningAdminManagementView,
    PlanningAdminQueueHeadView, PlanningAdminQueuePreview, PlanningAdminQueueTaskView,
    PlanningAdminRuntimeSummary, PlanningAdminTaskManagementView, PlanningAdminValidationIssueView,
    PlanningAdminValidationView,
};
use crate::application::service::planning::{
    DirectionsMaintenanceSummary, PlanningDoctorReport, PlanningRuntimeSnapshot,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PlanningFileKind, PlanningValidationReport,
    PlanningValidationSeverity, PriorityQueueProjection, TaskAuthorityDocument,
};

/*
 * Projection code is the adapter-like part of the admin application service:
 * domain documents and runtime snapshots enter here, and route/template shaped
 * view models leave here. Keeping this translation centralized prevents admin
 * handlers from learning domain enum labels, queue preview limits, or form
 * text-joining rules.
 */
pub(super) fn map_management_view(
    directions: &DirectionCatalogDocument,
    task_authority: &TaskAuthorityDocument,
    default_direction_id: &str,
) -> PlanningAdminManagementView {
    // Direction ids are trimmed for the count join to match queue resolution
    // behavior, while the original ids are preserved in the editable rows.
    let mut task_counts = BTreeMap::<&str, usize>::new();
    for task in &task_authority.tasks {
        *task_counts.entry(task.direction_id.trim()).or_default() += 1;
    }

    PlanningAdminManagementView {
        default_direction_id: default_direction_id.to_string(),
        // Multi-value direction fields become newline blocks because the admin
        // management form edits them as text areas, not nested JSON controls.
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
        // Task dependency lists use the same newline representation so a submit
        // can round-trip through the mutation parser without losing order.
        tasks: task_authority
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
    // Doctor reports already aggregate system health; this mapper only freezes
    // labels into the admin response contract.
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
    // Queue projection is optional during startup or broken states. The overview
    // still renders runtime status while falling back to empty queue lists.
    let queue_preview = snapshot.queue_projection().map(map_queue_preview);
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
    // Maintenance summary owns file-system checks; the admin view adds only
    // labels and needs_attention flags for route/template branching.
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
    // The report remains the validation authority. Counts are duplicated here so
    // admin clients can render badges without reclassifying issue severities.
    let error_count = report
        .issues
        .iter()
        .filter(|issue| issue.severity == PlanningValidationSeverity::Error)
        .count();
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
                // Keep string values stable for templates and any JSON clients.
                severity: match issue.severity {
                    PlanningValidationSeverity::Error => "error".to_string(),
                    PlanningValidationSeverity::Warning => "warning".to_string(),
                },
                file_kind: match issue.file_kind {
                    PlanningFileKind::Directions => "directions".to_string(),
                    PlanningFileKind::TaskAuthority => "task_authority".to_string(),
                    PlanningFileKind::ResultOutput => "result_output".to_string(),
                },
                code: issue.code.clone(),
                message: issue.message.clone(),
            })
            .collect(),
    }
}

pub(super) fn map_queue_preview(snapshot: &PriorityQueueProjection) -> PlanningAdminQueuePreview {
    // The overview needs a compact operational preview, not the full queue. Five
    // rows keeps the page scannable while the queue head carries detailed rank
    // reasons for the active handoff.
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
    // The admin form accepts these lower-case labels when mutating directions.
    match state {
        DirectionState::Active => "active",
        DirectionState::Paused => "paused",
        DirectionState::Done => "done",
    }
}
