#[path = "assembly_contract/builder.rs"]
mod builder;
#[path = "assembly_contract/surface.rs"]
mod surface;

use super::sections::composition::PlanningSimpleReviewOverlaySections;
use builder::build_simple_review_assembly_contract_from_sections;
pub(super) use surface::PlanningSimpleReviewAssemblyContract;

pub(super) fn build_simple_review_assembly_contract(
    sections: PlanningSimpleReviewOverlaySections,
) -> PlanningSimpleReviewAssemblyContract {
    build_simple_review_assembly_contract_from_sections(sections)
}
