use super::super::NativeTuiApp;
use super::status_projection::compact_queue_framing_summary;
use crate::domain::text::compact_whitespace_detail;

pub(crate) fn build_planner_panel_lines(app: &NativeTuiApp, max_detail_len: usize) -> Vec<String> {
    if !app.planner_shows_debug_details() {
        return Vec::new();
    }

    let planner = &app.planner_worker_panel_state;
    if !planner.has_content() {
        return Vec::new();
    }

    let mut first_line = format!("planner status: {}", planner.status.label());
    if let Some(queue_summary) = planner.last_queue_summary.as_deref() {
        first_line.push_str(&format!(
            "  |  planner queue: {}",
            compact_queue_framing_summary(queue_summary, max_detail_len)
        ));
    }

    let mut lines = vec![first_line];
    if let Some(summary) = planner.last_summary.as_deref() {
        lines.push(format!(
            "planner detail: {}",
            compact_whitespace_detail(summary, max_detail_len)
        ));
    }
    if let Some(notice_detail) = planner.last_notice_detail.as_deref() {
        lines.push(format!(
            "planner notice: {}",
            compact_whitespace_detail(notice_detail, max_detail_len)
        ));
    }
    if let Some(host_detail) = planner.last_host_detail.as_deref() {
        lines.push(format!(
            "planner host detail: {}",
            compact_whitespace_detail(host_detail, max_detail_len)
        ));
    }
    if let Some(rejected_summary) = planner.last_rejected_summary.as_deref() {
        lines.push(format!(
            "planner rejected: {}",
            compact_whitespace_detail(rejected_summary, max_detail_len)
        ));
    }
    lines
}
