// Status panel facade keeps footer/tail copy modules private while exposing the few stable projection helpers
// needed by shell rendering, overlays, and contract tests.
use ratatui::text::Line;

use crate::application::service::planning::PlanningRuntimeProjection;

use super::ConversationViewModel;
use super::NativeTuiApp;

// Width-aware inline tail layout owns cursor placement and top-anchored startup behavior.
#[path = "status_panels/live_status_layout.rs"]
mod live_status_layout;
// Planning substate vocabulary is centralized here so workspace popup copy stays consistent.
#[path = "status_panels/plan_indicator.rs"]
mod plan_indicator;
// Parallel slot activity copy mirrors the single-turn working line in the inline tail.
#[path = "status_panels/parallel_working_copy.rs"]
mod parallel_working_copy;
// Tail copy builds the textual ribbon; live_status_layout decides how that ribbon occupies terminal rows.
#[path = "status_panels/tail_copy.rs"]
mod tail_copy;
// Shared status helpers stay behind this facade to keep overlay modules from depending on tail internals.
#[path = "status_panels/tail_shared.rs"]
mod tail_shared;

// Rendering needs the full layout DTO; callers outside the TUI adapter should never see this presentation type.
pub(in super::super) use live_status_layout::InlineTailView;

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

// Re-export the planning substate label so workspace popups and footer indicators share the same wording.
pub(super) fn plan_runtime_substate_label(
    runtime_projection: &PlanningRuntimeProjection,
) -> &'static str {
    plan_indicator::plan_runtime_substate_label(runtime_projection)
}
