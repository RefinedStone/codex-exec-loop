//! 이슈 장부의 확정된 원본 승인과 checkpoint를 대조한 뒤에만 task 기준을 채택한다.
use super::AkraIssueWorkExecutionAdapter;
use crate::application::service::planning::task_mutation::work_order::acceptance::VerifiedWorkOrderAcceptance;
use crate::domain::issue_automation::coordinator::{IssueWorkOrder, issue_work_order_id};
use crate::domain::issue_automation::criteria::{
    IssueWorkCriteriaAdoption, IssueWorkCriterionChannel, IssueWorkVerificationRecipe,
};
use crate::domain::planning::verification::TaskVerificationRecipe;
use crate::domain::planning::{TaskAcceptanceChannel, TaskAcceptanceCriterion};
use anyhow::{Context, Result, bail};

pub(super) fn verified(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    order: &IssueWorkOrder,
) -> Result<Option<VerifiedWorkOrderAcceptance>> {
    let Some(adoption) = &order.criteria_adoption else {
        if let Some(store) = &adapter.criteria_store
            && let Some(checkpoint) = store.get_issue_work(workspace, &order.issue)?
            && checkpoint.order.criteria_adoption.is_some()
        {
            bail!("confirmed work-order criteria cannot be omitted on resubmission");
        }
        return Ok(None);
    };
    adoption.validate().map_err(anyhow::Error::msg)?;
    if order.work_order_id != issue_work_order_id(&order.issue)
        || adoption.basis.field_hash.as_ref() != Some(&order.field_hash)
        || order.scope != workspace
        || order.issue.project_scope.workspace_dir != workspace
    {
        bail!("criteria approval belongs to another issue specification or work order");
    }
    let store = adapter
        .criteria_store
        .as_ref()
        .context("confirmed criteria store is unavailable")?;
    let stored = store
        .get_issue_decision_commit(workspace, &adoption.decision_id)?
        .context("the confirmed criteria decision is unavailable")?;
    if stored.record.request.issue() != &order.issue
        || IssueWorkCriteriaAdoption::from_stored(&stored.record)
            .map_err(anyhow::Error::msg)?
            .as_ref()
            != Some(adoption)
    {
        bail!("criteria adoption does not match the persisted human approval");
    }
    let checkpoint = store
        .get_issue_work(workspace, &order.issue)?
        .context("the approved issue work checkpoint is unavailable")?;
    checkpoint.validate().map_err(anyhow::Error::msg)?;
    if checkpoint.order != *order {
        bail!("criteria adoption does not match the immutable work checkpoint");
    }
    let criteria = adoption
        .criteria
        .iter()
        .map(|criterion| TaskAcceptanceCriterion {
            id: criterion.id.clone(),
            scenario: criterion.scenario.clone(),
            channel: match criterion.channel {
                IssueWorkCriterionChannel::Command => TaskAcceptanceChannel::Command,
                IssueWorkCriterionChannel::Data => TaskAcceptanceChannel::Data,
                IssueWorkCriterionChannel::Http => TaskAcceptanceChannel::Http,
                IssueWorkCriterionChannel::Screen => TaskAcceptanceChannel::Screen,
            },
            essential: criterion.essential,
            verification_recipe: criterion.verification_recipe.map(|recipe| match recipe {
                IssueWorkVerificationRecipe::GitDiffCheckV1 => {
                    TaskVerificationRecipe::GitDiffCheckV1
                }
            }),
        })
        .collect();
    VerifiedWorkOrderAcceptance::from_verified_receipt(
        workspace,
        &order.work_order_id,
        adoption.receipt_digest(&order.issue, &order.work_order_id),
        criteria,
    )
    .map(Some)
}
