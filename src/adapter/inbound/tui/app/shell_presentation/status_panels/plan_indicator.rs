use ratatui::style::Style;
use ratatui::text::Span;

use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningRuntimeProjection, PlanningRuntimeWorkspaceStatus,
};

use super::super::{AkraTheme, ConversationState, NativeTuiApp};

// Compact view model for status/footer surfaces that need planning state but should not know the runtime projection shape.
#[derive(Clone, Copy)]
pub(in super::super) struct PlanModeIndicatorView {
    // Primary label tracks workspace lifecycle, not queue activity, so the footer has one stable visual anchor.
    primary_label: &'static str,
    // Detail label carries volatile runtime substate such as pause or actionable queue head.
    detail_label: Option<&'static str>,
    // Only the primary label receives color; detail text stays neutral so footer rows remain scannable.
    style: Style,
}

// Select the planning runtime projection for the current shell phase and project it into footer copy.
pub(super) fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
    match &app.conversation_state {
        // Ready conversations own the runtime projection updated by turn execution, keeping footer copy aligned with auto-follow decisions.
        ConversationState::Ready(conversation) => {
            plan_mode_indicator_from_runtime_projection(&conversation.planning_runtime_projection)
        }
        // Startup/loading surfaces lack a conversation cache, so read the headless core projection instead of doing render-path IO.
        ConversationState::Loading | ConversationState::Failed(_) => {
            let runtime_projection = app.planning_runtime_projection_snapshot();
            plan_mode_indicator_from_runtime_projection(&runtime_projection)
        }
    }
}

// Derive the execution-level substate that sits beside the broader workspace lifecycle label.
pub(super) fn plan_runtime_substate_label(
    runtime_projection: &PlanningRuntimeProjection,
) -> &'static str {
    let application_projection =
        PlanningApplicationProjection::from_runtime_projection(runtime_projection);
    plan_runtime_substate_label_from_projection(&application_projection)
}

fn plan_runtime_substate_label_from_projection(
    projection: &PlanningApplicationProjection,
) -> &'static str {
    if projection.workspace_status == PlanningRuntimeWorkspaceStatus::Invalid {
        "invalid"
    // A pause reason suppresses automatic continuation even when queue work exists, so it outranks queue readiness.
    } else if projection.auto_follow_paused {
        "paused"
    } else if projection.workspace_status == PlanningRuntimeWorkspaceStatus::ReadyWithTask {
        "ready"
    } else {
        "idle"
    }
}

// Append planning state to an existing footer line without letting indicator styling bleed into the leading copy.
pub(super) fn plan_mode_prefixed_spans(
    leading_text: String,
    indicator: PlanModeIndicatorView,
) -> Vec<Span<'static>> {
    // Separator is its own neutral span so ratatui does not inherit the indicator style across the boundary.
    let mut spans = vec![Span::raw(leading_text), Span::raw("  |  ")];
    spans.push(Span::styled(indicator.primary_label, indicator.style));
    if let Some(detail_label) = indicator.detail_label {
        spans.push(Span::raw(format!(" / {detail_label}")));
    }
    spans
}

// Central mapping from application planning runtime to TUI vocabulary, shared by footer and tail surfaces.
fn plan_mode_indicator_from_runtime_projection(
    runtime_projection: &PlanningRuntimeProjection,
) -> PlanModeIndicatorView {
    let application_projection =
        PlanningApplicationProjection::from_runtime_projection(runtime_projection);
    plan_mode_indicator_from_projection(&application_projection)
}

// Central mapping from the application planning projection to TUI vocabulary, shared by footer and tail surfaces.
fn plan_mode_indicator_from_projection(
    projection: &PlanningApplicationProjection,
) -> PlanModeIndicatorView {
    PlanModeIndicatorView {
        // Task presence is a detail concern; both ready workspace variants keep the same primary label.
        primary_label: match projection.workspace_status {
            PlanningRuntimeWorkspaceStatus::Uninitialized => "Plan setup",
            PlanningRuntimeWorkspaceStatus::Invalid => "Plan invalid",
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
            | PlanningRuntimeWorkspaceStatus::ReadyWithTask => "Plan ready",
        },
        // Always include detail so repeated footer scans expose pause and queue readiness without opening the planning popup.
        detail_label: Some(plan_runtime_substate_label_from_projection(projection)),
        // Reserve danger for invalid workspace state; pause and idle are operational states rather than hard failures.
        style: if projection.workspace_status == PlanningRuntimeWorkspaceStatus::Invalid {
            AkraTheme::danger()
        } else {
            AkraTheme::accent()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::planning::{PriorityQueueTask, TaskStatus};

    #[test]
    fn plan_indicator_projects_invalid_runtime_projection_through_application_projection() {
        let runtime_projection = PlanningRuntimeProjection::invalid("planning validation failed");
        let indicator = plan_mode_indicator_from_runtime_projection(&runtime_projection);

        assert_eq!(indicator.primary_label, "Plan invalid");
        assert_eq!(indicator.detail_label, Some("invalid"));
    }

    #[test]
    fn plan_indicator_projection_keeps_pause_ahead_of_ready_queue() {
        let runtime_projection = PlanningRuntimeProjection::ready(
            "Planning Context".to_string(),
            "queue head ready".to_string(),
            Some(queue_task("task-1", "Ready task")),
        )
        .with_auto_follow_pause_reason("queue head repeated");
        let projection =
            PlanningApplicationProjection::from_runtime_projection(&runtime_projection);

        assert_eq!(
            plan_runtime_substate_label_from_projection(&projection),
            "paused"
        );
        let indicator = plan_mode_indicator_from_projection(&projection);
        assert_eq!(indicator.primary_label, "Plan ready");
        assert_eq!(indicator.detail_label, Some("paused"));
    }

    #[test]
    fn plan_indicator_projection_distinguishes_ready_and_idle_queue_state() {
        let ready = PlanningApplicationProjection::from_runtime_projection(
            &PlanningRuntimeProjection::ready(
                "Planning Context".to_string(),
                "queue head ready".to_string(),
                Some(queue_task("task-1", "Ready task")),
            ),
        );
        let idle = PlanningApplicationProjection::from_runtime_projection(
            &PlanningRuntimeProjection::ready(
                "Planning Context".to_string(),
                "no executable tasks".to_string(),
                None,
            ),
        );

        assert_eq!(plan_runtime_substate_label_from_projection(&ready), "ready");
        assert_eq!(plan_runtime_substate_label_from_projection(&idle), "idle");
    }

    fn queue_task(task_id: &str, task_title: &str) -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: task_id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_title: "Direction A".to_string(),
            task_title: task_title.to_string(),
            status: TaskStatus::Ready,
            combined_priority: 90,
            updated_at: "2026-05-08T00:00:00Z".to_string(),
            rank_reasons: vec!["priority".to_string()],
        }
    }
}
