use super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::super::super::assembly_contract::PlanningSimpleReviewAssemblyContract;
use super::super::super::chaining::build_simple_review_assembly_contract_for_copy;

pub(super) fn build_simple_review_assembly_contract_from_copy(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewAssemblyContract {
    build_simple_review_assembly_contract_for_copy(copy)
}
