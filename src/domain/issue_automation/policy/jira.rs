use serde::{Deserialize, Serialize};

use super::{IssueAutomationAutonomy, IssueAutomationPolicyValueError};

/// No token, URL-derived remote identity, or workflow display name grants authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationJiraBinding {
    pub site_url: String,
    pub cloud_id: String,
    pub project_id: String,
    pub expected_account_id: String,
    /// A filter expression; the adapter owns the stable project and updated ordering.
    pub jql: String,
    pub mode: IssueAutomationAutonomy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_transition: Option<IssueAutomationJiraCloseTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationJiraCloseTransition {
    pub transition_id: String,
    pub from_status_ids: Vec<String>,
    pub to_status_id: String,
}

impl IssueAutomationJiraBinding {
    pub fn validate(&self) -> Result<(), IssueAutomationPolicyValueError> {
        let normalized = crate::domain::work_context::normalize_jira_site(&self.site_url);
        if self.site_url.len() > 2048 || normalized.as_deref() != Some(self.site_url.as_str()) {
            return Err(invalid(
                "binding.jira.site_url",
                "canonical_https_site_required",
            ));
        }
        if self.cloud_id.is_empty()
            || self.cloud_id.len() > 64
            || !self
                .cloud_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(invalid("binding.jira.cloud_id", "invalid_stable_cloud_id"));
        }
        numeric_id(&self.project_id, "binding.jira.project_id")?;
        if self.expected_account_id.is_empty()
            || self.expected_account_id.len() > 256
            || self.expected_account_id.trim() != self.expected_account_id
            || self.expected_account_id.chars().any(char::is_control)
        {
            return Err(invalid(
                "binding.jira.expected_account_id",
                "invalid_stable_account_id",
            ));
        }
        if self.jql.is_empty()
            || self.jql.len() > 4096
            || self.jql.trim() != self.jql
            || self.jql.chars().any(char::is_control)
            || self
                .jql
                .split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|words| {
                    words[0].eq_ignore_ascii_case("order") && words[1].eq_ignore_ascii_case("by")
                })
        {
            return Err(invalid(
                "binding.jira.jql",
                "bounded_filter_without_order_required",
            ));
        }
        if let Some(transition) = &self.close_transition {
            transition.validate()?;
        }
        Ok(())
    }
}

impl IssueAutomationJiraCloseTransition {
    pub fn validate(&self) -> Result<(), IssueAutomationPolicyValueError> {
        numeric_id(
            &self.transition_id,
            "binding.jira.close_transition.transition_id",
        )?;
        numeric_id(
            &self.to_status_id,
            "binding.jira.close_transition.to_status_id",
        )?;
        if self.from_status_ids.is_empty() || self.from_status_ids.len() > 32 {
            return Err(invalid(
                "binding.jira.close_transition.from_status_ids",
                "bounded_nonempty_status_set_required",
            ));
        }
        for status in &self.from_status_ids {
            numeric_id(status, "binding.jira.close_transition.from_status_ids")?;
            if status == &self.to_status_id {
                return Err(invalid(
                    "binding.jira.close_transition.from_status_ids",
                    "destination_must_differ",
                ));
            }
        }
        Ok(())
    }

    pub fn restricts(&self, granted: &Self) -> bool {
        self.transition_id == granted.transition_id
            && self.to_status_id == granted.to_status_id
            && self
                .from_status_ids
                .iter()
                .all(|id| granted.from_status_ids.contains(id))
    }
}

fn numeric_id(value: &str, field: &'static str) -> Result<(), IssueAutomationPolicyValueError> {
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(invalid(field, "invalid_numeric_id"));
    }
    Ok(())
}

fn invalid(field: &'static str, reason: &'static str) -> IssueAutomationPolicyValueError {
    IssueAutomationPolicyValueError::new(field, reason)
}
