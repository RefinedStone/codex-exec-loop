//! Auto is a current policy decision, distinct from a human exception to a risky diff.
use super::*;
use crate::application::port::outbound::issue_work_execution_port::IssueWorkDeliveryAuthority;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::IssueAutomationGateAction;
use crate::domain::issue_automation::source::{
    IssueRepositoryVisibility, validate_repository_name_with_owner,
};

impl IssueCoordinatorService {
    pub(super) fn current_delivery_authority(
        &self,
        work: &IssueWorkCheckpoint,
        item: &IssueCollectionItem,
        assessment: &IssueTriageAssessment,
        delivery: &IssueDecisionBasis,
    ) -> Result<IssueWorkDeliveryAuthority> {
        if assessment.effective_mode() != IssueAutomationAutonomy::Auto
            || work
                .delivery_assessment
                .as_ref()
                .is_none_or(|review| review.verdict.requires_human())
            || [IssueAutomationGate::Delivery, IssueAutomationGate::Merge]
                .iter()
                .any(|expected| {
                    !assessment.gates().iter().any(|gate| {
                        gate.gate == *expected
                            && gate.enabled
                            && gate.action == IssueAutomationGateAction::Auto
                    })
                })
        {
            return Ok(IssueWorkDeliveryAuthority::HumanReview);
        }
        // Source metadata is freshly reassessed before and after grant. A display
        // URL, an older observation, or a private fork never supplies this identity.
        let Some(IssueSourceRead::Observed(observation)) = item.current.as_ref() else {
            bail!("automatic delivery requires the current issue source observation");
        };
        if observation.issue != work.order.issue
            || !matches!(
                observation.repository_visibility,
                IssueRepositoryVisibility::Private | IssueRepositoryVisibility::Internal
            )
        {
            bail!("automatic delivery requires the current non-public source repository");
        }
        let source_repository = observation
            .repository_name_with_owner
            .as_ref()
            .ok_or_else(|| anyhow!("the current source repository identity is unavailable"))?
            .clone();
        validate_repository_name_with_owner(&source_repository)
            .map_err(|_| anyhow!("the current source repository identity is invalid"))?;
        let mut merge_basis = IssueDecisionBasis::from_assessment(
            assessment,
            IssueAutomationGate::Merge,
            Some(work.order.work_order_id.clone()),
        )?;
        merge_basis.authorization_epoch = delivery.authorization_epoch;
        merge_basis.delivery = delivery.delivery.clone();
        merge_basis.validate()?;
        Ok(IssueWorkDeliveryAuthority::Auto {
            merge_basis,
            source_repository,
        })
    }
}
