//! Closed Jira Cloud feedback operations. No URL, authorization header or retry is supplied by a client.
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use serde_json::Value;

use super::provider_credential_mutation_port::ProviderMutationAuthorizationPort;
use super::provider_credentials_port::ProviderCredentialLease;
use crate::domain::issue_automation::feedback::JiraFeedbackRejection;
use crate::domain::provider_credentials::CredentialResult;

pub enum JiraIssueFeedbackReadRequest {
    TenantInfo,
    Actor,
    Issue {
        issue_id: String,
    },
    Comments {
        issue_id: String,
        start_at: u64,
    },
    Comment {
        issue_id: String,
        comment_id: String,
    },
    CommentProperty {
        issue_id: String,
        comment_id: String,
    },
    Transitions {
        issue_id: String,
    },
    Changelog {
        issue_id: String,
        start_at: u64,
    },
}

/// ADF and the matching non-secret intent property are constructed only by the Jira adapter.
pub enum JiraIssueFeedbackWriteRequest {
    CreateComment {
        issue_id: String,
        body: Value,
        property: Value,
    },
    UpdateComment {
        issue_id: String,
        comment_id: String,
        body: Value,
        property: Value,
    },
    Transition {
        issue_id: String,
        transition_id: String,
    },
}

pub struct JiraIssueFeedbackDocument(Value);
impl JiraIssueFeedbackDocument {
    pub fn new(value: Value) -> Self {
        Self(value)
    }
    pub fn into_value(self) -> Value {
        self.0
    }
}
impl std::fmt::Debug for JiraIssueFeedbackDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JiraIssueFeedbackDocument([redacted])")
    }
}

pub enum JiraIssueFeedbackWriteResponse {
    Comment(JiraIssueFeedbackDocument),
    /// Only a 204 response with an empty body establishes this acknowledgement.
    Transitioned,
    Rejected(JiraFeedbackRejection),
}

pub trait JiraIssueFeedbackApiPort: Send + Sync {
    fn read(
        &self,
        lease: &ProviderCredentialLease,
        request: &JiraIssueFeedbackReadRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<JiraIssueFeedbackDocument>;

    fn write(
        &self,
        lease: &ProviderCredentialLease,
        request: &JiraIssueFeedbackWriteRequest,
        authorization: Arc<dyn ProviderMutationAuthorizationPort>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<JiraIssueFeedbackWriteResponse>;
}
