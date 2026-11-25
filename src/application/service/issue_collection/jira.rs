use super::*;
use crate::application::port::outbound::issue_source_claim_port::IssueSourceClaimPort;
use crate::application::port::outbound::issue_source_port::IssueDiscoveryPage;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialReadPort;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::domain::issue_automation::policy::IssueAutomationJiraBinding;
use crate::domain::issue_automation::source::IssueSourceObservation;
use crate::domain::issue_automation::{IssueAutomationBinding, TrackedIssue};
use crate::domain::provider_credentials::{
    ConnectionRef, JiraDeployment, Provider, ProviderConnectionStatus,
};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(super) struct ConnectedSourceContext {
    pub policy: IssueAutomationPolicyService,
    pub credentials: Option<Arc<dyn ProviderCredentialReadPort>>,
    pub claims: Arc<dyn IssueSourceClaimPort>,
}

/// Prefer the existing tracked owner, then the collection's binding and existing source binding.
/// The derived ID is byte-for-byte the existing collection store's first-discovery ID.
pub(crate) fn binding_for_source(
    workspace: &str,
    store: &dyn IssueAutomationStorePort,
    repository: &IssueSourceRepository,
    issue: Option<&IssueReference>,
) -> Result<IssueAutomationBinding> {
    if let Some(issue) = issue
        && let Some(tracked) = store.get_issue(workspace, issue)?
    {
        return Ok(store
            .get_binding(workspace, tracked.binding_id())?
            .context("tracked source binding is missing")?
            .binding);
    }
    if let Some(binding) = store
        .get_collection_projection(workspace)?
        .and_then(|value| value.binding)
    {
        ensure!(
            binding.provider() == repository.provider
                && binding.server_id() == repository.server_id
                && binding.repository_id() == repository.repository_id
                && binding.project_scope() == &repository.project_scope,
            "the source binding changed"
        );
        return Ok(binding);
    }
    let mut bindings = store.list_bindings(workspace)?;
    bindings.sort_by(|left, right| left.binding.id().as_str().cmp(right.binding.id().as_str()));
    if let Some(saved) = bindings.into_iter().find(|saved| {
        saved.binding.provider() == repository.provider
            && saved.binding.server_id() == repository.server_id
            && saved.binding.repository_id() == repository.repository_id
            && saved.binding.project_scope() == &repository.project_scope
    }) {
        return Ok(saved.binding);
    }
    Ok(IssueAutomationBinding::for_source(repository)?)
}

impl IssueCollectionService {
    /// A repository opts in to exactly one connected source; otherwise GitHub collection runs.
    pub(super) fn connected_selection(
        &self,
    ) -> std::result::Result<Option<super::ConnectedSelection>, IssueCollectionFailure> {
        let Some(context) = &self.connected else {
            return Ok(None);
        };
        let policy = context
            .policy
            .resolve(&self.workspace_dir)
            .map_err(|_| IssueCollectionFailure::PolicyUnavailable)?;
        let version = policy.version;
        let selection = match (policy.policy.binding.jira, policy.policy.binding.linear) {
            (Some(binding), _) => super::ConnectedSelection::Jira { binding, version },
            (None, Some(binding)) => super::ConnectedSelection::Linear { binding, version },
            (None, None) => return Ok(None),
        };
        if context.credentials.is_none() {
            return Err(IssueCollectionFailure::Source(
                IssueSourceFailure::ProviderUnavailable,
            ));
        }
        Ok(Some(selection))
    }

    fn current_jira(
        &self,
        expected_version: &str,
    ) -> Result<(&ConnectedSourceContext, IssueAutomationJiraBinding)> {
        let context = self
            .connected
            .as_ref()
            .context("Jira collection is unavailable")?;
        let policy = context.policy.resolve(&self.workspace_dir)?;
        ensure!(
            policy.version == expected_version,
            "Jira selection changed before the collection commit"
        );
        Ok((
            context,
            policy
                .policy
                .binding
                .jira
                .context("Jira binding is unavailable")?,
        ))
    }

