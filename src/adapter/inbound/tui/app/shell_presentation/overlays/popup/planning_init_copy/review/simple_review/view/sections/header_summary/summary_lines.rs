use super::super::super::super::super::header;
use crate::adapter::inbound::tui::app::Line;

pub(super) fn collect_simple_review_summary_lines() -> Vec<Line<'static>> {
    header::build_simple_review_summary_lines()
}
