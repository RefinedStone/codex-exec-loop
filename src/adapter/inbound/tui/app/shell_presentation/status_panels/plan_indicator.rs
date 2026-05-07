use ratatui::style::Style;
use ratatui::text::Span;

use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};

use super::super::{AkraTheme, ConversationState, NativeTuiApp};

// Compact view model for status/footer surfaces that need planning state but should not know the runtime snapshot shape.
#[derive(Clone, Copy)]
pub(in super::super) struct PlanModeIndicatorView {
    // Primary label tracks workspace lifecycle, not queue activity, so the footer has one stable visual anchor.
    primary_label: &'static str,
    // Detail label carries volatile runtime substate such as pause or actionable queue head.
    detail_label: Option<&'static str>,
    // Only the primary label receives color; detail text stays neutral so footer rows remain scannable.
    style: Style,
}

// Select the freshest planning runtime snapshot available for the current shell phase and project it into footer copy.
pub(super) fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
    match &app.conversation_state {
        // Ready conversations own the runtime snapshot updated by turn execution, keeping footer copy aligned with auto-follow decisions.
        ConversationState::Ready(conversation) => {
            plan_mode_indicator_from_snapshot(&conversation.planning_runtime_snapshot)
        }
        // Startup/loading surfaces lack a conversation cache, so reload from the current workspace to avoid a blank indicator.
        ConversationState::Loading | ConversationState::Failed(_) => {
            let workspace_directory = app.current_workspace_directory();
            let snapshot = app.load_planning_runtime_snapshot(&workspace_directory);
            plan_mode_indicator_from_snapshot(&snapshot)
        }
    }
}

// Derive the execution-level substate that sits beside the broader workspace lifecycle label.
pub(super) fn plan_runtime_substate_label(snapshot: &PlanningRuntimeSnapshot) -> &'static str {
    if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
        "invalid"
    // A pause reason suppresses automatic continuation even when queue work exists, so it outranks queue readiness.
    } else if snapshot.auto_follow_pause_reason().is_some() {
        "paused"
    } else if snapshot.has_actionable_queue_head() {
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
fn plan_mode_indicator_from_snapshot(snapshot: &PlanningRuntimeSnapshot) -> PlanModeIndicatorView {
    PlanModeIndicatorView {
        // Task presence is a detail concern; both ready workspace variants keep the same primary label.
        primary_label: match snapshot.workspace_status() {
            PlanningRuntimeWorkspaceStatus::Uninitialized => "Plan setup",
            PlanningRuntimeWorkspaceStatus::Invalid => "Plan invalid",
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
            | PlanningRuntimeWorkspaceStatus::ReadyWithTask => "Plan ready",
        },
        // Always include detail so repeated footer scans expose pause and queue readiness without opening the planning popup.
        detail_label: Some(plan_runtime_substate_label(snapshot)),
        // Reserve danger for invalid workspace state; pause and idle are operational states rather than hard failures.
        style: if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
            AkraTheme::danger()
        } else {
            AkraTheme::accent()
        },
    }
}
