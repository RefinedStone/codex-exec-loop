#[cfg(test)]
use ratatui::text::Line;

use crate::application::service::planning::PlanningRuntimeSnapshot;

#[cfg(test)]
use super::ConversationViewModel;
use super::NativeTuiApp;
#[cfg(test)]
use super::ShellCorePresentationContext;

#[cfg(test)]
#[path = "status_panels/footer_copy.rs"]
mod footer_copy;
#[path = "status_panels/live_status_layout.rs"]
mod live_status_layout;
#[path = "status_panels/plan_indicator.rs"]
mod plan_indicator;
#[path = "status_panels/tail_copy.rs"]
mod tail_copy;
#[path = "status_panels/tail_shared.rs"]
mod tail_shared;

pub(in super::super) use live_status_layout::InlineTailView;
pub(super) use plan_indicator::PlanModeIndicatorView;

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
    footer_copy::build_shell_footer_lines_with_context(
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
    tail_shared::current_live_agent_lines(conversation)
}

#[cfg(test)]
pub(super) fn parallel_mode_summary_line(app: &NativeTuiApp) -> String {
    tail_shared::parallel_mode_summary_line(app)
}

#[cfg(test)]
pub(super) fn parallel_mode_alert_line(app: &NativeTuiApp) -> Option<String> {
    tail_shared::parallel_mode_alert_line(app)
}

pub(super) fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
    plan_indicator::current_plan_mode_indicator(app)
}

pub(super) fn plan_runtime_substate_label(snapshot: &PlanningRuntimeSnapshot) -> &'static str {
    plan_indicator::plan_runtime_substate_label(snapshot)
}
