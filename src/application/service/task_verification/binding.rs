use super::*;
use crate::application::port::agent_action::AgentActionGuard;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::service::issue_binding_admission::IssueBindingAdmission;
use crate::application::service::parallel_mode::binding_action::{
    self, CapturedTaskAction, IssueBindingActionConfiguration,
};

#[derive(Clone)]
pub(super) struct BindingVerification {
    captured: CapturedTaskAction,
    configuration: Option<IssueBindingActionConfiguration>,
}

impl TaskVerificationService {
    pub fn with_issue_binding_admission(
        mut self,
        admission: Arc<IssueBindingAdmission>,
        store: Arc<dyn IssueAutomationStorePort>,
    ) -> Self {
        self.issue_binding = Some(IssueBindingActionConfiguration { admission, store });
        self
    }

    pub(super) fn capture_binding(&self, task_id: &str) -> Result<BindingVerification> {
        Ok(BindingVerification {
            captured: binding_action::capture(
                self.dependencies.authority.as_ref(),
                self.issue_binding.as_ref(),
                &self.dependencies.workspace,
                task_id,
            )?,
            configuration: self.issue_binding.clone(),
        })
    }
}

impl BindingVerification {
    pub(super) fn acquire(&self) -> Result<Box<dyn AgentActionGuard>> {
        self.captured.acquire()
    }

    pub(super) fn is_current(&self) -> bool {
        self.captured.is_current()
    }

    pub(super) fn validate(
        &self,
        dependencies: &TaskVerificationDependencies,
        stored: &StoredTaskVerification,
    ) -> Result<()> {
        binding_action::validate(
            dependencies.authority.as_ref(),
            self.configuration.as_ref(),
            &dependencies.workspace,
            &stored.attempt.scope.task_id,
            &self.captured,
        )
    }
}
