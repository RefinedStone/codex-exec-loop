//! Capture once before delayed preparation; never adopt a resumed generation mid-operation.
use super::ParallelModeService;
use crate::application::port::agent_action::AgentActionGuard;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkerActionIdentity;
use crate::application::service::issue_binding_admission::{
    IssueBindingActionGuard, IssueBindingAdmission,
};
use crate::domain::issue_automation::IssueAutomationBindingId;
use anyhow::{Context, Result, bail};
use std::sync::Arc;

impl AgentActionGuard for IssueBindingActionGuard {
    fn validate(&self) -> Result<()> {
        IssueBindingActionGuard::validate(self)
    }
}

#[derive(Clone)]
pub(crate) struct IssueBindingActionConfiguration {
    pub admission: Arc<IssueBindingAdmission>,
    pub store: Arc<dyn IssueAutomationStorePort>,
}

#[derive(Debug, Clone)]
pub(crate) struct CapturedTaskAction {
    pub identity: Option<PlanningWorkerActionIdentity>,
    binding: Option<(Arc<IssueBindingAdmission>, IssueAutomationBindingId, u64)>,
}

impl CapturedTaskAction {
    pub(crate) fn issue_capture(
        &self,
    ) -> Option<crate::domain::planning::IssueBindingActionCapture> {
        let (_, binding, generation) = self.binding.as_ref()?;
        let identity = self.identity.as_ref()?;
        Some(crate::domain::planning::IssueBindingActionCapture {
            binding_id: binding.clone(),
            revision: *generation,
            work_order_id: identity.work_order_id.clone(),
            authorization_epoch: identity.authorization_epoch,
        })
    }

    pub(crate) fn acquire(&self) -> Result<Box<dyn AgentActionGuard>> {
        match &self.binding {
            Some((admission, binding, generation)) => {
                Ok(Box::new(admission.acquire(binding, *generation)?))
            }
            None => Ok(Box::new(())),
        }
    }

    pub(crate) fn is_current(&self) -> bool {
        self.binding
            .as_ref()
            .is_none_or(|(admission, binding, generation)| {
                admission.capture(binding).ok() == Some(*generation)
            })
    }
}

pub(crate) fn restore_issue_capture(
    authority: &dyn PlanningAuthorityPort,
    configuration: Option<&IssueBindingActionConfiguration>,
    workspace: &str,
    task_id: &str,
    capture: &crate::domain::planning::IssueBindingActionCapture,
) -> Result<CapturedTaskAction> {
    let configuration = configuration.context("issue binding admission is unavailable")?;
    let canonical_workspace = issue_workspace(authority, workspace)?;
    let state = configuration
        .store
        .issue_binding_pause_state(&canonical_workspace, &capture.binding_id)?;
    configuration.admission.register_if_absent(&state)?;
    let action = CapturedTaskAction {
        identity: Some(PlanningWorkerActionIdentity {
            work_order_id: capture.work_order_id.clone(),
            origin: "issue_automation".into(),
            authorization_epoch: capture.authorization_epoch,
        }),
        binding: Some((
            configuration.admission.clone(),
            capture.binding_id.clone(),
            capture.revision,
        )),
    };
    validate(authority, Some(configuration), workspace, task_id, &action)?;
    Ok(action)
}

pub(crate) fn capture(
    authority: &dyn PlanningAuthorityPort,
    configuration: Option<&IssueBindingActionConfiguration>,
    workspace: &str,
    task_id: &str,
) -> Result<CapturedTaskAction> {
    let identity = authority.worker_action_identity_for_task(workspace, task_id)?;
    let binding = if let Some(identity) = identity
        .as_ref()
        .filter(|identity| identity.origin == "issue_automation")
    {
        let configuration = configuration.context("issue binding admission is unavailable")?;
        let canonical_workspace = issue_workspace(authority, workspace)?;
        let binding = configuration
            .store
            .resolve_issue_work_binding(&canonical_workspace, &identity.work_order_id)?
            .context("issue work has no verified binding lineage")?;
        let state = configuration
            .store
            .issue_binding_pause_state(&canonical_workspace, &binding)?;
        configuration.admission.register_if_absent(&state)?;
        let generation = configuration.admission.capture(&binding)?;
        Some((configuration.admission.clone(), binding, generation))
    } else {
        None
    };
    Ok(CapturedTaskAction { identity, binding })
}

pub(crate) fn validate(
    authority: &dyn PlanningAuthorityPort,
    configuration: Option<&IssueBindingActionConfiguration>,
    workspace: &str,
    task_id: &str,
    captured: &CapturedTaskAction,
) -> Result<()> {
    if authority.worker_action_identity_for_task(workspace, task_id)? != captured.identity {
        bail!("worker action lineage or authorization epoch changed");
    }
    if let Some((_, binding, _)) = &captured.binding {
        let configuration = configuration.context("issue binding admission is unavailable")?;
        let identity = captured
            .identity
            .as_ref()
            .context("issue action identity is unavailable")?;
        let canonical_workspace = issue_workspace(authority, workspace)?;
        if configuration
            .store
            .resolve_issue_work_binding(&canonical_workspace, &identity.work_order_id)?
            .as_ref()
            != Some(binding)
        {
            bail!("issue work binding changed before its action");
        }
    }
    if !captured.is_current() {
        bail!("issue binding automation is paused or its generation changed");
    }
    Ok(())
}

pub(super) fn paused(
    authority: &dyn PlanningAuthorityPort,
    configuration: Option<&IssueBindingActionConfiguration>,
    workspace: &str,
    task_id: &str,
) -> Result<bool> {
    let Some(identity) = authority.worker_action_identity_for_task(workspace, task_id)? else {
        return Ok(false);
    };
    if identity.origin != "issue_automation" {
        return Ok(false);
    }
    let configuration = configuration.context("issue binding admission is unavailable")?;
    let canonical_workspace = issue_workspace(authority, workspace)?;
    let binding = configuration
        .store
        .resolve_issue_work_binding(&canonical_workspace, &identity.work_order_id)?
        .context("issue work has no verified binding lineage")?;
    let state = configuration
        .store
        .issue_binding_pause_state(&canonical_workspace, &binding)?;
    configuration.admission.register_if_absent(&state)?;
    if !state.allows_automation() {
        return Ok(true);
    }
    configuration.admission.capture(&binding)?;
    Ok(false)
}

fn issue_workspace(authority: &dyn PlanningAuthorityPort, workspace: &str) -> Result<String> {
    // Git returns a process-facing path spelling, and a worker may pass a linked
    // worktree. Issue references instead use the primary authority workspace's exact
    // UTF-8 scope, including the canonical Windows prefix. Resolve it through the
    // existing authority port; never guess aliases or use a lossy Path conversion.
    Ok(authority
        .resolve_authority_location(workspace)?
        .canonical_repo_root)
}

impl ParallelModeService {
    pub(super) fn capture_task_action(
        &self,
        workspace: &str,
        task_id: &str,
    ) -> Result<CapturedTaskAction> {
        capture(
            self.planning_authority.as_ref(),
            self.issue_binding.as_ref(),
            workspace,
            task_id,
        )
    }
}
