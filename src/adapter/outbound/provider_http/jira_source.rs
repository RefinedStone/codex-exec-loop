//! Fixed Cloud source operations. No caller supplies a URL, header, field list, or write method.
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use serde_json::json;

use super::{JiraCloudRead, ProviderHttpAdapter};
use crate::application::port::outbound::jira_issue_source_api_port::{
    JiraIssueSourceApiPort, JiraIssueSourceApiRequest, JiraIssueSourceDocument,
};
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLease;
use crate::domain::provider_credentials::{CredentialResult, Provider, ProviderCredentialError};

const FIELDS: &[&str] = &[
    "summary",
    "description",
    "updated",
    "project",
    "issuetype",
    "status",
    "labels",
    "assignee",
    "creator",
    "priority",
];

impl JiraIssueSourceApiPort for ProviderHttpAdapter {
    fn read(
        &self,
        lease: &ProviderCredentialLease,
        request: &JiraIssueSourceApiRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<JiraIssueSourceDocument> {
        if lease.reference.provider != Provider::Jira {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let relative;
        let request = match request {
            JiraIssueSourceApiRequest::TenantInfo => JiraCloudRead::tenant_info(),
            JiraIssueSourceApiRequest::Project { project_id } => {
                numeric_id(project_id)?;
                relative = format!("project/{project_id}");
                JiraCloudRead::get_rest(&relative, vec![])
            }
            JiraIssueSourceApiRequest::Search {
                jql,
                next_page_token,
                page_size,
            } => {
                if jql.is_empty()
                    || jql.len() > 8192
                    || jql.chars().any(char::is_control)
                    || !(1..=100).contains(page_size)
                    || next_page_token.as_ref().is_some_and(|value| {
                        value.is_empty()
                            || value.len() > 2048
                            || value.chars().any(char::is_control)
                    })
                {
                    return Err(ProviderCredentialError::InvalidRequest);
                }
                let mut body = json!({"jql":jql,"maxResults":page_size,"fields":FIELDS});
                if let Some(token) = next_page_token {
                    body["nextPageToken"] = json!(token);
                }
                JiraCloudRead::post_rest("search/jql", body)
            }
            JiraIssueSourceApiRequest::Issue { issue_id } => {
                numeric_id(issue_id)?;
                relative = format!("issue/{issue_id}");
                JiraCloudRead::get_rest(&relative, vec![("fields".into(), FIELDS.join(","))])
            }
            JiraIssueSourceApiRequest::EditMetadata { issue_id } => {
                numeric_id(issue_id)?;
                relative = format!("issue/{issue_id}/editmeta");
                JiraCloudRead::get_rest(&relative, vec![])
            }
        };
        self.jira_cloud_read(&lease.metadata, &lease.token, request, cancelled, deadline)
            .map(JiraIssueSourceDocument::new)
    }
}

fn numeric_id(value: &str) -> CredentialResult<()> {
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
