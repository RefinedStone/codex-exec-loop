//! Typed, bounded provider reads. No arbitrary URL, GraphQL, header, or write proxy.
use crate::domain::provider_credentials::{ConnectionMetadata, CredentialResult, SecretToken};
use crate::domain::provider_work::{JiraSummary, LinearIssue, ProviderLinkRequest};

pub trait ProviderWorkApiPort: Send + Sync {
    fn jira_draft_ids(
        &self,
        _metadata: &ConnectionMetadata,
        _token: &SecretToken,
        _key: &str,
    ) -> CredentialResult<(String, String, String)> {
        Err(crate::domain::provider_credentials::ProviderCredentialError::Unsupported)
    }

    fn jira_summary(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
    ) -> CredentialResult<JiraSummary>;
    fn jira_issue_prompt(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
        key: &str,
    ) -> CredentialResult<String>;
    fn jira_current_version(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
        key: &str,
    ) -> CredentialResult<String>;
    fn linear_assigned_issues(&self, token: &SecretToken) -> CredentialResult<Vec<LinearIssue>>;
    fn linear_issue_prompt(&self, token: &SecretToken, id: &str) -> CredentialResult<String>;
    /// Read only the version for the canonical workspace/identifier ledger key.
    fn linear_current_version(
        &self,
        token: &SecretToken,
        resource_key: &str,
    ) -> CredentialResult<String>;
}
/// Only the native private channel can ask main to open a URL for the current connection.
pub trait ProviderLinkPort: Send + Sync {
    fn validate_link(&self, request: &ProviderLinkRequest) -> CredentialResult<String>;
}
