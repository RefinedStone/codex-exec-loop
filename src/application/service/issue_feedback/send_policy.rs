//! Final policy recheck for adapters whose remote preflight happens after durable Sent.
use anyhow::{Result, ensure};

use super::{
    IssueFeedbackIntent, IssueFeedbackOperation, IssuePolicyService, issue_policy_version,
};
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_autonomy::resolve_runtime_policy;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::IssueAutomationCloseWhen;

pub(crate) fn validate_send_policy(
    workspace: &str,
    store: &dyn IssueAutomationStorePort,
    automation: &IssueAutomationPolicyService,
    disclosure: &IssuePolicyService,
    intent: &IssueFeedbackIntent,
) -> Result<()> {
    ensure!(
        intent.issue.project_scope.workspace_dir == workspace,
        "feedback policy workspace changed"
    );
    let (current, _) = resolve_runtime_policy(workspace, store, automation)?;
    let sharing = disclosure.resolve(workspace)?;
    let policy = &current.policy;
    ensure!(
        policy.selected_provider() == intent.issue.provider
            && policy.mode_for_provider(intent.issue.provider) != IssueAutomationAutonomy::Off
            && policy.binding.jira.as_ref().is_some_and(|binding| {
                binding.cloud_id == intent.issue.server_id
                    && binding.project_id == intent.issue.repository_id
                    && binding.expected_account_id == intent.actor_id
            })
            && current.version == intent.automation_policy_version
            && issue_policy_version(&sharing) == intent.disclosure_policy_version,
        "feedback policy changed before send"
    );
    let allowed = match &intent.operation {
        IssueFeedbackOperation::CreateComment { .. }
        | IssueFeedbackOperation::UpdateComment { .. } => policy.feedback.comment,
        IssueFeedbackOperation::CloseIssue {
            jira_transition: Some(target),
            ..
        } => {
            policy.feedback.transition
                && policy.close.when == IssueAutomationCloseWhen::Integration
                && policy
                    .binding
                    .jira
                    .as_ref()
                    .and_then(|binding| binding.close_transition.as_ref())
                    .is_some_and(|mapping| {
                        mapping.transition_id == target.transition_id
                            && mapping.to_status_id == target.to_status_id
                            && mapping.from_status_ids.contains(&target.from_status_id)
                    })
        }
        _ => false,
    };
    ensure!(allowed, "feedback operation is disabled before send");
    Ok(())
}
