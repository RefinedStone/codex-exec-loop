use serde::{Deserialize, Serialize};

use super::{IssueAutomationAutonomy, IssueAutomationPolicyValueError};
use crate::domain::provider_work::valid_linear_id;

/// No token, display key or workflow state name grants authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationLinearBinding {
    pub team_id: String,
    /// The connected user's stable `viewer.id`; a display name or email never replaces it.
    pub expected_viewer_id: String,
    pub mode: IssueAutomationAutonomy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_state: Option<IssueAutomationLinearCloseState>,
}

/// An explicit user grant: move from one of these workflow states to one completed state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationLinearCloseState {
    pub from_state_ids: Vec<String>,
    pub to_state_id: String,
}

impl IssueAutomationLinearBinding {
    pub fn validate(&self) -> Result<(), IssueAutomationPolicyValueError> {
        linear_id(&self.team_id, "binding.linear.team_id")?;
        linear_id(
            &self.expected_viewer_id,
            "binding.linear.expected_viewer_id",
        )?;
        if let Some(close) = &self.close_state {
            close.validate()?;
        }
        Ok(())
    }
}

impl IssueAutomationLinearCloseState {
    pub fn validate(&self) -> Result<(), IssueAutomationPolicyValueError> {
        linear_id(&self.to_state_id, "binding.linear.close_state.to_state_id")?;
        if self.from_state_ids.is_empty() || self.from_state_ids.len() > 32 {
            return Err(invalid(
                "binding.linear.close_state.from_state_ids",
                "bounded_nonempty_state_set_required",
            ));
        }
        for state in &self.from_state_ids {
            linear_id(state, "binding.linear.close_state.from_state_ids")?;
            if state == &self.to_state_id {
                return Err(invalid(
                    "binding.linear.close_state.from_state_ids",
                    "destination_must_differ",
                ));
            }
        }
        Ok(())
    }

    pub fn restricts(&self, granted: &Self) -> bool {
        self.to_state_id == granted.to_state_id
            && self
                .from_state_ids
                .iter()
                .all(|id| granted.from_state_ids.contains(id))
    }

    pub fn permits(&self, from_state_id: &str, to_state_id: &str) -> bool {
        self.to_state_id == to_state_id && self.from_state_ids.iter().any(|id| id == from_state_id)
    }
}

fn linear_id(value: &str, field: &'static str) -> Result<(), IssueAutomationPolicyValueError> {
    if !valid_linear_id(value) || value.len() > 64 {
        return Err(invalid(field, "invalid_stable_linear_id"));
    }
    Ok(())
}

fn invalid(field: &'static str, reason: &'static str) -> IssueAutomationPolicyValueError {
    IssueAutomationPolicyValueError::new(field, reason)
}
