//! Shared Gateway credential ownership feeds repository graphs; no per-repository owner is built.
use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
use crate::adapter::outbound::github::{
    issue_feedback::GithubIssueFeedbackAdapter, issue_source::GithubIssueSourceAdapter,
};
use crate::adapter::outbound::jira::{JiraIssueFeedbackAdapter, JiraIssueSourceAdapter};
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_feedback_port::*;
use crate::application::port::outbound::issue_source_claim_port::IssueSourceClaimPort;
use crate::application::port::outbound::issue_source_port::*;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::jira::binding_for_source;
use crate::composition::provider_credentials::ProviderServices;
use crate::domain::issue_automation::feedback::IssueFeedbackIntent;
use crate::domain::issue_automation::policy::{
    IssueAutomationJiraBinding, IssueAutomationJiraCloseTransition,
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
    fn binding(
        &self,
    ) -> std::result::Result<Option<IssueAutomationJiraBinding>, IssueSourceFailure> {
        policy()
            .resolve(&self.workspace)
            .map(|policy| policy.policy.binding.jira)
            .map_err(|_| IssueSourceFailure::ProviderUnavailable)
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
            Some(binding) => self.jira(binding)?.discover(request),
            None => self.github.discover(request),
        }
    }
    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        let expected = request.issue.clone();
        let unknown = |reason| IssueSourceRead::Unknown {
            issue: expected.clone(),
            reason,
        };
        match self.binding() {
            Ok(Some(binding)) if expected.provider == IssueProvider::Jira => {
                if !owns(
                    &self.workspace,
                    self.store.as_ref(),
                    self.claims.as_ref(),
                    &expected,
                )
                .unwrap_or(false)
                {
                    return unknown(IssueSourceFailure::IdentityMismatch);
                }
                match self.jira(binding) {
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
        let jira = providers.map(|providers| {
            JiraIssueFeedbackAdapter::new(
                providers.credentials.clone(),
                providers.api.clone(),
                binding_reader,
            )
        });
        Self {
            workspace: workspace.to_owned(),
            github: GithubIssueFeedbackAdapter::new(PathBuf::from(workspace)),
            jira,
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
    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        self.for_issue(&intent.issue)?.write(intent)
    }
    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        self.for_issue(&intent.issue)?.reconcile(intent)
    }
}
