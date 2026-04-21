use ratatui::style::{Color, Style};
#[cfg(test)]
use ratatui::text::Line;
use ratatui::text::Span;

use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};

#[cfg(test)]
use super::ConversationViewModel;
#[cfg(test)]
use super::ShellCorePresentationContext;
use super::{ConversationState, NativeTuiApp};

#[path = "status_panels/live_status_layout.rs"]
mod live_status_layout;
#[path = "status_panels/tail_copy.rs"]
mod tail_copy;

pub(in super::super) use live_status_layout::InlineTailView;

#[derive(Clone, Copy)]
pub(super) struct PlanModeIndicatorView {
    primary_label: &'static str,
    detail_label: Option<&'static str>,
    color: Color,
}

#[cfg(test)]
pub(super) fn build_shell_footer_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
    plan_mode_indicator: PlanModeIndicatorView,
    parallel_mode_summary_line: String,
    parallel_mode_alert_line: Option<String>,
    github_review_recent_changes_summary: Option<String>,
    planning_summary_line: Option<String>,
    planning_notice_line: Option<String>,
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    tail_copy::build_shell_footer_lines_with_context(
        context,
        plan_mode_indicator,
        parallel_mode_summary_line,
        parallel_mode_alert_line,
        github_review_recent_changes_summary,
        planning_summary_line,
        planning_notice_line,
        planner_panel_lines,
    )
}

pub(crate) fn build_inline_tail_view(app: &NativeTuiApp, content_width: u16) -> InlineTailView {
    live_status_layout::build_inline_tail_view(app, content_width)
}

#[cfg(test)]
pub(super) fn build_inline_tail_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_inline_tail_view(app, 0).lines
}

#[cfg(test)]
pub(super) fn current_live_agent_lines(
    conversation: &ConversationViewModel,
) -> Option<Vec<Line<'static>>> {
    tail_copy::current_live_agent_lines(conversation)
}

#[cfg(test)]
pub(super) fn parallel_mode_summary_line(app: &NativeTuiApp) -> String {
    tail_copy::parallel_mode_summary_line(app)
}

#[cfg(test)]
pub(super) fn parallel_mode_alert_line(app: &NativeTuiApp) -> Option<String> {
    tail_copy::parallel_mode_alert_line(app)
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

fn plan_mode_indicator_from_snapshot(snapshot: &PlanningRuntimeSnapshot) -> PlanModeIndicatorView {
    if !snapshot.plan_enabled() {
        return PlanModeIndicatorView {
            primary_label: "Plan off",
            detail_label: None,
            color: Color::Red,
        };
    }

    PlanModeIndicatorView {
        primary_label: "Plan on",
        detail_label: Some(plan_runtime_substate_label(snapshot)),
        color: Color::Blue,
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

fn plan_mode_prefixed_spans(
    leading_text: String,
    indicator: PlanModeIndicatorView,
) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(leading_text), Span::raw("  |  ")];
    spans.push(Span::styled(
        indicator.primary_label,
        Style::default().fg(indicator.color),
    ));
    if let Some(detail_label) = indicator.detail_label {
        spans.push(Span::raw(format!(" / {detail_label}")));
    }
    spans
}
