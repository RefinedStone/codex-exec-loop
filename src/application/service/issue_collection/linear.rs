//! Linear issues have no repository. Only the team the repository file binds is collected,
//! and one stable issue belongs to one local binding within the authority profile.
use super::jira::{ConnectedSourceContext, binding_for_source, validate_observation};
use super::*;
use crate::application::port::outbound::issue_source_port::IssueDiscoveryPage;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialReadPort;
use crate::domain::issue_automation::policy::IssueAutomationLinearBinding;
use crate::domain::issue_automation::source::LINEAR_SERVER_ID;
use crate::domain::provider_credentials::{ConnectionRef, Provider, ProviderConnectionStatus};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;

impl IssueCollectionService {
    fn current_linear(
        &self,
        expected_version: &str,
    ) -> Result<(&ConnectedSourceContext, IssueAutomationLinearBinding)> {
        let context = self
            .connected
            .as_ref()
            .context("Linear collection is unavailable")?;
        let policy = context.policy.resolve(&self.workspace_dir)?;
        ensure!(
            policy.version == expected_version,
            "Linear selection changed before the collection commit"
        );
        ensure!(
            policy.policy.binding.jira.is_none(),
            "Linear binding is unavailable"
        );
        Ok((
            context,
            policy
                .policy
                .binding
                .linear
                .context("Linear binding is unavailable")?,
        ))
    }

    pub(super) fn claim_linear_page(
        &self,
        request: &IssueDiscoveryRequest,
        page: &mut IssueDiscoveryPage,
        projection: &IssueCollectionProjection,
        policy_version: &str,
    ) -> Result<()> {
        let (context, selected_binding) = self.current_linear(policy_version)?;
        let credentials = context
            .credentials
            .as_ref()
            .context("Linear credentials are unavailable")?;
        let operation = page
            .linear
            .as_ref()
            .context("Linear operation metadata is missing")?;
        ensure!(
            operation.binding == selected_binding
                && page.jira_binding.is_none()
                && page.jira_connection.is_none(),
            "Linear discovery used a different policy binding"
        );
        let connection = operation.connection.clone();
        validate_connection(credentials.as_ref(), &selected_binding, &connection)?;
        let IssueDiscoveryTarget::Repository(repository) = &request.target else {
            anyhow::bail!("Linear needs an explicit stable source");
        };
        ensure!(
            repository == &page.repository
                && repository.provider == IssueProvider::Linear
                && repository.server_id == LINEAR_SERVER_ID
                && repository.repository_id == selected_binding.team_id
                && repository.project_scope.workspace_dir == self.workspace_dir,
            "Linear discovery changed its bound team"
        );
        ensure!(
            page.scanned_count <= request.page_size
                && page.observations.len() <= usize::from(page.scanned_count),
            "Linear page exceeds its bound"
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
                "Linear page did not advance"
            );
        }
        let mut seen = BTreeSet::new();
        let mut owners = Vec::with_capacity(page.observations.len());
        // Validate the complete page before the first durable claim.
        for observation in &page.observations {
            ensure!(
                repository.matches_reference(&observation.issue)
                    && seen.insert(&observation.issue.issue_id),
                "Linear page contains duplicate or foreign identities"
            );
            let binding = binding_for_source(
                &self.workspace_dir,
                self.store.as_ref(),
                repository,
                Some(&observation.issue),
            )?;
            validate_observation(observation, &binding, &request.disclosure.policy_version)?;
            let linear = observation
                .linear
                .as_ref()
                .context("Linear source metadata is missing")?;
            ensure!(
                linear.connection == connection && observation.jira.is_none(),
                "Linear page mixed credential generations"
            );
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
        // Claim storage can block. Check the complete operation again immediately before
        // the caller's local page/cursor transaction, including empty pages.
        let (_, current_binding) = self.current_linear(policy_version)?;
        ensure!(
            operation.binding == current_binding,
            "Linear discovery binding changed before commit"
        );
        validate_connection(credentials.as_ref(), &current_binding, &connection)
    }

    pub(super) fn validate_linear_resync(
        &self,
        targets: &[IssueReference],
        reads: &mut [IssueSourceRead],
        policy_version: &str,
    ) -> Result<()> {
        let (context, binding) = self.current_linear(policy_version)?;
        let credentials = context
            .credentials
            .as_ref()
            .context("Linear credentials are unavailable")?;
        ensure!(targets.len() == reads.len(), "Linear resync batch changed");
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
                    && target.provider == IssueProvider::Linear
                    && target.server_id == LINEAR_SERVER_ID
                    && target.repository_id == binding.team_id
                    && context.claims.owns(target, owner.id())?
                    && observation.linear.as_ref().is_some_and(|linear| {
                        validate_connection(credentials.as_ref(), &binding, &linear.connection)
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
        let (_, current_binding) = self.current_linear(policy_version)?;
        for read in reads {
            if let IssueSourceRead::Observed(observation) = read {
                let connection = &observation
                    .linear
                    .as_ref()
                    .context("Linear source metadata is missing")?
                    .connection;
                validate_connection(credentials.as_ref(), &current_binding, connection)?;
            }
        }
        Ok(())
    }
}

fn validate_connection(
    credentials: &dyn ProviderCredentialReadPort,
    binding: &IssueAutomationLinearBinding,
    expected: &ConnectionRef,
) -> Result<()> {
    let state = credentials.current_state(Provider::Linear)?;
    ensure!(
        state.provider == Provider::Linear
            && state.profile_id == expected.profile_id
            && state.status == ProviderConnectionStatus::Connected,
        "Linear connection is unavailable"
    );
    let active = state
        .active_connection
        .context("Linear connection is unavailable")?;
    active.metadata.validate(Provider::Linear)?;
    ensure!(
        &active.reference == expected
            && expected.provider == Provider::Linear
            && active.metadata.account_id == binding.expected_viewer_id,
        "Linear connection no longer matches the selected binding"
    );
    credentials.validate_current(expected)?;
    Ok(())
}
