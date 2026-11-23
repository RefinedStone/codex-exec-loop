//! One stable remote issue belongs to one local binding within an authority profile.
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_work::IssueReference;
use anyhow::Result;

pub trait IssueSourceClaimPort: Send + Sync {
    /// A repeated claim by the same binding is idempotent. Another owner is never replaced.
    fn claim(&self, issue: &IssueReference, binding: &IssueAutomationBindingId) -> Result<bool>;
    /// Read-only confirmation; absence does not create a file or transfer ownership.
    fn owns(&self, issue: &IssueReference, binding: &IssueAutomationBindingId) -> Result<bool>;
}
