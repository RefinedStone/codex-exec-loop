//! Jira facts remain separate from the display Markdown and from execution authority.
use serde::{Deserialize, Serialize};

use super::IssueSourceFailure;
use crate::domain::issue_work::{IssueProvider, IssueReference};
use crate::domain::provider_credentials::{
    ConnectionRef, Provider, valid_connection_id, valid_profile_id, valid_revision,
};
use crate::domain::provider_work::valid_jira_key;
use crate::domain::work_context::normalize_jira_site;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSourceContentProblem {
    UnsupportedAdf,
    UnsupportedRequiredField,
    MissingRequiredField,
    OversizedContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JiraIssueStatusCategory {
    New,
    Indeterminate,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JiraPullRequestLookup {
    Available,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourceJiraMetadata {
    pub key: String,
    pub url: String,
    pub status_id: String,
    pub status_category: JiraIssueStatusCategory,
    pub pull_request_lookup: JiraPullRequestLookup,
    pub connection: ConnectionRef,
}

impl IssueSourceJiraMetadata {
    pub fn validate_for(&self, issue: &IssueReference) -> Result<(), IssueSourceFailure> {
        let site = self.url.strip_suffix(&format!("/browse/{}", self.key));
        if issue.provider != IssueProvider::Jira
            || !valid_jira_key(&self.key)
            || self.url.len() > 4096
            || site.is_none_or(|site| normalize_jira_site(site).as_deref() != Some(site))
            || !numeric_id(&self.status_id)
            || self.connection.provider != Provider::Jira
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

fn numeric_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|byte| byte.is_ascii_digit())
}
