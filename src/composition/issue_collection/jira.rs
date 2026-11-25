//! Shared Gateway credential ownership feeds repository graphs; no per-repository owner is built.
//! The repository policy selects GitHub, Jira or Linear; a connected source never falls back.
use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
use crate::adapter::outbound::github::{
    issue_feedback::GithubIssueFeedbackAdapter, issue_source::GithubIssueSourceAdapter,
};
use crate::adapter::outbound::jira::{JiraIssueFeedbackAdapter, JiraIssueSourceAdapter};
use crate::adapter::outbound::linear::{LinearIssueFeedbackAdapter, LinearIssueSourceAdapter};
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_feedback_port::*;
use crate::application::port::outbound::issue_source_claim_port::IssueSourceClaimPort;
use crate::application::port::outbound::issue_source_port::*;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::jira::binding_for_source;
use crate::application::service::issue_feedback::validate_send_policy;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::composition::provider_credentials::ProviderServices;
use crate::domain::issue_automation::feedback::IssueFeedbackIntent;
use crate::domain::issue_automation::policy::{
    IssueAutomationJiraBinding, IssueAutomationJiraCloseTransition, IssueAutomationLinearBinding,
    IssueAutomationLinearCloseState,
};
use crate::domain::issue_automation::source::{
    IssueSourceFailure, IssueSourceRead, IssueSourceRepository,
};
use crate::domain::issue_work::{IssueProvider, IssueReference};
use anyhow::{Context, Result, ensure};
use std::path::PathBuf;
use std::sync::Arc;

fn policy() -> IssueAutomationPolicyService {
    IssueAutomationPolicyService::new(Arc::new(FilesystemIssueAutomationPolicySourceAdapter::new()))
}
fn owns(
    workspace: &str,
    store: &dyn IssueAutomationStorePort,
    claims: &dyn IssueSourceClaimPort,
    issue: &IssueReference,
) -> Result<bool> {
    ensure!(
        issue.project_scope.workspace_dir == workspace,
        "issue source workspace changed"
    );
    let binding = binding_for_source(
        workspace,
        store,
        &IssueSourceRepository::from_reference(issue),
        Some(issue),
    )?;
    claims.owns(issue, binding.id())
}

/// The repository file opts in to at most one connected source.
enum ConnectedBinding {
    Jira(IssueAutomationJiraBinding),
    Linear(IssueAutomationLinearBinding),
}

