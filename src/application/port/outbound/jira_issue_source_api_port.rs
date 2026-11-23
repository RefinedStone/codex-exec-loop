//! Closed Jira Cloud reads; credentials and raw provider documents never enter public RPC.
use std::fmt;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use serde_json::Value;

use super::provider_credentials_port::ProviderCredentialLease;
use crate::domain::provider_credentials::CredentialResult;

#[derive(Clone, PartialEq, Eq)]
pub enum JiraIssueSourceApiRequest {
    TenantInfo,
    Project {
        project_id: String,
    },
    Search {
        jql: String,
        next_page_token: Option<String>,
        page_size: u16,
    },
    Issue {
        issue_id: String,
    },
    EditMetadata {
        issue_id: String,
    },
}

impl fmt::Debug for JiraIssueSourceApiRequest {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("JiraIssueSourceApiRequest(<redacted>)")
    }
}

pub struct JiraIssueSourceDocument(Value);

impl JiraIssueSourceDocument {
    pub fn new(value: Value) -> Self {
        Self(value)
    }
    pub fn into_value(self) -> Value {
        self.0
    }
}

impl fmt::Debug for JiraIssueSourceDocument {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("JiraIssueSourceDocument(<redacted>)")
    }
}

pub trait JiraIssueSourceApiPort: Send + Sync {
    fn read(
        &self,
        lease: &ProviderCredentialLease,
        request: &JiraIssueSourceApiRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<JiraIssueSourceDocument>;
}
