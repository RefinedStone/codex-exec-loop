#[path = "status/key_lines.rs"]
mod key_lines;
#[path = "status/lines.rs"]
mod lines;
#[path = "status/view.rs"]
mod view;

use ratatui::text::Line;

use crate::adapter::inbound::tui::app::shell_presentation::overlays::popup::planning::copy::PlanningSimpleReviewCopy;

pub(super) struct PlanningSimpleReviewStatusView {
    pub(super) status_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) fn build_simple_review_status_view(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewStatusView {
    view::build_simple_review_status_view(copy)
}