    pub(super) fn claim_jira_page(
        &self,
        request: &IssueDiscoveryRequest,
        page: &mut IssueDiscoveryPage,
        projection: &IssueCollectionProjection,
        policy_version: &str,
    ) -> Result<()> {
        let (context, selected_binding) = self.current_jira(policy_version)?;
        let credentials = context
            .credentials
            .as_ref()
            .context("Jira credentials are unavailable")?;
        ensure!(
            page.jira_binding.as_ref() == Some(&selected_binding),
            "Jira discovery used a different policy binding"
        );
        let connection = page
            .jira_connection
            .as_ref()
            .context("Jira operation connection is missing")?;
        validate_connection(credentials.as_ref(), &selected_binding, connection)?;
        let IssueDiscoveryTarget::Repository(repository) = &request.target else {
            anyhow::bail!("Jira needs an explicit stable source");
        };
        ensure!(
            repository == &page.repository
                && repository.provider == IssueProvider::Jira
                && repository.project_scope.workspace_dir == self.workspace_dir,
            "Jira discovery changed its bound source"
        );
        ensure!(
            page.scanned_count <= request.page_size
                && page.observations.len() <= usize::from(page.scanned_count),
            "Jira page exceeds its bound"
        );
        request.validate()?;
        if let Some(cursor) = &page.next_cursor {
            cursor.validate_for(
                repository,
                &request.selection,
                request.page_size,
                &request.disclosure,
            )?;
            ensure!(
                request.cursor.as_ref() != Some(cursor),
                "Jira page did not advance"
            );
        }
        let mut seen = BTreeSet::new();
        let mut owners = Vec::with_capacity(page.observations.len());
        // Validate the complete page before the first durable claim.
        for observation in &page.observations {
            ensure!(
                repository.matches_reference(&observation.issue)
                    && seen.insert(&observation.issue.issue_id),
                "Jira page contains duplicate or foreign identities"
            );
            let binding = binding_for_source(
                &self.workspace_dir,
                self.store.as_ref(),
                repository,
                Some(&observation.issue),
            )?;
            validate_observation(observation, &binding, &request.disclosure.policy_version)?;
            let jira = observation
                .jira
                .as_ref()
                .context("Jira source metadata is missing")?;
            ensure!(
                &jira.connection == connection,
                "Jira page mixed credential generations"
            );
            credentials.validate_current(&jira.connection)?;
            owners.push(binding);
        }
        // Recheck the local fence before retaining a profile claim. A later local CAS loss
        // keeps that idempotent owner for replay; it never grants execution or transfers it.
        let current = self
            .store
            .get_collection_projection(&self.workspace_dir)?
            .context("collection disappeared before source ownership claim")?;
        ensure!(
            current.state.fence() == projection.state.fence(),
            "collection changed before source ownership claim"
        );
        let mut retained = Vec::with_capacity(page.observations.len());
        for (observation, binding) in page.observations.drain(..).zip(owners) {
            if context.claims.claim(&observation.issue, binding.id())? {
                retained.push(observation);
            }
        }
        page.observations = retained;
        // Claim storage can block or invoke another owner. Check the complete operation again
        // immediately before the caller's local page/cursor transaction, including empty pages.
        let (_, current_binding) = self.current_jira(policy_version)?;
        ensure!(
            page.jira_binding.as_ref() == Some(&current_binding),
            "Jira discovery binding changed before commit"
        );
        validate_connection(credentials.as_ref(), &current_binding, connection)?;
        for observation in &page.observations {
            credentials.validate_current(
                &observation
                    .jira
                    .as_ref()
                    .context("Jira source metadata is missing")?
                    .connection,
            )?;
        }
        Ok(())
    }

    pub(super) fn claim_known_connected_issue(&self, issue: &IssueReference) -> Result<bool> {
        if !matches!(issue.provider, IssueProvider::Jira | IssueProvider::Linear) {
            return Ok(false);
        }
        let context = self
            .connected
            .as_ref()
            .context("connected source ownership is unavailable")?;
        let repository = IssueSourceRepository::from_reference(issue);
        let binding = binding_for_source(
            &self.workspace_dir,
            self.store.as_ref(),
            &repository,
            Some(issue),
        )?;
        context.claims.claim(issue, binding.id())
    }

