pub(super) mod controller;
mod debug_panel_state;
mod presentation;
pub(crate) mod status_projection;

pub(super) use debug_panel_state::{
    PlannerVisibility, PlannerWorkerPanelState, PlannerWorkerStatus,
};
pub(super) use presentation::{
    build_automation_preview_lines, build_automation_status_lines, build_planner_panel_lines,
};
