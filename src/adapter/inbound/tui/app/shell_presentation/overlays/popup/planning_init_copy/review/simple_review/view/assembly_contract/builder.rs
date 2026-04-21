use super::super::sections::composition::PlanningSimpleReviewOverlaySections;
use super::PlanningSimpleReviewAssemblyContract;

pub(super) fn build_simple_review_assembly_contract_from_sections(
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
