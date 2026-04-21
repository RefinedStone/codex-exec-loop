use super::sections::composition::PlanningSimpleReviewOverlaySections;
use crate::adapter::inbound::tui::app::Line;

pub(super) struct PlanningSimpleReviewAssemblyContract {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) status_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) fn build_simple_review_assembly_contract(
    sections: PlanningSimpleReviewOverlaySections,
) -> PlanningSimpleReviewAssemblyContract {
    PlanningSimpleReviewAssemblyContract {
        header_lines: sections.header_lines,
        summary_lines: sections.summary_lines,
        option_lines: sections.option_lines,
        status_lines: sections.status_view.status_lines,
        key_lines: sections.status_view.key_lines,
    }
}
