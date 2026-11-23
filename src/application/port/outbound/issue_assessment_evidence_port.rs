//! Read a bounded selection from one immutable commit, never arbitrary workspace files.
use serde::Serialize;

use crate::domain::issue_automation::policy::IssueAutomationPolicy;

pub const ISSUE_ASSESSMENT_EVIDENCE_VERSION: &str = "issue-assessment-evidence-v1";
pub const MAX_ISSUE_ASSESSMENT_EVIDENCE_PATHS: usize = 16;
pub const MAX_ISSUE_ASSESSMENT_EVIDENCE_PATH_BYTES: usize = 1024;
pub const MAX_ISSUE_ASSESSMENT_EVIDENCE_FILE_BYTES: usize = 16 * 1024;
pub const MAX_ISSUE_ASSESSMENT_EVIDENCE_TOTAL_BYTES: usize = 64 * 1024;

pub struct IssueAssessmentEvidenceRequest<'a> {
    /// Host-only routing. It must not enter model input, stored evidence or error text.
    pub workspace_directory: &'a str,
    pub commit_sha: &'a str,
    pub selected_paths: &'a [String],
    pub policy: &'a IssueAutomationPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueAssessmentRepositoryEvidence {
    pub commit_sha: String,
    pub selection_version: String,
    pub snapshot_sha256: String,
    /// Every requested, permitted path is present exactly once, in stable path order.
    /// This is complete for that selection; it does not claim to cover the repository.
    pub entries: Vec<IssueAssessmentEvidenceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IssueAssessmentEvidenceEntry {
    File {
        path: String,
        blob_sha: String,
        text: String,
    },
    /// The exact tree proves the path does not exist. Read failures are never absence.
    Absent { path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAssessmentEvidenceError {
    NoEligiblePaths,
    ProtectedPath,
    UnsupportedEntry,
    TooLarge,
    InvalidText,
    Incomplete,
    SourceChanged,
    Unavailable,
}

impl std::fmt::Display for IssueAssessmentEvidenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::NoEligiblePaths => "no permitted assessment evidence paths",
            Self::ProtectedPath => "assessment evidence includes a protected path",
            Self::UnsupportedEntry => "assessment evidence has an unsupported entry",
            Self::TooLarge => "assessment evidence exceeds its bounds",
            Self::InvalidText => "assessment evidence is not shareable text",
            Self::Incomplete => "assessment evidence could not be read completely",
            Self::SourceChanged => "the assessment repository source changed",
            Self::Unavailable => "assessment repository evidence is unavailable",
        };
        formatter.write_str(reason)
    }
}

impl std::error::Error for IssueAssessmentEvidenceError {}

pub trait IssueAssessmentEvidencePort: Send + Sync {
    fn current_commit_sha(&self, workspace: &str) -> Result<String, IssueAssessmentEvidenceError>;

    fn read(
        &self,
        request: IssueAssessmentEvidenceRequest<'_>,
    ) -> Result<IssueAssessmentRepositoryEvidence, IssueAssessmentEvidenceError>;
}