pub(super) struct RoutedIssueSource {
    workspace: String,
    github: GithubIssueSourceAdapter,
    store: Arc<dyn IssueAutomationStorePort>,
    claims: Arc<dyn IssueSourceClaimPort>,
    providers: Option<ProviderServices>,
}
impl RoutedIssueSource {
    pub(super) fn new(
        workspace: &str,
        store: Arc<dyn IssueAutomationStorePort>,
        claims: Arc<dyn IssueSourceClaimPort>,
        providers: Option<ProviderServices>,
    ) -> Self {
        Self {
            workspace: workspace.to_owned(),
            github: GithubIssueSourceAdapter::new(PathBuf::from(workspace)),
            store,
            claims,
            providers,
        }
    }
    fn binding(&self) -> std::result::Result<Option<ConnectedBinding>, IssueSourceFailure> {
        let binding = policy()
            .resolve(&self.workspace)
            .map_err(|_| IssueSourceFailure::ProviderUnavailable)?
            .policy
            .binding;
        Ok(match (binding.jira, binding.linear) {
            (Some(jira), _) => Some(ConnectedBinding::Jira(jira)),
            (None, Some(linear)) => Some(ConnectedBinding::Linear(linear)),
            (None, None) => None,
        })
    }
    fn linear(
        &self,
        binding: IssueAutomationLinearBinding,
    ) -> std::result::Result<LinearIssueSourceAdapter, IssueSourceFailure> {
        let providers = self
            .providers
            .as_ref()
            .ok_or(IssueSourceFailure::ProviderUnavailable)?;
        LinearIssueSourceAdapter::new(
            binding,
            providers.credentials.clone(),
            providers.api.clone(),
        )
    }
    fn jira(
        &self,
        binding: IssueAutomationJiraBinding,
    ) -> std::result::Result<JiraIssueSourceAdapter, IssueSourceFailure> {
        let providers = self
            .providers
            .as_ref()
            .ok_or(IssueSourceFailure::ProviderUnavailable)?;
        JiraIssueSourceAdapter::new(
            binding,
            providers.credentials.clone(),
            providers.api.clone(),
        )
    }
}
impl IssueSourcePort for RoutedIssueSource {
    fn capabilities(&self) -> IssueSourceCapabilities {
        match self.binding() {
            Ok(None) => self.github.capabilities(),
            Ok(Some(_)) => IssueSourceCapabilities {
                discovery: true,
                resync: true,
                incremental: true,
                comments: self.providers.is_some(),
                transition: self.providers.is_some(),
                labels: false,
                assign: false,
                link_pull_request: false,
                board_fields: false,
            },
            Err(_) => IssueSourceCapabilities {
                discovery: false,
                resync: false,
                incremental: false,
                comments: false,
                transition: false,
                labels: false,
                assign: false,
                link_pull_request: false,
                board_fields: false,
            },
        }
    }
    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> std::result::Result<IssueDiscoveryPage, IssueSourceFailure> {
        match self.binding()? {
            Some(ConnectedBinding::Jira(binding)) => self.jira(binding)?.discover(request),
            Some(ConnectedBinding::Linear(binding)) => self.linear(binding)?.discover(request),
            None => self.github.discover(request),
        }
    }
    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        let expected = request.issue.clone();
        let unknown = |reason| IssueSourceRead::Unknown {
            issue: expected.clone(),
            reason,
        };
        let owned = || {
            owns(
                &self.workspace,
                self.store.as_ref(),
                self.claims.as_ref(),
                &expected,
            )
            .unwrap_or(false)
        };
        match self.binding() {
            Ok(Some(ConnectedBinding::Jira(binding)))
                if expected.provider == IssueProvider::Jira =>
            {
                if !owned() {
                    return unknown(IssueSourceFailure::IdentityMismatch);
                }
                match self.jira(binding) {
                    Ok(source) => source.resync(request),
                    Err(reason) => unknown(reason),
                }
            }
            Ok(Some(ConnectedBinding::Linear(binding)))
                if expected.provider == IssueProvider::Linear =>
            {
                if !owned() {
                    return unknown(IssueSourceFailure::IdentityMismatch);
                }
                match self.linear(binding) {
                    Ok(source) => source.resync(request),
                    Err(reason) => unknown(reason),
                }
            }
            Ok(None) if expected.provider == IssueProvider::GitHub => self.github.resync(request),
            Ok(_) => unknown(IssueSourceFailure::IdentityMismatch),
            Err(reason) => unknown(reason),
        }
    }
}

