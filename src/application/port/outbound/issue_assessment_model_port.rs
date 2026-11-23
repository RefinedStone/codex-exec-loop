//! One read-only assessment attempt. The host owns claims, budgets and decision authority.
use std::time::Duration;

use anyhow::Result;
use serde::Serialize;

pub const MAX_ISSUE_ASSESSMENT_INPUT_BYTES: usize = 256 * 1024;
pub const MAX_ISSUE_ASSESSMENT_REPLY_BYTES: usize = 64 * 1024;
pub const ISSUE_ASSESSMENT_PROFILE_VERSION: &str = "issue-assessment-v1";

/// Read from current host configuration, not from model output. The host combines these
/// ceilings with the issue policy before reserving the durable call budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAssessmentModelConfiguration {
    pub route_revision: String,
    pub model: Option<String>,
    pub max_running: u32,
    pub max_started_per_day: u32,
    pub timeout_seconds: u64,
    pub profile_version: String,
}

/// Contains only the bounded, disclosed input prepared by the host. This request cannot
/// select a workspace or expand tools; the adapter reuses the existing read-only profile.
/// Repository evidence is supplied as data, not by changing the backend's private cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAssessmentModelRequest {
    pub attempt_id: String,
    pub input_json: String,
    pub expected_route_revision: String,
    pub model: Option<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAssessmentModelFailure {
    Unavailable,
    Quota,
    Rejected,
    Interrupted,
    InvalidReply,
    Uncertain,
    ConfigurationChanged,
}

/// A complete transport response is still untrusted model text. The application must
/// validate its structure, apply privacy policy and recheck current decision authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAssessmentModelStatus {
    Pending,
    Complete { json: String },
    Unusable(IssueAssessmentModelFailure),
}

pub trait IssueAssessmentModelPort: Send + Sync {
    fn configuration(&self) -> Result<IssueAssessmentModelConfiguration>;

    /// Only the caller that just committed a new durable claim may dispatch. Returning
    /// Pending never waits for model completion; repeating observation spends no new call.
    fn dispatch(&self, request: IssueAssessmentModelRequest) -> Result<IssueAssessmentModelStatus>;

    /// An unknown attempt after restart is Unusable(Uncertain), never a redispatch request.
    fn observe(&self, attempt_id: &str) -> Result<IssueAssessmentModelStatus>;

    fn cancel(&self, attempt_id: &str) -> Result<()>;

    /// Forget only after the durable finish transaction succeeded. A failed store write
    /// must leave the same completed response available for the next observation.
    fn forget(&self, attempt_id: &str);
}
