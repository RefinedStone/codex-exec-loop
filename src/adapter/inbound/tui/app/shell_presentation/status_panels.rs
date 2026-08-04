// Status panel facade keeps footer/tail copy modules private while exposing the few stable projection helpers
// needed by shell rendering, overlays, and contract tests.
use ratatui::text::Line;

use crate::domain::planning::RuntimeProjection as PlanningRuntimeProjection;

use super::ConversationScreenModel;

// Activity rail copy owns cross-source priority and width budgeting for the live operator notice.
#[path = "status_panels/activity_rail.rs"]
mod activity_rail;
// Width-aware shell tail layout owns cursor placement and top-anchored startup behavior.
#[path = "status_panels/live_status_layout.rs"]
mod live_status_layout;
// Operator ribbon projection turns authoritative shell facts into a stable,
// width-aware status rail and keeps raw warning payloads in diagnostics.
#[path = "status_panels/operator_ribbon.rs"]
mod operator_ribbon;
// Planning substate vocabulary is centralized here so workspace popup copy stays consistent.
#[path = "status_panels/plan_indicator.rs"]
mod plan_indicator;
// Parallel slot activity copy mirrors the single-turn working line in the shell tail.
#[path = "status_panels/parallel_working_copy.rs"]
mod parallel_working_copy;
// Tail copy builds the textual ribbon; live_status_layout decides how that ribbon occupies terminal rows.
#[path = "status_panels/tail_copy.rs"]
mod tail_copy;
// Shared status helpers stay behind this facade to keep overlay modules from depending on tail internals.
#[path = "status_panels/tail_shared.rs"]
mod tail_shared;

// Rendering needs the full layout DTO; callers outside the TUI adapter should never see this presentation type.
pub(in super::super) use live_status_layout::{ShellTailView, composer_inner_width};

// Production entrypoint for the fullscreen bottom region: text rows plus cursor/layout metadata.
pub(crate) fn build_shell_tail_view(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
) -> ShellTailView {
    live_status_layout::build_shell_tail_view(screen_model, content_width)
}

pub(in crate::adapter::inbound::tui::app) fn build_operator_diagnostic_lines(
    screen_model: &ConversationScreenModel<'_>,
) -> Vec<Line<'static>> {
    operator_ribbon::build_operator_diagnostic_lines(screen_model)
}

// Re-export the planning substate label so workspace popups and footer indicators share the same wording.
pub(super) fn plan_runtime_substate_label(
    runtime_projection: &PlanningRuntimeProjection,
) -> &'static str {
    plan_indicator::plan_runtime_substate_label(runtime_projection)
}