pub(super) struct RoutedIssueFeedback {
    workspace: String,
    github: GithubIssueFeedbackAdapter,
    jira: Option<JiraIssueFeedbackAdapter>,
    linear: Option<LinearIssueFeedbackAdapter>,
    store: Arc<dyn IssueAutomationStorePort>,
    claims: Arc<dyn IssueSourceClaimPort>,
}
impl RoutedIssueFeedback {
    pub(super) fn new(
        workspace: &str,
        store: Arc<dyn IssueAutomationStorePort>,
        claims: Arc<dyn IssueSourceClaimPort>,
        providers: Option<&ProviderServices>,
    ) -> Self {
        let selected_workspace = workspace.to_owned();
        let binding_reader = Arc::new(move || {
            policy()
                .resolve(&selected_workspace)?
                .policy
                .binding
                .jira
                .context("Jira binding is unavailable")
        });
        let linear_workspace = workspace.to_owned();
        let linear_binding_reader = Arc::new(move || {
            policy()
                .resolve(&linear_workspace)?
                .policy
                .binding
                .linear
                .context("Linear binding is unavailable")
        });
        let policy_workspace = workspace.to_owned();
        let policy_store = store.clone();
        let automation = policy();
        let disclosure =
            IssuePolicyService::new(Arc::new(FilesystemIssuePolicySourceAdapter::new()));
        let send_policy_validator = Arc::new(move |intent: &IssueFeedbackIntent| {
            validate_send_policy(
                &policy_workspace,
                policy_store.as_ref(),
                &automation,
                &disclosure,
                intent,
            )
        });
        let jira = providers.map(|providers| {
            JiraIssueFeedbackAdapter::new(
                providers.credentials.clone(),
                providers.api.clone(),
                binding_reader,
                send_policy_validator.clone(),
            )
        });
        let linear = providers.map(|providers| {
            LinearIssueFeedbackAdapter::new(
                providers.credentials.clone(),
                providers.api.clone(),
                linear_binding_reader,
                send_policy_validator,
            )
        });
        Self {
            workspace: workspace.to_owned(),
            github: GithubIssueFeedbackAdapter::new(PathBuf::from(workspace)),
            jira,
            linear,
            store,
            claims,
        }
    }
    fn for_issue(&self, issue: &IssueReference) -> Result<&dyn IssueFeedbackPort> {
        let policy = policy().resolve(&self.workspace)?;
        ensure!(
            issue.project_scope.workspace_dir == self.workspace
                && issue.provider == policy.policy.selected_provider(),
            "feedback source changed"
        );
        match issue.provider {
            IssueProvider::GitHub => Ok(&self.github),
            IssueProvider::Jira => {
                ensure!(
                    owns(
                        &self.workspace,
                        self.store.as_ref(),
                        self.claims.as_ref(),
                        issue
                    )?,
                    "Jira issue belongs to another binding"
                );
                Ok(self
                    .jira
                    .as_ref()
                    .context("Jira credentials are unavailable")?)
            }
            IssueProvider::Linear => {
                ensure!(
                    owns(
                        &self.workspace,
                        self.store.as_ref(),
                        self.claims.as_ref(),
                        issue
                    )?,
                    "Linear issue belongs to another binding"
                );
                Ok(self
                    .linear
                    .as_ref()
                    .context("Linear credentials are unavailable")?)
            }
        }
    }
}
impl IssueFeedbackPort for RoutedIssueFeedback {
    fn actor_id(&self, issue: &IssueReference) -> Result<String> {
        self.for_issue(issue)?.actor_id(issue)
    }
    fn actor_context(&self, issue: &IssueReference) -> Result<IssueFeedbackActorContext> {
        self.for_issue(issue)?.actor_context(issue)
    }
    fn read_label(&self, issue: &IssueReference, name: &str) -> Result<IssueFeedbackLabelSnapshot> {
        self.for_issue(issue)?.read_label(issue, name)
    }
    fn read_closure(&self, issue: &IssueReference) -> Result<IssueFeedbackClosureSnapshot> {
        self.for_issue(issue)?.read_closure(issue)
    }
    fn read_jira_close_transition(
        &self,
        issue: &IssueReference,
        actor: &IssueFeedbackActorContext,
        mapping: &IssueAutomationJiraCloseTransition,
    ) -> Result<IssueFeedbackJiraTransitionSnapshot> {
        self.for_issue(issue)?
            .read_jira_close_transition(issue, actor, mapping)
    }
    fn read_linear_close_state(
        &self,
        issue: &IssueReference,
        actor: &IssueFeedbackActorContext,
        mapping: &IssueAutomationLinearCloseState,
    ) -> Result<IssueFeedbackLinearStateSnapshot> {
        self.for_issue(issue)?
            .read_linear_close_state(issue, actor, mapping)
    }
    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        self.for_issue(&intent.issue)?.write(intent)
    }
    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        self.for_issue(&intent.issue)?.reconcile(intent)
    }
}
