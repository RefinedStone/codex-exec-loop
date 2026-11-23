//! Work-panel reads use Gateway-owned credentials and fence every result against the active revision.
use crate::application::port::outbound::external_resource_version_port::ExternalResourceVersionPort;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialReadPort;
use crate::application::port::outbound::provider_work_port::{
    ProviderLinkPort, ProviderWorkApiPort,
};
use crate::domain::provider_credentials::*;
use crate::domain::provider_work::*;
use crate::domain::work_context::{ExternalResourceRef, jira_resource_site};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Default)]
struct LinearCache {
    connection: Option<ConnectionRef>,
    cached: Option<Arc<Vec<LinearIssue>>>,
    flight: Option<Arc<LinearFlight>>,
}
#[derive(Default)]
struct LinearFlight {
    result: Mutex<Option<CredentialResult<Arc<Vec<LinearIssue>>>>>,
    completed: Condvar,
}
pub struct ProviderWorkService {
    credentials: Arc<dyn ProviderCredentialReadPort>,
    api: Arc<dyn ProviderWorkApiPort>,
    linear: Mutex<LinearCache>,
}
impl ProviderWorkService {
    pub fn new(
        credentials: Arc<dyn ProviderCredentialReadPort>,
        api: Arc<dyn ProviderWorkApiPort>,
    ) -> Self {
        Self {
            credentials,
            api,
            linear: Mutex::new(LinearCache::default()),
        }
    }
    fn require_provider(reference: &ConnectionRef, provider: Provider) -> CredentialResult<()> {
        if reference.provider != provider {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        Ok(())
    }
    pub fn jira_summary(&self, reference: &ConnectionRef) -> CredentialResult<JiraSummary> {
        Self::require_provider(reference, Provider::Jira)?;
        let lease = self.credentials.acquire(reference)?;
        let result = self.api.jira_summary(&lease.metadata, &lease.token);
        self.credentials.validate_current(reference)?;
        result
    }
    pub fn jira_issue_prompt(
        &self,
        reference: &ConnectionRef,
        key: &str,
    ) -> CredentialResult<String> {
        Self::require_provider(reference, Provider::Jira)?;
        if !valid_jira_key(key) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let lease = self.credentials.acquire(reference)?;
        let result = self
            .api
            .jira_issue_prompt(&lease.metadata, &lease.token, key);
        self.credentials.validate_current(reference)?;
        result
    }
    pub fn jira_current_version(
        &self,
        reference: &ConnectionRef,
        resource: &ExternalResourceRef,
    ) -> CredentialResult<String> {
        Self::require_provider(reference, Provider::Jira)?;
        let state = self.credentials.current_state(Provider::Jira)?;
        let active = state
            .active_connection
            .ok_or(ProviderCredentialError::NotConnected)?;
        if active.reference != *reference {
            return Err(ProviderCredentialError::Conflict);
        }
        let site = active
            .metadata
            .configuration
            .site_url
            .as_deref()
            .ok_or(ProviderCredentialError::Corrupt)?;
        if jira_resource_site(resource).as_deref() != Some(site) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let key = jira_issue_key_for_link(site, &resource.url)
            .ok_or(ProviderCredentialError::InvalidRequest)?;
        let lease = self.credentials.acquire(reference)?;
        let result = self
            .api
            .jira_current_version(&lease.metadata, &lease.token, &key);
        self.credentials.validate_current(reference)?;
        result
    }
    pub fn linear_summary(
        &self,
        reference: &ConnectionRef,
        filters: &LinearFilters,
        refresh: bool,
    ) -> CredentialResult<LinearSummary> {
        Self::require_provider(reference, Provider::Linear)?;
        filters.validate()?;
        // State and cache hits never open the OS key store.
        self.credentials.validate_current(reference)?;
        let (flight, leader) = {
            let mut cache = self
                .linear
                .lock()
                .map_err(|_| ProviderCredentialError::Unavailable)?;
            if cache.connection.as_ref() != Some(reference) {
                *cache = LinearCache {
                    connection: Some(reference.clone()),
                    ..Default::default()
                };
            }
            if !refresh && let Some(all) = &cache.cached {
                let result = summarize(all, filters);
                drop(cache);
                self.credentials.validate_current(reference)?;
                return Ok(result);
            }
            match &cache.flight {
                Some(flight) => (flight.clone(), false),
                None => {
                    let flight = Arc::new(LinearFlight::default());
                    cache.flight = Some(flight.clone());
                    (flight, true)
                }
            }
        };
        if leader {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let lease = self.credentials.acquire(reference)?;
                let items = self.api.linear_assigned_issues(&lease.token)?;
                self.credentials.validate_current(reference)?;
                Ok(Arc::new(items))
            }))
            .unwrap_or(Err(ProviderCredentialError::Unavailable));
            // Publish only for the same active connection and same flight; a later connection owns its own cache.
            if let Ok(mut cache) = self.linear.lock()
                && cache.connection.as_ref() == Some(reference)
                && cache
                    .flight
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &flight))
            {
                if let Ok(items) = &result {
                    cache.cached = Some(items.clone());
                }
                cache.flight = None;
            }
            let mut answer = flight
                .result
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *answer = Some(result);
            flight.completed.notify_all();
        }
        let mut answer = flight
            .result
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        while answer.is_none() {
            answer = flight
                .completed
                .wait(answer)
                .map_err(|_| ProviderCredentialError::Unavailable)?;
        }
        let items = answer
            .as_ref()
            .ok_or(ProviderCredentialError::Unavailable)?
            .clone()?;
        drop(answer);
        self.credentials.validate_current(reference)?;
        Ok(summarize(&items, filters))
    }
    pub fn linear_issue_prompt(
        &self,
        reference: &ConnectionRef,
        id: &str,
    ) -> CredentialResult<String> {
        Self::require_provider(reference, Provider::Linear)?;
        if !valid_linear_id(id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let lease = self.credentials.acquire(reference)?;
        let result = self.api.linear_issue_prompt(&lease.token, id);
        self.credentials.validate_current(reference)?;
        result
    }
}
fn summarize(all: &[LinearIssue], selected: &LinearFilters) -> LinearSummary {
    let issues = all
        .iter()
        .filter(|item| {
            selected
                .team_id
                .as_ref()
                .is_none_or(|id| &item.team.id == id)
                && selected.project_id.as_ref().is_none_or(|id| {
                    item.project
                        .as_ref()
                        .is_some_and(|project| &project.id == id)
                })
                && (!selected.current_cycle
                    || item.cycle.as_ref().is_some_and(|cycle| cycle.is_active))
        })
        .cloned()
        .collect();
    let mut teams = Vec::new();
    let mut projects = Vec::new();
    for item in all {
        if !teams
            .iter()
            .any(|team: &NamedEntity| team.id == item.team.id)
        {
            teams.push(NamedEntity {
                id: item.team.id.clone(),
                name: item.team.name.clone(),
            });
        }
        if let Some(project) = &item.project
            && !projects
                .iter()
                .any(|value: &NamedEntity| value.id == project.id)
        {
            projects.push(project.clone());
        }
    }
    LinearSummary {
        issues,
        teams,
        projects,
        total_cached: all.len(),
        limit: ISSUE_LIMIT,
    }
}
impl ProviderLinkPort for ProviderWorkService {
    fn validate_link(&self, request: &ProviderLinkRequest) -> CredentialResult<String> {
        let state = self
            .credentials
            .current_state(request.connection.provider)?;
        let active = state
            .active_connection
            .ok_or(ProviderCredentialError::NotConnected)?;
        if active.reference != request.connection {
            return Err(ProviderCredentialError::Conflict);
        }
        let valid = match request.connection.provider {
            Provider::Jira => active
                .metadata
                .configuration
                .site_url
                .as_deref()
                .is_some_and(|site| jira_issue_key_for_link(site, &request.url).is_some()),
            Provider::Linear => valid_linear_issue_url(&request.url),
        };
        if !valid {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        self.credentials.validate_current(&request.connection)?;
        Ok(request.url.clone())
    }
}
impl ExternalResourceVersionPort for ProviderWorkService {
    fn kind(&self) -> &'static str {
        "jira"
    }
    fn available(&self, resource: &ExternalResourceRef) -> bool {
        let Some(site) = jira_resource_site(resource) else {
            return false;
        };
        self.credentials
            .current_state(Provider::Jira)
            .is_ok_and(|state| {
                state.active_connection.is_some_and(|active| {
                    active.metadata.configuration.site_url.as_deref() == Some(site.as_str())
                })
            })
    }
    fn current_version(&self, resource: &ExternalResourceRef) -> anyhow::Result<String> {
        let state = self.credentials.current_state(Provider::Jira)?;
        let active = state
            .active_connection
            .ok_or(ProviderCredentialError::NotConnected)?;
        self.jira_current_version(&active.reference, resource)
            .map_err(Into::into)
    }
}
#[cfg(test)]
mod tests;
