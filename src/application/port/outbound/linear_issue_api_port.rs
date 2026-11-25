//! Closed Linear GraphQL operations for issue automation. A caller supplies typed values only;
//! the adapter owns the endpoint, query text, authorization header and retry policy.
use std::fmt;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use serde_json::Value;

use super::provider_credential_mutation_port::ProviderMutationAuthorizationPort;
use super::provider_credentials_port::ProviderCredentialLease;
use crate::domain::provider_credentials::CredentialResult;

#[derive(Clone, PartialEq, Eq)]
pub enum LinearIssueReadRequest {
    Viewer,
    Team {
        team_id: String,
    },
    /// Open issues of one team whose `updatedAt` is in `[updated_from, updated_before)`.
    Issues {
        team_id: String,
        updated_from: String,
        updated_before: String,
        after: Option<String>,
        page_size: u16,
    },
    Issue {
        issue_id: String,
    },
    Comments {
        issue_id: String,
        after: Option<String>,
    },
    Comment {
        comment_id: String,
    },
    History {
        issue_id: String,
        after: Option<String>,
    },
    WorkflowState {
        state_id: String,
    },
}

impl fmt::Debug for LinearIssueReadRequest {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("LinearIssueReadRequest(<redacted>)")
    }
}

/// The Markdown body is constructed only by the Linear feedback adapter.
pub enum LinearIssueWriteRequest {
    CreateComment { issue_id: String, body: String },
    UpdateComment { comment_id: String, body: String },
    UpdateState { issue_id: String, state_id: String },
}

/// The GraphQL `data` object of one response. Errors never become a document.
pub struct LinearIssueDocument(Value);

impl LinearIssueDocument {
    pub fn new(value: Value) -> Self {
        Self(value)
    }
    pub fn into_value(self) -> Value {
        self.0
    }
}

impl fmt::Debug for LinearIssueDocument {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("LinearIssueDocument(<redacted>)")
    }
}

pub trait LinearIssueApiPort: Send + Sync {
    fn read(
        &self,
        lease: &ProviderCredentialLease,
        request: &LinearIssueReadRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<LinearIssueDocument>;

    fn write(
        &self,
        lease: &ProviderCredentialLease,
        request: &LinearIssueWriteRequest,
        authorization: Arc<dyn ProviderMutationAuthorizationPort>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<LinearIssueDocument>;
}
