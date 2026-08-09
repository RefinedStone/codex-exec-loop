use anyhow::Result;

use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};

/// Read-only request for one PR revision. The cursor is adapter-opaque and can be replayed
/// without application code interpreting provider pagination details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPrValidationObservationRequest {
    pub target: GithubPullRequestTarget,
    pub target_sha: GithubCommitSha,
    /// Authority-attested evidence SHA for post-integration polls. GitHub merges must match the PR
    /// merge SHA; distributor integrations remain valid even while the PR is open or closed.
    pub evidence_sha: Option<GithubCommitSha>,
    pub cursor: Option<GithubValidationCursor>,
}

impl GithubPrValidationObservationRequest {
    pub fn new(
        target: GithubPullRequestTarget,
        target_sha: GithubCommitSha,
        cursor: Option<GithubValidationCursor>,
    ) -> Self {
        Self {
            target,
            target_sha,
            evidence_sha: None,
            cursor,
        }
    }

    pub fn with_evidence_sha(mut self, evidence_sha: Option<GithubCommitSha>) -> Self {
        self.evidence_sha = evidence_sha;
        self
    }
}

/// Application-owned outbound read boundary. Implementations own credentials, endpoint DTOs,
/// pagination, and response parsing; this contract intentionally exposes no write operations.
pub trait GithubPrValidationPort: Send + Sync {
    fn load_validation_snapshot(
        &self,
        request: &GithubPrValidationObservationRequest,
    ) -> Result<GithubPrValidationSnapshot>;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GithubValidationCursor(String);

impl GithubValidationCursor {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubPrMergeState {
    Open,
    Closed,
    Merged,
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GithubValidationSource {
    PullRequest,
    Reviews,
    IssueComments,
    ReviewThreads,
    CheckRuns,
    WorkflowRuns,
}

impl GithubValidationSource {
    pub const ALL: [Self; 6] = [
        Self::PullRequest,
        Self::Reviews,
        Self::IssueComments,
        Self::ReviewThreads,
        Self::CheckRuns,
        Self::WorkflowRuns,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubValidationSourceStatus {
    Complete,
    Paginated,
    Unknown,
}

/// Provenance for one normalized endpoint family. `endpoint` is a credential-free adapter label,
/// not a raw response body or request URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubValidationSourceObservation {
    pub source: GithubValidationSource,
    pub endpoint: String,
    pub status: GithubValidationSourceStatus,
    pub next_cursor: Option<GithubValidationCursor>,
}

impl GithubValidationSourceObservation {
    pub fn new(
        source: GithubValidationSource,
        endpoint: impl Into<String>,
        status: GithubValidationSourceStatus,
        next_cursor: Option<GithubValidationCursor>,
    ) -> Self {
        Self {
            source,
            endpoint: endpoint.into(),
            status,
            next_cursor,
        }
    }

