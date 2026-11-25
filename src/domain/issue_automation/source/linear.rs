//! Linear facts stay separate from the display Markdown and from execution authority.
use serde::{Deserialize, Serialize};

use super::IssueSourceFailure;
use crate::domain::issue_work::{IssueProvider, IssueReference};
use crate::domain::provider_credentials::{
    ConnectionRef, Provider, valid_connection_id, valid_profile_id, valid_revision,
};
use crate::domain::provider_work::{
    valid_linear_id, valid_linear_identifier, valid_linear_issue_url,
};

/// Linear has no tenant host per workspace. Team IDs are globally unique stable IDs.
pub const LINEAR_SERVER_ID: &str = "linear.app";
pub const MAX_LINEAR_BRANCH_NAME_BYTES: usize = 256;

/// `WorkflowState.type` values documented in Linear's public GraphQL schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinearIssueStateType {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
    Duplicate,
}

impl LinearIssueStateType {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "triage" => Self::Triage,
            "backlog" => Self::Backlog,
            "unstarted" => Self::Unstarted,
            "started" => Self::Started,
            "completed" => Self::Completed,
            "canceled" => Self::Canceled,
            "duplicate" => Self::Duplicate,
            _ => return None,
        })
    }

    pub fn closed(self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Duplicate)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourceLinearMetadata {
    /// Display key. Linear can change it when an issue moves; the stable ID stays `issue_id`.
    pub identifier: String,
    pub url: String,
    pub state_id: String,
    pub state_type: LinearIssueStateType,
    /// Provider suggestion only. It never names an owned slot, lease or existing worktree.
    /// An unsafe provider value is dropped; it never blocks the rest of the team.
    pub branch_name: Option<String>,
    pub connection: ConnectionRef,
}

impl IssueSourceLinearMetadata {
    pub fn validate_for(&self, issue: &IssueReference) -> Result<(), IssueSourceFailure> {
        if issue.provider != IssueProvider::Linear
            || issue.server_id != LINEAR_SERVER_ID
            || !valid_linear_id(&issue.repository_id)
            || !valid_linear_id(&issue.issue_id)
            || !valid_linear_identifier(&self.identifier)
            || canonical_linear_issue_url(&self.url, &self.identifier).as_deref()
                != Some(self.url.as_str())
            || !valid_linear_id(&self.state_id)
            || self
                .branch_name
                .as_deref()
                .is_some_and(|name| !valid_linear_branch_name(name))
            || self.connection.provider != Provider::Linear
            || !valid_profile_id(&self.connection.profile_id)
            || !valid_connection_id(&self.connection.connection_id)
            || !valid_revision(self.connection.revision)
            || self.connection.revision == 0
        {
            return Err(IssueSourceFailure::MalformedResponse);
        }
        Ok(())
    }
}

/// `https://linear.app/<workspace>/issue/<identifier>` without the mutable title slug,
/// query or fragment. The display link must name the same identifier.
pub fn canonical_linear_issue_url(url: &str, identifier: &str) -> Option<String> {
    if !valid_linear_issue_url(url) {
        return None;
    }
    let base = url.split(['?', '#']).next()?;
    let mut segments = base.strip_prefix("https://linear.app/")?.split('/');
    let workspace = segments.next()?;
    (segments.next()? == "issue" && segments.next()? == identifier)
        .then(|| format!("https://linear.app/{workspace}/issue/{identifier}"))
}

/// A Git-ref-safe subset. The branch allocator still sanitizes and prefixes it.
pub fn valid_linear_branch_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LINEAR_BRANCH_NAME_BYTES
        && !value.starts_with(['/', '-', '.'])
        && !value.ends_with(['/', '.'])
        && !value.contains("..")
        && !value.contains("//")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_./".contains(&byte))
}
