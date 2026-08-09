use anyhow::Result;

use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};
use crate::domain::parallel_mode::{
    PostMergeValidationContract, PrValidationCheckContext, PrValidationCompletionSource,
};

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
    pub app_slug: Option<String>,
    pub check_suite_id: Option<GithubOpaqueId>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
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
            app_slug: None,
            check_suite_id: None,
            started_at: None,
            completed_at: None,
            target_sha,
            status,
        }
    }

    pub fn with_attempt_metadata(
        mut self,
        app_slug: Option<String>,
        check_suite_id: Option<GithubOpaqueId>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> Self {
        self.app_slug = app_slug;
        self.check_suite_id = check_suite_id;
        self.started_at = started_at;
        self.completed_at = completed_at;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubValidationWorkflowRun {
    pub id: GithubOpaqueId,
    pub name: String,
    pub target_sha: GithubCommitSha,
    pub status: GithubValidationRunStatus,
    pub run_attempt: u64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
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
            run_attempt: 1,
            created_at: None,
            updated_at: None,
        }
    }

    pub fn with_attempt_metadata(
        mut self,
        run_attempt: u64,
        created_at: Option<String>,
        updated_at: Option<String>,
    ) -> Self {
        self.run_attempt = run_attempt;
        self.created_at = created_at;
        self.updated_at = updated_at;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubExpectedCheckStatus {
    Missing,
    Pending,
    Succeeded,
    ActionableFailure,
    PolicyBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubExpectedCheckEvaluation {
    pub context: PrValidationCheckContext,
    pub status: GithubExpectedCheckStatus,
    pub selected_run: Option<GithubValidationCheckRun>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPostMergeValidationDecision {
    pub required: Vec<GithubExpectedCheckEvaluation>,
    pub optional: Vec<GithubExpectedCheckEvaluation>,
}

impl GithubPostMergeValidationDecision {
    pub fn is_successful(&self) -> bool {
        !self.required.is_empty()
            && self
                .required
                .iter()
                .all(|check| check.status == GithubExpectedCheckStatus::Succeeded)
    }

    pub fn has_policy_blocker(&self) -> bool {
        self.has_missing_required() || self.has_explicit_policy_blocker()
    }

    pub fn has_missing_required(&self) -> bool {
        self.required
            .iter()
            .any(|check| check.status == GithubExpectedCheckStatus::Missing)
    }

    pub fn has_explicit_policy_blocker(&self) -> bool {
        self.required
            .iter()
            .any(|check| check.status == GithubExpectedCheckStatus::PolicyBlocked)
    }

    pub fn actionable_failures(&self) -> impl Iterator<Item = &GithubExpectedCheckEvaluation> {
        self.required
            .iter()
            .filter(|check| check.status == GithubExpectedCheckStatus::ActionableFailure)
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
                .then_with(|| left.app_slug.cmp(&right.app_slug))
                .then_with(|| left.started_at.cmp(&right.started_at))
                .then_with(|| left.completed_at.cmp(&right.completed_at))
                .then_with(|| left.check_suite_id.cmp(&right.check_suite_id))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.workflow_runs.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.run_attempt.cmp(&right.run_attempt))
                .then_with(|| left.created_at.cmp(&right.created_at))
                .then_with(|| left.updated_at.cmp(&right.updated_at))
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

    pub fn evaluate_post_merge_contract(
        &self,
        contract: &PostMergeValidationContract,
    ) -> GithubPostMergeValidationDecision {
        match contract.completion_source() {
            PrValidationCompletionSource::CheckRuns => GithubPostMergeValidationDecision {
                required: contract
                    .required_check_contexts()
                    .iter()
                    .map(|context| {
                        self.evaluate_check_context(context, contract.latest_attempt_only())
                    })
                    .collect(),
                optional: contract
                    .optional_check_contexts()
                    .iter()
                    .map(|context| {
                        self.evaluate_check_context(context, contract.latest_attempt_only())
                    })
                    .collect(),
            },
        }
    }

    /// Reports explicit successful evidence against the record-owned contract. Workflow runs are
    /// diagnostic containers; they never duplicate or override check-context completion.
    pub fn is_successfully_complete(
        &self,
        contract: &PostMergeValidationContract,
        expected_evidence_sha: &GithubCommitSha,
    ) -> bool {
        let merge_identity_matches = match self.merge_state {
            GithubPrMergeState::Merged => self.merge_sha.as_ref() == Some(expected_evidence_sha),
            GithubPrMergeState::Open | GithubPrMergeState::Closed => true,
            GithubPrMergeState::Unknown(_) => false,
        };
        self.observations_complete()
            && merge_identity_matches
            && &self.evidence_sha == expected_evidence_sha
            && self
                .check_runs
                .iter()
                .all(|run| run.target_sha == self.evidence_sha)
            && self
                .workflow_runs
                .iter()
                .all(|run| run.target_sha == self.evidence_sha)
            && self.evaluate_post_merge_contract(contract).is_successful()
    }

    fn evaluate_check_context(
        &self,
        context: &PrValidationCheckContext,
        latest_attempt_only: bool,
    ) -> GithubExpectedCheckEvaluation {
        let mut matching = self
            .check_runs
            .iter()
            .filter(|run| context.matches(run.app_slug.as_deref(), &run.name))
            .collect::<Vec<_>>();
        matching.sort_by(|left, right| check_attempt_order(left, right));
        let selected_run = matching.last().copied().cloned();
        let statuses = if latest_attempt_only {
            matching.last().into_iter().copied().collect::<Vec<_>>()
        } else {
            matching
        };
        let status = if statuses.is_empty() {
            GithubExpectedCheckStatus::Missing
        } else if statuses.iter().any(|run| {
            matches!(
                run.status,
                GithubValidationRunStatus::Failed | GithubValidationRunStatus::Cancelled
            )
        }) {
            GithubExpectedCheckStatus::ActionableFailure
        } else if statuses.iter().any(|run| {
            matches!(
                run.status,
                GithubValidationRunStatus::Skipped | GithubValidationRunStatus::Unknown(_)
            )
        }) {
            GithubExpectedCheckStatus::PolicyBlocked
        } else if statuses.iter().any(|run| {
            matches!(
                run.status,
                GithubValidationRunStatus::Queued | GithubValidationRunStatus::InProgress
            )
        }) {
            GithubExpectedCheckStatus::Pending
        } else {
            GithubExpectedCheckStatus::Succeeded
        };
        GithubExpectedCheckEvaluation {
            context: context.clone(),
            status,
            selected_run,
        }
    }
}

fn check_attempt_order(
    left: &GithubValidationCheckRun,
    right: &GithubValidationCheckRun,
) -> std::cmp::Ordering {
    left.started_at
        .cmp(&right.started_at)
        .then_with(|| left.completed_at.cmp(&right.completed_at))
        .then_with(|| opaque_id_order(left.check_suite_id.as_ref(), right.check_suite_id.as_ref()))
        .then_with(|| opaque_id_order(Some(&left.id), Some(&right.id)))
}

fn opaque_id_order(
    left: Option<&GithubOpaqueId>,
    right: Option<&GithubOpaqueId>,
) -> std::cmp::Ordering {
    let numeric = |id: &GithubOpaqueId| {
        id.as_str()
            .rsplit_once(':')
            .and_then(|(_, suffix)| suffix.parse::<u128>().ok())
    };
    match (left, right) {
        (Some(left), Some(right)) => match (numeric(left), numeric(right)) {
            (Some(left), Some(right)) => left.cmp(&right),
            _ => left.cmp(right),
        },
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GithubExpectedCheckStatus, GithubPrMergeState, GithubPrValidationObservationRequest,
        GithubPrValidationPort, GithubPrValidationSnapshot, GithubValidationActivity,
        GithubValidationActivityKind, GithubValidationCheckRun, GithubValidationCursor,
        GithubValidationRunStatus, GithubValidationSource, GithubValidationSourceObservation,
        GithubValidationSourceStatus, GithubValidationWorkflowRun,
    };
    use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};
    use crate::domain::parallel_mode::PostMergeValidationContract;

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
                    "Post-Merge Gate",
                    target_sha.clone(),
                    GithubValidationRunStatus::Succeeded,
                )
                .with_attempt_metadata(
                    Some("github-actions".to_string()),
                    Some(GithubOpaqueId::new("check-suite:1")),
                    Some("2026-08-07T10:00:00Z".to_string()),
                    Some("2026-08-07T10:00:01Z".to_string()),
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
        assert!(snapshot.is_successfully_complete(
            &PostMergeValidationContract::production_v1(),
            &GithubCommitSha::new("head-sha")
        ));
        assert!(snapshot.observations_complete());
    }

    #[test]
    fn successful_completion_requires_explicit_ci_evidence() {
        let mut snapshot = complete_snapshot();
        snapshot.check_runs.clear();
        snapshot.workflow_runs.clear();

        assert!(snapshot.observations_complete());
        assert!(!snapshot.is_successfully_complete(
            &PostMergeValidationContract::production_v1(),
            &GithubCommitSha::new("head-sha")
        ));
    }

    #[test]
    fn closed_pull_request_completion_uses_the_attested_evidence_sha() {
        let mut snapshot = complete_snapshot();
        let evidence_sha = GithubCommitSha::new("distributor-evidence-sha");
        snapshot.merge_state = GithubPrMergeState::Closed;
        snapshot.evidence_sha = evidence_sha.clone();
        for run in &mut snapshot.check_runs {
            run.target_sha = evidence_sha.clone();
        }
        for run in &mut snapshot.workflow_runs {
            run.target_sha = evidence_sha.clone();
        }

        let contract = PostMergeValidationContract::production_v1();
        assert!(snapshot.is_successfully_complete(&contract, &evidence_sha));
        assert!(!snapshot.is_successfully_complete(&contract, &snapshot.target_sha));
    }

    #[test]
    fn unknown_or_paginated_status_is_incomplete() {
        let mut unknown = complete_snapshot();
        unknown.sources[0].status = GithubValidationSourceStatus::Unknown;
        assert!(!unknown.observations_complete());
        assert!(!unknown.is_successfully_complete(
            &PostMergeValidationContract::production_v1(),
            &GithubCommitSha::new("head-sha")
        ));

        let mut paginated = complete_snapshot();
        paginated.sources[0].status = GithubValidationSourceStatus::Paginated;
        paginated.sources[0].next_cursor = Some(GithubValidationCursor::new("source:page-2"));
        paginated.next_cursor = Some(GithubValidationCursor::new("snapshot:page-2"));
        assert!(!paginated.observations_complete());
        assert!(!paginated.is_successfully_complete(
            &PostMergeValidationContract::production_v1(),
            &GithubCommitSha::new("head-sha")
        ));

        let mut missing_source = complete_snapshot();
        missing_source.sources.pop();
        assert!(!missing_source.observations_complete());
    }

    #[test]
    fn latest_check_attempt_wins_independently_of_provider_order() {
        let target_sha = GithubCommitSha::new("head-sha");
        let attempt = |id: &str, started_at: &str, status| {
            GithubValidationCheckRun::new(
                GithubOpaqueId::new(id),
                "Post-Merge Gate",
                target_sha.clone(),
                status,
            )
            .with_attempt_metadata(
                Some("github-actions".to_string()),
                Some(GithubOpaqueId::new(format!("suite:{id}"))),
                Some(started_at.to_string()),
                Some(started_at.to_string()),
            )
        };
        let failed = attempt(
            "check:attempt-1",
            "2026-08-07T10:00:00Z",
            GithubValidationRunStatus::Failed,
        );
        let succeeded = attempt(
            "check:attempt-2",
            "2026-08-07T10:01:00Z",
            GithubValidationRunStatus::Succeeded,
        );
        let contract = PostMergeValidationContract::production_v1();

        let mut forward = complete_snapshot();
        forward.check_runs = vec![failed.clone(), succeeded.clone()];
        let mut reversed = forward.clone();
        reversed.check_runs.reverse();

        let forward_decision = forward.evaluate_post_merge_contract(&contract);
        let reversed_decision = reversed.evaluate_post_merge_contract(&contract);
        assert_eq!(forward_decision, reversed_decision);
        assert!(forward_decision.is_successful());
        assert_eq!(
            forward_decision.required[0]
                .selected_run
                .as_ref()
                .map(|run| run.id.as_str()),
            Some("check:attempt-2")
        );
    }

    #[test]
    fn numeric_provider_id_breaks_equal_timestamp_attempt_ties() {
        let target_sha = GithubCommitSha::new("head-sha");
        let attempt = |id: &str, status| {
            GithubValidationCheckRun::new(
                GithubOpaqueId::new(id),
                "Post-Merge Gate",
                target_sha.clone(),
                status,
            )
            .with_attempt_metadata(
                Some("github-actions".to_string()),
                Some(GithubOpaqueId::new("check-suite:1")),
                Some("2026-08-07T10:00:00Z".to_string()),
                Some("2026-08-07T10:00:00Z".to_string()),
            )
        };
        let mut snapshot = complete_snapshot();
        snapshot.check_runs = vec![
            attempt("check-run:10", GithubValidationRunStatus::Succeeded),
            attempt("check-run:9", GithubValidationRunStatus::Failed),
        ];

        let decision =
            snapshot.evaluate_post_merge_contract(&PostMergeValidationContract::production_v1());
        assert!(decision.is_successful());
        assert_eq!(
            decision.required[0]
                .selected_run
                .as_ref()
                .map(|run| run.id.as_str()),
            Some("check-run:10")
        );
    }

    #[test]
    fn required_missing_or_skipped_is_policy_blocked() {
        let contract = PostMergeValidationContract::production_v1();
        let mut missing = complete_snapshot();
        missing.check_runs.clear();
        let missing = missing.evaluate_post_merge_contract(&contract);
        assert!(missing.has_policy_blocker());
        assert!(!missing.is_successful());

        let mut skipped = complete_snapshot();
        skipped
            .check_runs
            .retain(|run| run.name == "Post-Merge Gate");
        skipped.check_runs[0].status = GithubValidationRunStatus::Skipped;
        let skipped = skipped.evaluate_post_merge_contract(&contract);
        assert!(skipped.has_policy_blocker());
        assert!(!skipped.is_successful());
    }

    #[test]
    fn optional_skips_and_workflow_failures_are_diagnostic_only() {
        let contract = PostMergeValidationContract::production_v1();
        let mut snapshot = complete_snapshot();
        snapshot.check_runs.push(
            GithubValidationCheckRun::new(
                GithubOpaqueId::new("check:optional-rust"),
                "Rust Tests",
                snapshot.evidence_sha.clone(),
                GithubValidationRunStatus::Skipped,
            )
            .with_attempt_metadata(
                Some("github-actions".to_string()),
                Some(GithubOpaqueId::new("suite:optional")),
                Some("2026-08-07T10:00:00Z".to_string()),
                Some("2026-08-07T10:00:00Z".to_string()),
            ),
        );
        snapshot.workflow_runs[0].status = GithubValidationRunStatus::Failed;

        let decision = snapshot.evaluate_post_merge_contract(&contract);
        assert!(decision.is_successful());
        assert_eq!(
            decision.optional[1].status,
            GithubExpectedCheckStatus::PolicyBlocked
        );
        assert!(snapshot.is_successfully_complete(&contract, &snapshot.evidence_sha));
    }
}