    pub(super) fn validate_jira_resync(
        &self,
        targets: &[IssueReference],
        reads: &mut [IssueSourceRead],
        policy_version: &str,
    ) -> Result<()> {
        let (context, binding) = self.current_jira(policy_version)?;
        let credentials = context
            .credentials
            .as_ref()
            .context("Jira credentials are unavailable")?;
        ensure!(targets.len() == reads.len(), "Jira resync batch changed");
        for (target, read) in targets.iter().zip(reads.iter_mut()) {
            if let IssueSourceRead::Observed(observation) = read {
                let repository = IssueSourceRepository::from_reference(target);
                let owner = binding_for_source(
                    &self.workspace_dir,
                    self.store.as_ref(),
                    &repository,
                    Some(target),
                )?;
                let valid = observation.issue == *target
                    && target.provider == IssueProvider::Jira
                    && target.server_id == binding.cloud_id
                    && target.repository_id == binding.project_id
                    && context.claims.owns(target, owner.id())?
                    && observation.jira.as_ref().is_some_and(|jira| {
                        validate_connection(credentials.as_ref(), &binding, &jira.connection)
                            .is_ok()
                    });
                if !valid {
                    *read = IssueSourceRead::Unknown {
                        issue: target.clone(),
                        reason: IssueSourceFailure::IdentityMismatch,
                    };
                }
            }
        }
        // A later ownership read may outlive an earlier credential check. Validate the whole
        // retained batch after every database read, against the initial policy selection.
        let (_, current_binding) = self.current_jira(policy_version)?;
        for read in reads {
            if let IssueSourceRead::Observed(observation) = read {
                let connection = &observation
                    .jira
                    .as_ref()
                    .context("Jira source metadata is missing")?
                    .connection;
                validate_connection(credentials.as_ref(), &current_binding, connection)?;
            }
        }
        Ok(())
    }
}

fn validate_connection(
    credentials: &dyn ProviderCredentialReadPort,
    binding: &IssueAutomationJiraBinding,
    expected: &ConnectionRef,
) -> Result<()> {
    let state = credentials.current_state(Provider::Jira)?;
    ensure!(
        state.provider == Provider::Jira
            && state.profile_id == expected.profile_id
            && state.status == ProviderConnectionStatus::Connected,
        "Jira connection is unavailable"
    );
    let active = state
        .active_connection
        .context("Jira connection is unavailable")?;
    active.metadata.validate(Provider::Jira)?;
    ensure!(
        &active.reference == expected
            && expected.provider == Provider::Jira
            && active.metadata.configuration.deployment == Some(JiraDeployment::Cloud)
            && active.metadata.configuration.site_url.as_deref() == Some(binding.site_url.as_str())
            && active.metadata.account_id == binding.expected_account_id,
        "Jira connection no longer matches the selected binding"
    );
    credentials.validate_current(expected)?;
    Ok(())
}

pub(super) fn validate_observation(
    observation: &IssueSourceObservation,
    binding: &IssueAutomationBinding,
    disclosure: &str,
) -> Result<()> {
    ensure!(
        serde_json::to_vec(observation)?.len() <= 4 * 1024 * 1024,
        "Jira observation is too large"
    );
    observation.validate_repository_metadata()?;
    if let Some(web) = &observation.web {
        web.validate_for(&observation.issue)?;
    }
    match &observation.content {
        IssueSourceContent::Shared(snapshot) => {
            ensure!(
                snapshot.policy_version == disclosure,
                "Jira disclosure policy changed"
            );
            TrackedIssue::discovered(observation.issue.clone(), binding, snapshot.clone())?;
        }
        IssueSourceContent::Withheld {
            policy_version,
            basis,
            ..
        } => {
            ensure!(
                policy_version == disclosure
                    && basis.field_hash.version > 0
                    && basis.field_hash.sha256.len() == 64
                    && basis
                        .field_hash
                        .sha256
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                    && !basis.provider_updated_at.is_empty(),
                "invalid withheld Jira observation"
            );
        }
    }
    let text = |value: &str| !value.trim().is_empty() && !value.contains('\0');
    for label in &observation.labels.items {
        ensure!(text(&label.id) && text(&label.name), "invalid Jira label");
    }
    for actor in observation
        .assignees
        .items
        .iter()
        .chain(observation.author.iter())
    {
        ensure!(text(&actor.id) && text(&actor.login), "invalid Jira actor");
    }
    for attribution in &observation.label_actors.items {
        ensure!(
            text(&attribution.label.id)
                && text(&attribution.label.name)
                && text(&attribution.event_id)
                && text(&attribution.created_at)
                && attribution
                    .actor
                    .as_ref()
                    .is_none_or(|actor| text(&actor.id) && text(&actor.login)),
            "invalid Jira label attribution"
        );
    }
    Ok(())
}
