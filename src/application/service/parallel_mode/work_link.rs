//! Publication policy at the existing GitHub delivery boundary.
use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorDeliveryTarget;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkLinkReference;

use super::{
    ParallelModeService, try_parallel_mode_integration_branch_for_repo_with_runtime,
    try_push_remote_name_with_runtime,
};

impl ParallelModeService {
    /// Read-only preflight before an issue work order can create a planning task.
    pub fn validate_work_link_reference(
        &self,
        workspace: &str,
        reference: &PlanningWorkLinkReference,
    ) -> Result<(), String> {
        reference.validate().map_err(|error| error.to_string())?;
        if !reference.close_on_merge {
            return Ok(());
        }
        let location = self
            .planning_authority
            .resolve_authority_location(workspace)
            .map_err(|error| error.to_string())?;
        let root = &location.canonical_repo_root;
        let remote = try_push_remote_name_with_runtime(self.parallel_runtime.as_ref(), root)?;
        let push_url = self
            .github_automation
            .credential_redacted_push_url_for_remote(root, &remote)
            .map_err(|error| error.to_string())?;
        let integration = try_parallel_mode_integration_branch_for_repo_with_runtime(
            self.parallel_runtime.as_ref(),
            root,
        )?;
        validate_default_branch(
            self.github_automation.as_ref(),
            root,
            &remote,
            &push_url,
            &integration,
        )
    }
}

pub(super) fn validate_delivery_target(
    github: &dyn GithubAutomationPort,
    repo_root: &str,
    target: &PlanningAuthorityDistributorDeliveryTarget,
    reference: &PlanningWorkLinkReference,
) -> Result<(), String> {
    reference.validate().map_err(|error| error.to_string())?;
    if !reference.close_on_merge {
        return Ok(());
    }
    let push_url = target
        .credential_redacted_push_url
        .as_deref()
        .ok_or_else(|| "close.when: provider_link requires a frozen delivery target".to_string())?;
    validate_default_branch(
        github,
        repo_root,
        &target.push_remote,
        push_url,
        &target.integration_branch,
    )
}

fn validate_default_branch(
    github: &dyn GithubAutomationPort,
    repo_root: &str,
    remote: &str,
    push_url: &str,
    integration: &str,
) -> Result<(), String> {
    let default_branch = github
        .repository_default_branch_for_push_url(repo_root, remote, push_url)
        .map_err(|_| {
            "close.when: provider_link default branch could not be verified".to_string()
        })?;
    if integration != default_branch {
        return Err("close.when: provider_link_requires_default_branch".to_string());
    }
    Ok(())
}
