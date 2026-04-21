#[path = "assembly_contract/builder.rs"]
mod builder;

use super::sections::composition::PlanningSimpleReviewOverlaySections;
use crate::adapter::inbound::tui::app::Line;
use builder::build_simple_review_assembly_contract_from_sections;

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
    build_simple_review_assembly_contract_from_sections(sections)
}
