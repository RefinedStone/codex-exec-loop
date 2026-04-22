use super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::super::super::{assembly, contract};

pub(super) fn build_simple_review_overlay_view_from_copy(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    assembly::build_simple_review_overlay_view_from_contract(
        contract::build_simple_review_assembly_contract_from_copy(&copy),
    )
}
