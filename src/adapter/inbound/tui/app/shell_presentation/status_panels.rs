// Status panel facade keeps footer/tail copy modules private while exposing the few stable projection helpers
// needed by shell rendering, overlays, and contract tests.
use ratatui::text::Line;

use crate::application::service::planning::PlanningRuntimeSnapshot;

use super::ConversationViewModel;
use super::NativeTuiApp;
#[cfg(test)]
use super::ShellCorePresentationContext;

#[cfg(test)]
#[path = "status_panels/footer_copy.rs"]
mod footer_copy;
// Width-aware inline tail layout owns cursor placement and top-anchored startup behavior.
#[path = "status_panels/live_status_layout.rs"]
mod live_status_layout;
// Planning indicator vocabulary is centralized here so footer and workspace popup copy do not diverge.
#[path = "status_panels/plan_indicator.rs"]
mod plan_indicator;
// Tail copy builds the textual ribbon; live_status_layout decides how that ribbon occupies terminal rows.
#[path = "status_panels/tail_copy.rs"]
mod tail_copy;
// Shared status helpers stay behind this facade to keep overlay modules from depending on tail internals.
#[path = "status_panels/tail_shared.rs"]
mod tail_shared;

// Rendering needs the full layout DTO; callers outside the TUI adapter should never see this presentation type.
pub(in super::super) use live_status_layout::InlineTailView;
#[cfg(test)]
pub(super) use plan_indicator::PlanModeIndicatorView;

#[cfg(test)]
// Footer copy deliberately composes many independent status slices; contract tests need this exact join point.
#[allow(clippy::too_many_arguments)]
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

// Production entrypoint for the inline bottom region: text rows plus cursor/layout metadata.
pub(crate) fn build_inline_tail_view(app: &NativeTuiApp, content_width: u16) -> InlineTailView {
    live_status_layout::build_inline_tail_view(app, content_width)
}

#[cfg(test)]
// Test adapter for older line-only assertions; production keeps cursor metadata through InlineTailView.
pub(super) fn build_inline_tail_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_inline_tail_view(app, 0).lines
}

// Shared live-agent projection used by main tail and overlays so streaming/tool activity speaks with one vocabulary.
pub(super) fn current_live_agent_lines(
    conversation: &ConversationViewModel,
) -> Option<Vec<Line<'static>>> {
    tail_shared::current_live_agent_lines(conversation)
}

#[cfg(test)]
// Test-only exposure keeps overlay contract tests independent of tail_shared's file layout.
pub(super) fn parallel_mode_summary_line(app: &NativeTuiApp) -> String {
    tail_shared::parallel_mode_summary_line(app)
}

#[cfg(test)]
// Alert copy is optional and deserves direct branch coverage without widening production surface area.
pub(super) fn parallel_mode_alert_line(app: &NativeTuiApp) -> Option<String> {
    tail_shared::parallel_mode_alert_line(app)
}

#[cfg(test)]
// Tests can construct footer inputs explicitly while production callers use higher-level app projections.
pub(super) fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
    plan_indicator::current_plan_mode_indicator(app)
}

// Re-export the planning substate label so workspace popups and footer indicators share the same wording.
pub(super) fn plan_runtime_substate_label(snapshot: &PlanningRuntimeSnapshot) -> &'static str {
    plan_indicator::plan_runtime_substate_label(snapshot)
}