    fn is_complete(&self) -> bool {
        self.status == GithubValidationSourceStatus::Complete && self.next_cursor.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GithubValidationActivityKind {
    Review,
    IssueComment,
    ReviewThread,
    ReviewComment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubValidationActivity {
    pub id: GithubOpaqueId,
    pub kind: GithubValidationActivityKind,
    pub observed_at: String,
    pub commit_sha: Option<GithubCommitSha>,
}

impl GithubValidationActivity {
    pub fn new(
        id: GithubOpaqueId,
        kind: GithubValidationActivityKind,
        observed_at: impl Into<String>,
    ) -> Self {
        Self {
            id,
            kind,
            observed_at: observed_at.into(),
            commit_sha: None,
        }
    }

    pub fn with_commit_sha(mut self, commit_sha: GithubCommitSha) -> Self {
        self.commit_sha = Some(commit_sha);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubValidationRunStatus {
    Queued,
    InProgress,
    Succeeded,
    Failed,
    Cancelled,
    Skipped,
    Unknown(String),
}

impl GithubValidationRunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Skipped
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubValidationCheckRun {
    pub id: GithubOpaqueId,
    pub name: String,
    pub target_sha: GithubCommitSha,
    pub status: GithubValidationRunStatus,
}

impl GithubValidationCheckRun {
    pub fn new(
        id: GithubOpaqueId,
        name: impl Into<String>,
        target_sha: GithubCommitSha,
        status: GithubValidationRunStatus,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            target_sha,
            status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubValidationWorkflowRun {
    pub id: GithubOpaqueId,
    pub name: String,
    pub target_sha: GithubCommitSha,
    pub status: GithubValidationRunStatus,
}

impl GithubValidationWorkflowRun {
    pub fn new(
        id: GithubOpaqueId,
        name: impl Into<String>,
        target_sha: GithubCommitSha,
        status: GithubValidationRunStatus,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            target_sha,
            status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPrValidationSnapshot {
    pub target: GithubPullRequestTarget,
    pub target_sha: GithubCommitSha,
    /// Immutable commit whose check and workflow evidence was actually observed. Before merge this
    /// is the PR head; after merge it must be GitHub's reported merge commit SHA.
    pub evidence_sha: GithubCommitSha,
    pub merge_state: GithubPrMergeState,
    pub merge_sha: Option<GithubCommitSha>,
    pub activities: Vec<GithubValidationActivity>,
    pub check_runs: Vec<GithubValidationCheckRun>,
    pub workflow_runs: Vec<GithubValidationWorkflowRun>,
    pub sources: Vec<GithubValidationSourceObservation>,
    pub next_cursor: Option<GithubValidationCursor>,
}

impl GithubPrValidationSnapshot {
    /// Canonicalizes independently paginated endpoint results without interpreting opaque IDs.
    pub fn normalize(&mut self) {
        self.activities.sort_by(|left, right| {
            left.observed_at
                .cmp(&right.observed_at)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.check_runs.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.workflow_runs.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.sources.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.endpoint.cmp(&right.endpoint))
                .then_with(|| left.next_cursor.cmp(&right.next_cursor))
        });
    }

    /// True only when every required source is represented by complete, unpaginated provenance.
    pub fn observations_complete(&self) -> bool {
        self.next_cursor.is_none()
            && self.sources.iter().all(|source| source.is_complete())
            && GithubValidationSource::ALL.into_iter().all(|required| {
                self.sources
                    .iter()
                    .any(|observation| observation.source == required)
            })
    }

    /// Reports explicit successful evidence only; failed, cancelled, skipped, unknown, and active
    /// runs cannot settle validation merely because some of them are terminal.
    pub fn is_successfully_complete(&self) -> bool {
        let expected_evidence_sha = match self.merge_state {
            GithubPrMergeState::Merged => self.merge_sha.as_ref(),
            GithubPrMergeState::Open | GithubPrMergeState::Closed => Some(&self.target_sha),
            GithubPrMergeState::Unknown(_) => None,
        };
        self.observations_complete()
            && expected_evidence_sha == Some(&self.evidence_sha)
            && (!self.check_runs.is_empty() || !self.workflow_runs.is_empty())
            && self.check_runs.iter().all(|run| {
                run.target_sha == self.evidence_sha
                    && run.status == GithubValidationRunStatus::Succeeded
            })
            && self.workflow_runs.iter().all(|run| {
                run.target_sha == self.evidence_sha
                    && run.status == GithubValidationRunStatus::Succeeded
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GithubPrMergeState, GithubPrValidationObservationRequest, GithubPrValidationPort,
        GithubPrValidationSnapshot, GithubValidationActivity, GithubValidationActivityKind,
        GithubValidationCheckRun, GithubValidationCursor, GithubValidationRunStatus,
        GithubValidationSource, GithubValidationSourceObservation, GithubValidationSourceStatus,
        GithubValidationWorkflowRun,
    };
    use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};

    struct FakeGithubPrValidationPort {
        snapshot: GithubPrValidationSnapshot,
    }

    impl GithubPrValidationPort for FakeGithubPrValidationPort {
        fn load_validation_snapshot(
            &self,
            _request: &GithubPrValidationObservationRequest,
        ) -> anyhow::Result<GithubPrValidationSnapshot> {
            let mut snapshot = self.snapshot.clone();
            snapshot.normalize();
            Ok(snapshot)
        }
    }

    fn complete_sources() -> Vec<GithubValidationSourceObservation> {
        GithubValidationSource::ALL
            .into_iter()
            .map(|source| {
                GithubValidationSourceObservation::new(
                    source,
                    format!("rest:{source:?}"),
                    GithubValidationSourceStatus::Complete,
                    None,
                )
            })
            .rev()
            .collect()
    }

    fn complete_snapshot() -> GithubPrValidationSnapshot {
        let target_sha = GithubCommitSha::new("head-sha");
        GithubPrValidationSnapshot {
            target: GithubPullRequestTarget::new("owner/repo", 42),
            target_sha: target_sha.clone(),
            evidence_sha: target_sha.clone(),
            merge_state: GithubPrMergeState::Open,
            merge_sha: None,
            activities: vec![
                GithubValidationActivity::new(
                    GithubOpaqueId::new("review:opaque-z"),
                    GithubValidationActivityKind::Review,
                    "2026-08-07T10:00:02Z",
                ),
                GithubValidationActivity::new(
                    GithubOpaqueId::new("thread:opaque-a"),
                    GithubValidationActivityKind::ReviewThread,
                    "2026-08-07T10:00:01Z",
                ),
            ],
            check_runs: vec![
                GithubValidationCheckRun::new(
                    GithubOpaqueId::new("check:9007199254740993"),
                    "z-check",
                    target_sha.clone(),
                    GithubValidationRunStatus::Succeeded,
                ),
                GithubValidationCheckRun::new(
                    GithubOpaqueId::new("check:not-a-number"),
                    "a-check",
                    target_sha.clone(),
                    GithubValidationRunStatus::Succeeded,
                ),
            ],
            workflow_runs: vec![GithubValidationWorkflowRun::new(
                GithubOpaqueId::new("workflow:opaque-1"),
                "ci",
                target_sha,
                GithubValidationRunStatus::Succeeded,
            )],
            sources: complete_sources(),
            next_cursor: None,
        }
    }

    #[test]
    fn complete_snapshot_is_stably_normalized() {
        let request = GithubPrValidationObservationRequest::new(
            GithubPullRequestTarget::new("owner/repo", 42),
            GithubCommitSha::new("head-sha"),
            Some(GithubValidationCursor::new("cursor:opaque-input")),
        );
        assert_eq!(request.target_sha.as_str(), "head-sha");
        assert_eq!(
            request.cursor.as_ref().map(GithubValidationCursor::as_str),
            Some("cursor:opaque-input")
        );

        let port = FakeGithubPrValidationPort {
            snapshot: complete_snapshot(),
        };
        let snapshot = port
            .load_validation_snapshot(&request)
            .expect("fake observation should succeed");

        assert_eq!(snapshot.target_sha.as_str(), "head-sha");
        assert_eq!(
            snapshot
                .activities
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread:opaque-a", "review:opaque-z"]
        );
        assert_eq!(
            snapshot
                .check_runs
                .iter()
                .map(|run| run.id.as_str())
                .collect::<Vec<_>>(),
            vec!["check:not-a-number", "check:9007199254740993"]
        );
        assert!(snapshot.is_successfully_complete());
        assert!(snapshot.observations_complete());
    }

    #[test]
    fn successful_completion_requires_explicit_ci_evidence() {
        let mut snapshot = complete_snapshot();
        snapshot.check_runs.clear();
        snapshot.workflow_runs.clear();

        assert!(snapshot.observations_complete());
        assert!(!snapshot.is_successfully_complete());
    }

    #[test]
    fn unknown_or_paginated_status_is_incomplete() {
        let mut unknown = complete_snapshot();
        unknown.sources[0].status = GithubValidationSourceStatus::Unknown;
        assert!(!unknown.observations_complete());
        assert!(!unknown.is_successfully_complete());

        let mut paginated = complete_snapshot();
        paginated.sources[0].status = GithubValidationSourceStatus::Paginated;
        paginated.sources[0].next_cursor = Some(GithubValidationCursor::new("source:page-2"));
        paginated.next_cursor = Some(GithubValidationCursor::new("snapshot:page-2"));
        assert!(!paginated.observations_complete());
        assert!(!paginated.is_successfully_complete());

        let mut missing_source = complete_snapshot();
        missing_source.sources.pop();
        assert!(!missing_source.observations_complete());
    }
}
