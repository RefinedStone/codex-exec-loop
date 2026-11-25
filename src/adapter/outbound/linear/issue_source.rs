//! Team-bound, credential-fenced Linear discovery and stable-ID resynchronization.
//! No credential input, raw provider document, comment, or remote write escapes this adapter.
use std::collections::BTreeSet;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::application::port::outbound::issue_source_port::*;
use crate::application::port::outbound::linear_issue_api_port::{
    LinearIssueApiPort, LinearIssueReadRequest,
};
use crate::application::port::outbound::provider_credentials_port::{
    ProviderCredentialLease, ProviderCredentialReadPort,
};
use crate::domain::issue_automation::policy::IssueAutomationLinearBinding;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_work::IssueProvider;
use crate::domain::provider_credentials::{ConnectionMetadata, Provider, ProviderCredentialError};

mod cursor;
mod observation;
#[cfg(test)]
mod tests;

type Clock = dyn Fn() -> u64 + Send + Sync;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(60);

pub struct LinearIssueSourceAdapter {
    binding: IssueAutomationLinearBinding,
    credentials: Arc<dyn ProviderCredentialReadPort>,
    api: Arc<dyn LinearIssueApiPort>,
    clock: Arc<Clock>,
}

impl LinearIssueSourceAdapter {
    /// Inert: composition may resolve current policy and create this adapter for each operation.
    pub fn new(
        binding: IssueAutomationLinearBinding,
        credentials: Arc<dyn ProviderCredentialReadPort>,
        api: Arc<dyn LinearIssueApiPort>,
    ) -> Result<Self, IssueSourceFailure> {
        binding
            .validate()
            .map_err(|_| IssueSourceFailure::InvalidRequest)?;
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
            .current_state(Provider::Linear)
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

    fn metadata(&self, metadata: &ConnectionMetadata) -> Result<(), IssueSourceFailure> {
        metadata.validate(Provider::Linear).map_err(failure)?;
        if metadata.account_id != self.binding.expected_viewer_id {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        Ok(())
    }

    fn read(
        &self,
        lease: &ProviderCredentialLease,
        request: &LinearIssueReadRequest,
        deadline: Instant,
    ) -> Result<Value, ProviderCredentialError> {
        if Instant::now() >= deadline {
            return Err(ProviderCredentialError::Cancelled);
        }
        self.credentials.validate_current(&lease.reference)?;
        let result = self
            .api
            .read(lease, request, &AtomicBool::new(false), deadline);
        // Also fences injected transports, not only native HTTP.
        self.credentials.validate_current(&lease.reference)?;
        if Instant::now() >= deadline {
            return Err(ProviderCredentialError::Cancelled);
        }
        result.map(|document| document.into_value())
    }

    fn repository(&self, repository: &IssueSourceRepository) -> Result<(), IssueSourceFailure> {
        repository.validate()?;
        if repository.provider != IssueProvider::Linear
            || repository.server_id != LINEAR_SERVER_ID
            || repository.repository_id != self.binding.team_id
        {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        Ok(())
    }

    /// The token's user and the bound team are confirmed by the provider on every operation.
    fn verify_account(
        &self,
        lease: &ProviderCredentialLease,
        deadline: Instant,
    ) -> Result<(), IssueSourceFailure> {
        let viewer = self
            .read(lease, &LinearIssueReadRequest::Viewer, deadline)
            .map_err(failure)?;
        if viewer["viewer"]["id"].as_str() != Some(self.binding.expected_viewer_id.as_str())
            || viewer["viewer"]["active"].as_bool() != Some(true)
        {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        let team = self
            .read(
                lease,
                &LinearIssueReadRequest::Team {
                    team_id: self.binding.team_id.clone(),
                },
                deadline,
            )
            .map_err(failure)?;
        if team["team"]["id"].as_str() != Some(self.binding.team_id.as_str()) {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        Ok(())
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
        self.verify_account(&lease, deadline)?;
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
        let search = |cursor: &cursor::Cursor| -> Result<Value, IssueSourceFailure> {
            let window = cursor.window()?;
            self.read(
                &lease,
                &LinearIssueReadRequest::Issues {
                    team_id: self.binding.team_id.clone(),
                    updated_from: window.from,
                    updated_before: window.before,
                    after: window.after,
                    page_size: request.page_size,
                },
                deadline,
            )
            .map_err(failure)
        };
        let continued = cursor.window()?.after.is_some();
        let value = match search(&cursor) {
            // A rejected continuation restarts the whole window. It never skips the page.
            Err(_) if continued => {
                cursor = cursor.restart_full(now);
                search(&cursor)?
            }
            result => result?,
        };
        let page = &value["issues"];
        let has_next_page = page["pageInfo"]["hasNextPage"]
            .as_bool()
            .ok_or(IssueSourceFailure::MalformedResponse)?;
        let end_cursor = match &page["pageInfo"]["endCursor"] {
            Value::Null => None,
            value => Some(observation::text(value, 2048)?),
        };
        let issues = page["nodes"]
            .as_array()
            .ok_or(IssueSourceFailure::MalformedResponse)?;
        if issues.len() > usize::from(request.page_size) {
            return Err(IssueSourceFailure::MalformedResponse);
        }
        let scanned_count = issues.len() as u16;
        let mut seen = BTreeSet::new();
        let mut observations = Vec::new();
        for value in issues {
            if !seen.insert(observation::stable_id(&value["id"])?) {
                return Err(IssueSourceFailure::MalformedResponse);
            }
            let current =
                observation::observation(value, repository, &lease.reference, &request.disclosure)?;
            if request.selection.matches_labels(&current.labels)? {
                observations.push(current);
            }
        }
        let opaque = cursor.finish_page(has_next_page, end_cursor)?;
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
            jira_connection: None,
            jira_binding: None,
            linear: Some(LinearDiscoveryOperation {
                connection: lease.reference,
                binding: self.binding.clone(),
            }),
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
        self.verify_account(&lease, deadline)?;
        let value = self
            .read(
                &lease,
                &LinearIssueReadRequest::Issue {
                    issue_id: request.issue.issue_id.clone(),
                },
                deadline,
            )
            .map_err(failure)?;
        let issue = &value["issue"];
        // `issue(id:)` also accepts an identifier; only the exact stable ID is the same issue.
        if observation::stable_id(&issue["id"])? != request.issue.issue_id {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        let result =
            observation::observation(issue, &repository, &lease.reference, &request.disclosure)?;
        self.credentials
            .validate_current(&lease.reference)
            .map_err(failure)?;
        Ok(result)
    }
}

impl IssueSourcePort for LinearIssueSourceAdapter {
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
