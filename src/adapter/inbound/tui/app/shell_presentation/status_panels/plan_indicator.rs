use ratatui::style::Style;
use ratatui::text::Span;

use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};

use super::super::{AkraTheme, ConversationState, NativeTuiApp};

#[derive(Clone, Copy)]
pub(in super::super) struct PlanModeIndicatorView {
    primary_label: &'static str,
    detail_label: Option<&'static str>,
    style: Style,
}

pub(super) fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
    match &app.conversation_state {
        ConversationState::Ready(conversation) => {
            plan_mode_indicator_from_snapshot(&conversation.planning_runtime_snapshot)
        }
        ConversationState::Loading | ConversationState::Failed(_) => {
            let workspace_directory = app.current_workspace_directory();
            let snapshot = app.load_planning_runtime_snapshot(&workspace_directory);
            plan_mode_indicator_from_snapshot(&snapshot)
        }
    }
}

pub(super) fn plan_runtime_substate_label(snapshot: &PlanningRuntimeSnapshot) -> &'static str {
    if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
        "invalid"
    } else if snapshot.auto_followup_pause_reason().is_some() {
        "paused"
    } else if snapshot.has_actionable_queue_head() {
        "ready"
    } else {
        "idle"
    }
}

pub(super) fn plan_mode_prefixed_spans(
    leading_text: String,
    indicator: PlanModeIndicatorView,
) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(leading_text), Span::raw("  |  ")];
    spans.push(Span::styled(indicator.primary_label, indicator.style));
    if let Some(detail_label) = indicator.detail_label {
        spans.push(Span::raw(format!(" / {detail_label}")));
    }
    spans
}

fn plan_mode_indicator_from_snapshot(snapshot: &PlanningRuntimeSnapshot) -> PlanModeIndicatorView {
    if !snapshot.plan_enabled() {
        return PlanModeIndicatorView {
            primary_label: "Plan off",
            detail_label: None,
            style: AkraTheme::danger(),
        };
    }

    PlanModeIndicatorView {
        primary_label: "Plan on",
        detail_label: Some(plan_runtime_substate_label(snapshot)),
        style: AkraTheme::accent(),
    }
}
