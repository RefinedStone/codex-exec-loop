//! Cloud-only, credential-fenced discovery and stable-ID resynchronization.
//! No credential input, raw provider document, comment, or remote write escapes this adapter.
use std::collections::BTreeSet;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::application::port::outbound::issue_source_port::*;
use crate::application::port::outbound::jira_issue_source_api_port::{
    JiraIssueSourceApiPort, JiraIssueSourceApiRequest,
};
use crate::application::port::outbound::provider_credentials_port::{
    ProviderCredentialLease, ProviderCredentialReadPort,
};
use crate::domain::issue_automation::policy::IssueAutomationJiraBinding;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_work::IssueProvider;
use crate::domain::provider_credentials::{JiraDeployment, Provider, ProviderCredentialError};

mod cursor;
mod observation;
#[cfg(test)]
mod tests;

type Clock = dyn Fn() -> u64 + Send + Sync;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(60);

pub struct JiraIssueSourceAdapter {
    binding: IssueAutomationJiraBinding,
    credentials: Arc<dyn ProviderCredentialReadPort>,
    api: Arc<dyn JiraIssueSourceApiPort>,
    clock: Arc<Clock>,
}

impl JiraIssueSourceAdapter {
    /// Inert: composition may resolve current policy and create this adapter for each operation.
    pub fn new(
        binding: IssueAutomationJiraBinding,
        credentials: Arc<dyn ProviderCredentialReadPort>,
        api: Arc<dyn JiraIssueSourceApiPort>,
    ) -> Result<Self, IssueSourceFailure> {
        binding
            .validate()
            .map_err(|_| IssueSourceFailure::InvalidRequest)?;
        cursor::validate_filter(&binding.jql)?;
        Ok(Self {
            binding,
            credentials,
            api,
            clock: Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .and_then(|time| time.as_millis().try_into().ok())
                    .unwrap_or(0)
            }),
        })
    }

    fn lease(&self) -> Result<ProviderCredentialLease, IssueSourceFailure> {
        let state = self
            .credentials
            .current_state(Provider::Jira)
            .map_err(failure)?;
        let active = state
            .active_connection
            .ok_or(IssueSourceFailure::ProviderUnavailable)?;
        self.metadata(&active.metadata)?;
        let lease = self
            .credentials
            .acquire(&active.reference)
            .map_err(failure)?;
        self.metadata(&lease.metadata)?;
        if lease.reference != active.reference {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        Ok(lease)
    }

    fn metadata(
        &self,
        metadata: &crate::domain::provider_credentials::ConnectionMetadata,
    ) -> Result<(), IssueSourceFailure> {
        metadata.validate(Provider::Jira).map_err(failure)?;
        if metadata.configuration.deployment != Some(JiraDeployment::Cloud)
            || metadata.configuration.site_url.as_deref() != Some(self.binding.site_url.as_str())
            || metadata.account_id != self.binding.expected_account_id
        {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        Ok(())
    }

    fn read(
        &self,
        lease: &ProviderCredentialLease,
        request: &JiraIssueSourceApiRequest,
        deadline: Instant,
    ) -> Result<Value, ProviderCredentialError> {
        if Instant::now() >= deadline {
            return Err(ProviderCredentialError::Cancelled);
        }
        self.credentials.validate_current(&lease.reference)?;
        let result = self
            .api
            .read(lease, request, &AtomicBool::new(false), deadline);
        // Also fences injected transports and the last editmeta read, not only native HTTP.
        self.credentials.validate_current(&lease.reference)?;
        if Instant::now() >= deadline {
            return Err(ProviderCredentialError::Cancelled);
        }
        result.map(|document| document.into_value())
    }

    fn repository(&self, repository: &IssueSourceRepository) -> Result<(), IssueSourceFailure> {
        repository.validate()?;
        if repository.provider != IssueProvider::Jira
            || repository.server_id != self.binding.cloud_id
            || repository.repository_id != self.binding.project_id
        {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        Ok(())
    }

    fn verify_cloud(
        &self,
        lease: &ProviderCredentialLease,
        deadline: Instant,
    ) -> Result<(), IssueSourceFailure> {
        let value = self
            .read(lease, &JiraIssueSourceApiRequest::TenantInfo, deadline)
            .map_err(failure)?;
        if value["cloudId"].as_str() != Some(self.binding.cloud_id.as_str()) {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        Ok(())
    }

    fn current_observation(
        &self,
        lease: &ProviderCredentialLease,
        value: Value,
        repository: &IssueSourceRepository,
        disclosure: &IssueSourceDisclosure,
        deadline: Instant,
    ) -> Result<IssueSourceObservation, IssueSourceFailure> {
        let issue_id = observation::numeric(&value["id"])?;
        let meta = self
            .read(
                lease,
                &JiraIssueSourceApiRequest::EditMetadata { issue_id },
                deadline,
            )
            .map_err(failure)?;
        observation::observation(
            value,
            meta,
            repository,
            &self.binding,
            &lease.reference,
            disclosure,
        )
    }

    fn discover_page(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        request.validate()?;
        let IssueDiscoveryTarget::Repository(repository) = &request.target else {
            return Err(IssueSourceFailure::UnsupportedProvider);
        };
        self.repository(repository)?;
        let lease = self.lease()?;
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        self.verify_cloud(&lease, deadline)?;
        let project = self
            .read(
                &lease,
                &JiraIssueSourceApiRequest::Project {
                    project_id: self.binding.project_id.clone(),
                },
                deadline,
            )
            .map_err(failure)?;
        if observation::numeric(&project["id"])? != self.binding.project_id {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        let raw = request
            .cursor
            .as_ref()
            .map(|cursor| {
                cursor.validate_for(
                    repository,
                    &request.selection,
                    request.page_size,
                    &request.disclosure,
                )
            })
            .transpose()?;
        let now = (self.clock)();
        let mut cursor = cursor::Cursor::begin(raw, &self.binding, &lease.reference, now)?;
        let search = |cursor: &cursor::Cursor| -> Result<Value, ProviderCredentialError> {
            let jql = cursor
                .query(&self.binding)
                .map_err(|_| ProviderCredentialError::InvalidRequest)?;
            self.read(
                &lease,
                &JiraIssueSourceApiRequest::Search {
                    jql,
                    next_page_token: cursor.token().map(str::to_owned),
                    page_size: request.page_size,
                },
                deadline,
            )
        };
        let value = match search(&cursor) {
            Err(ProviderCredentialError::InvalidRequest | ProviderCredentialError::NotFound)
                if cursor.token().is_some() =>
            {
                // An expired continuation is a new full read, never advancement past an unread page.
                cursor = cursor.restart_full(now);
                search(&cursor).map_err(failure)?
            }
            result => result.map_err(failure)?,
        };
        let is_last = value["isLast"]
            .as_bool()
            .ok_or(IssueSourceFailure::MalformedResponse)?;
        let token = match value.get("nextPageToken") {
            None | Some(Value::Null) => None,
            Some(value) => Some(observation::text(value, 2048)?),
        };
        let issues = value["issues"]
            .as_array()
            .ok_or(IssueSourceFailure::MalformedResponse)?;
        if issues.len() > usize::from(request.page_size) {
            return Err(IssueSourceFailure::MalformedResponse);
        }
        let scanned_count = issues.len() as u16;
        let mut seen = BTreeSet::new();
        let mut observations = Vec::new();
        for value in issues {
            if !seen.insert(observation::numeric(&value["id"])?) {
                return Err(IssueSourceFailure::MalformedResponse);
            }
            let current = self.current_observation(
                &lease,
                value.clone(),
                repository,
                &request.disclosure,
                deadline,
            )?;
            // Search indexing can lag; current closed results are still stored as closed, not discarded.
            if request.selection.matches_labels(&current.labels)? {
                observations.push(current);
            }
        }
        let opaque = cursor.finish_page(is_last, token)?;
        let next_cursor = Some(IssueDiscoveryCursor::new(
            repository.clone(),
            request.selection,
            request.page_size,
            &request.disclosure,
            opaque,
        )?);
        self.credentials
            .validate_current(&lease.reference)
            .map_err(failure)?;
        Ok(IssueDiscoveryPage {
            repository: repository.clone(),
            jira_connection: Some(lease.reference),
            jira_binding: Some(self.binding.clone()),
            linear: None,
            observations,
            next_cursor,
            scanned_count,
        })
    }

    fn read_issue(
        &self,
        request: &IssueResyncRequest,
    ) -> Result<IssueSourceObservation, IssueSourceFailure> {
        request.validate()?;
        let repository = IssueSourceRepository::from_reference(&request.issue);
        self.repository(&repository)?;
        let lease = self.lease()?;
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        self.verify_cloud(&lease, deadline)?;
        let value = self
            .read(
                &lease,
                &JiraIssueSourceApiRequest::Issue {
                    issue_id: request.issue.issue_id.clone(),
                },
                deadline,
            )
            .map_err(failure)?;
        if observation::numeric(&value["id"])? != request.issue.issue_id {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        let result =
            self.current_observation(&lease, value, &repository, &request.disclosure, deadline)?;
        self.credentials
            .validate_current(&lease.reference)
            .map_err(failure)?;
        Ok(result)
    }
}

impl IssueSourcePort for JiraIssueSourceAdapter {
    fn capabilities(&self) -> IssueSourceCapabilities {
        IssueSourceCapabilities {
            incremental: true,
            ..IssueSourceCapabilities::read_only()
        }
    }
    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        self.discover_page(request)
    }
    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        match self.read_issue(&request) {
            Ok(observation) => IssueSourceRead::Observed(Box::new(observation)),
            Err(reason) => IssueSourceRead::Unknown {
                issue: request.issue,
                reason,
            },
        }
    }
}

fn failure(error: ProviderCredentialError) -> IssueSourceFailure {
    match error {
        ProviderCredentialError::InvalidRequest => IssueSourceFailure::InvalidRequest,
        ProviderCredentialError::Corrupt | ProviderCredentialError::TooLarge => {
            IssueSourceFailure::MalformedResponse
        }
        _ => IssueSourceFailure::ProviderUnavailable,
    }
}
