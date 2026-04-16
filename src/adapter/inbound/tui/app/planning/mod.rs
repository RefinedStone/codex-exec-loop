pub(super) mod controller;
mod debug_panel_state;
mod presentation;
mod status_projection;

pub(super) use debug_panel_state::{
    PlannerVisibility, PlannerWorkerPanelState, PlannerWorkerStatus,
};
pub(super) use presentation::{
    build_automation_preview_lines, build_automation_status_lines,
    build_planner_panel_lines, build_planning_notice_line, build_planning_summary_line,
};
pub(super) use status_projection::{
    build_planning_followup_surface_projection, build_planning_status_surface_projection,
    build_queue_framing_lines_from_snapshot, build_queue_framing_summary_from_snapshot,
    build_resumed_session_status_text, compact_queue_framing_summary,
};
