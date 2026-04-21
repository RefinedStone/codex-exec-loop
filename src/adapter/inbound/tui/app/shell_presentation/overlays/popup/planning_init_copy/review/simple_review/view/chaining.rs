use super::super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::assembly_contract::{
    PlanningSimpleReviewAssemblyContract, build_simple_review_assembly_contract,
};
use super::sections::collect_simple_review_overlay_sections;

pub(super) fn build_simple_review_assembly_contract_for_copy(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewAssemblyContract {
    build_simple_review_assembly_contract(collect_simple_review_overlay_sections(copy))
}
