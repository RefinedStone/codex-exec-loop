//! Only the coordinator's exact current Merge basis can create an Auto permit.
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkDeliveryAuthority, IssueWorkPermitRequest,
};
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkDeliveryAuthority;
use crate::domain::issue_automation::decision::IssueDecisionBasis;
use crate::domain::issue_automation::source::validate_repository_name_with_owner;
use anyhow::{Result, anyhow, bail};

pub(super) fn validate(
    request: &IssueWorkPermitRequest,
    review: &IssueDecisionBasis,
) -> Result<PlanningWorkDeliveryAuthority> {
    match request
        .delivery_authority
        .as_ref()
        .ok_or_else(|| anyhow!("the delivery permit requires explicit delivery authority"))?
    {
        IssueWorkDeliveryAuthority::HumanReview => Ok(PlanningWorkDeliveryAuthority::HumanReview),
        IssueWorkDeliveryAuthority::Auto {
            merge_basis,
            source_repository,
        } => {
            merge_basis.validate()?;
            validate_repository_name_with_owner(source_repository).map_err(|_| {
                anyhow!("automatic delivery requires a canonical source repository")
            })?;
            if !request.delivery
                || merge_basis.authorization_epoch != request.authorization_epoch
                || merge_basis.work_order_id.as_deref() != Some(request.work_order_id.as_str())
                || merge_basis.policy_version != review.policy_version
                || merge_basis.field_hash != review.field_hash
                || merge_basis.delivery != review.delivery
                || merge_basis
                    .delivery
                    .as_ref()
                    .is_none_or(|range| range.source_commit_sha != request.source_sha)
            {
                bail!("automatic delivery does not match the current merge and source basis");
            }
            let authority = PlanningWorkDeliveryAuthority::Auto {
                merge_gate_revision: merge_basis.gate_revision,
                source_repository: source_repository.clone(),
            };
            authority.validate().map_err(|error| anyhow!(error))?;
            Ok(authority)
        }
    }
}

#[cfg(test)]
mod tests;
