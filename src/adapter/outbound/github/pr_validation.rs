use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::review_poller::GithubReviewPollerAdapter;
use crate::application::port::outbound::github_pr_validation_port::{
    GithubPrMergeState, GithubPrValidationError, GithubPrValidationObservationRequest,
    GithubPrValidationPort, GithubPrValidationSnapshot, GithubValidationActivity,
    GithubValidationActivityKind, GithubValidationActor, GithubValidationActorKind,
    GithubValidationBodyMarker, GithubValidationCheckRun, GithubValidationCursor,
    GithubValidationProviderMetadata, GithubValidationReviewState, GithubValidationRunStatus,
    GithubValidationSource, GithubValidationSourceObservation, GithubValidationSourceStatus,
    GithubValidationWorkflowRun,
};
use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId};

const PER_PAGE: usize = 100;
const MAX_PAGINATED_PAGES: usize = 20;
const CURSOR_VERSION: u8 = 2;
const REVIEW_THREADS_GRAPHQL_OPERATION: &str = "AkraPullRequestReviewThreads";
const REVIEW_THREADS_GRAPHQL_QUERY: &str = r#"
query AkraPullRequestReviewThreads(
  $owner: String!
  $name: String!
  $number: Int!
  $first: Int!
  $after: String
) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: $first, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          isResolved
          comments(first: 1) {
            nodes {
              id
              body
              updatedAt
              author { login __typename }
              commit { oid }
            }
          }
        }
      }
    }
  }
}
"#;

/// Read-only GitHub adapter for one immutable PR revision.
///
/// Transport and credential handling stay delegated to the review poller's hardened GitHub client;
/// this adapter owns only validation endpoint DTOs, resumable pagination, and normalization.
pub struct GithubPrValidationAdapter {
    api: Box<dyn GithubValidationApi>,
}

impl GithubPrValidationAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            api: Box::new(GithubReviewPollerAdapter::new(token)),
        }
    }

    pub fn from_local_github_credentials(repo_root: &Path) -> Result<Self> {
        Ok(Self {
            api: Box::new(GithubReviewPollerAdapter::from_local_github_credentials(
                repo_root,
            )?),
        })
    }

    /// Defers credential discovery until the runtime has a durable validation record to poll.
    /// Production composition can therefore install validation at startup without making an
    /// unrelated missing credential fatal before parallel delivery is active.
    pub fn for_local_github_credentials(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            api: Box::new(LocalCredentialsGithubValidationApi {
                repo_root: repo_root.into(),
                poller: OnceLock::new(),
            }),
        }
    }
}

impl GithubPrValidationAdapter {
    #[cfg(test)]
    fn with_api(api: impl GithubValidationApi + 'static) -> Self {
        Self { api: Box::new(api) }
    }
}

trait GithubValidationApi: Send + Sync {
    fn get(
        &self,
        endpoint: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError>;

    fn post(
        &self,
        _endpoint: &str,
        _body: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        Err(GithubPrValidationError::integrity_failed(
            "GitHub validation API does not support bounded POST requests",
        ))
    }
}

#[derive(Debug)]
pub(super) struct GithubValidationApiResponse {
    pub(super) body: String,
    pub(super) metadata: GithubValidationProviderMetadata,
}

struct LocalCredentialsGithubValidationApi {
    repo_root: PathBuf,
    poller: OnceLock<Result<GithubReviewPollerAdapter, String>>,
}

impl GithubValidationApi for LocalCredentialsGithubValidationApi {
    fn get(
        &self,
        endpoint: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        let poller = self.poller.get_or_init(|| {
            GithubReviewPollerAdapter::from_local_github_credentials(&self.repo_root)
                .map_err(|error| error.to_string())
        });
        match poller {
            Ok(poller) => poller.fetch_validation_response(endpoint),
            Err(_) => Err(GithubPrValidationError::authentication_blocked(
                "GitHub validation credentials are unavailable",
            )),
        }
    }

    fn post(
        &self,
        endpoint: &str,
        body: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        if endpoint != "/graphql" {
            return Err(GithubPrValidationError::integrity_failed(
                "GitHub validation attempted an unsupported POST endpoint",
            ));
        }
        let poller = self.poller.get_or_init(|| {
            GithubReviewPollerAdapter::from_local_github_credentials(&self.repo_root)
                .map_err(|error| error.to_string())
        });
        match poller {
            Ok(poller) => poller.fetch_validation_graphql_response(body),
            Err(_) => Err(GithubPrValidationError::authentication_blocked(
                "GitHub validation credentials are unavailable",
            )),
        }
    }
}

impl GithubValidationApi for GithubReviewPollerAdapter {
    fn get(
        &self,
        endpoint: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        self.fetch_validation_response(endpoint)
    }

    fn post(
        &self,
        endpoint: &str,
        body: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        if endpoint != "/graphql" {
            return Err(GithubPrValidationError::integrity_failed(
                "GitHub validation attempted an unsupported POST endpoint",
            ));
        }
        self.fetch_validation_graphql_response(body)
    }
}

impl GithubPrValidationPort for GithubPrValidationAdapter {
    fn load_validation_snapshot(
        &self,
        request: &GithubPrValidationObservationRequest,
    ) -> std::result::Result<GithubPrValidationSnapshot, GithubPrValidationError> {
        validate_cursor_target(request)?;
        let mut provider_metadata = GithubValidationProviderMetadata::default();
        let repository = &request.target.repository;
        let number = request.target.number;

        // Head identity is checked before any activity or run endpoint. A moved branch must never
        // produce a snapshot labelled with the stale SHA supplied by the application.
        let pull_path = format!("/repos/{repository}/pulls/{number}");
        let pull: PullRequestResponse = self.get_json(&pull_path, &mut provider_metadata)?;
        ensure_commit_sha(&pull.head.sha, "PR head")?;
        if pull.head.sha != request.target_sha.as_str() {
            return Err(GithubPrValidationError::identity_failed(
                "GitHub PR head SHA changed before validation observation completed",
            ));
        }
        let initial_merge = normalize_merge_state(&pull)?;
        let reported_evidence_sha = initial_merge
            .1
            .as_ref()
            .unwrap_or(&request.target_sha)
            .clone();
        if initial_merge.0 == GithubPrMergeState::Merged
            && request
                .evidence_sha
                .as_ref()
                .is_some_and(|recorded| recorded != &reported_evidence_sha)
        {
            return Err(GithubPrValidationError::identity_failed(
                "GitHub PR merge SHA did not match the recorded post-merge evidence target",
            ));
        }
        let evidence_sha = request
            .evidence_sha
            .clone()
            .unwrap_or(reported_evidence_sha);
        let allow_cursor_evidence_reset =
            request.evidence_sha.is_none() && initial_merge.0 == GithubPrMergeState::Merged;
        let mut pages =
            ValidationPages::for_request(request, &evidence_sha, allow_cursor_evidence_reset)?;

        let mut activities = Vec::new();
        let mut check_runs = Vec::new();
        let mut workflow_runs = Vec::new();
        let mut source_states = BTreeMap::new();
        source_states.insert(
            GithubValidationSource::PullRequest,
            SourcePageState::Complete,
        );

        let review_page = pages.reviews.poll_page();
        let mut review_count = 0;
        for page in pages.reviews.pages_through_poll() {
            let path = paged_path(&format!("/repos/{repository}/pulls/{number}/reviews"), page);
            let rows: Vec<ReviewResponse> = self.get_json(&path, &mut provider_metadata)?;
            ensure_page_bound(&rows, &path)?;
            if page == review_page {
                review_count = rows.len();
            }
            for row in rows {
                if let Some(activity) = normalize_review(row)? {
                    activities.push(activity);
                }
            }
        }
        source_states.insert(
            GithubValidationSource::Reviews,
            advance_array_page(&mut pages.reviews, review_page, review_count)?,
        );

        let issue_comment_page = pages.issue_comments.poll_page();
        let mut issue_comment_count = 0;
        for page in pages.issue_comments.pages_through_poll() {
            let path = paged_path(
                &format!("/repos/{repository}/issues/{number}/comments"),
                page,
            );
            let rows: Vec<IssueCommentResponse> = self.get_json(&path, &mut provider_metadata)?;
            ensure_page_bound(&rows, &path)?;
            if page == issue_comment_page {
                issue_comment_count = rows.len();
            }
            for row in rows {
                activities.push(normalize_issue_comment(row));
            }
        }
        source_states.insert(
            GithubValidationSource::IssueComments,
            advance_array_page(
                &mut pages.issue_comments,
                issue_comment_page,
                issue_comment_count,
            )?,
        );

        let (repository_owner, repository_name) = repository_parts(repository)?;
        let review_thread_page = pages.review_threads.poll_page();
        let mut review_thread_observed_page = 0;
        let mut review_thread_has_next_page = false;
        let mut review_thread_after = None;
        for page in 1..=review_thread_page {
            let body = review_threads_graphql_body(
                repository_owner,
                repository_name,
                number,
                review_thread_after.as_deref(),
            )?;
            let response: ReviewThreadsGraphqlData =
                self.post_graphql_json(&body, &mut provider_metadata)?;
            let connection = response
                .repository
                .ok_or_else(|| {
                    GithubPrValidationError::authentication_blocked(
                        "GitHub review thread repository was unavailable",
                    )
                })?
                .pull_request
                .ok_or_else(|| {
                    GithubPrValidationError::identity_failed(
                        "GitHub review thread pull request was unavailable",
                    )
                })?
                .review_threads;
            ensure_page_bound(&connection.nodes, "graphql:review-threads")?;
            activities.extend(normalize_review_threads(connection.nodes)?);
            review_thread_observed_page = page;
            review_thread_has_next_page = connection.page_info.has_next_page;
            if !review_thread_has_next_page {
                break;
            }
            review_thread_after = Some(connection.page_info.end_cursor.ok_or_else(|| {
                GithubPrValidationError::integrity_failed(
                    "GitHub review thread page omitted its continuation cursor",
                )
            })?);
        }
        source_states.insert(
            GithubValidationSource::ReviewThreads,
            advance_connection_page(
                &mut pages.review_threads,
                review_thread_observed_page,
                review_thread_has_next_page,
            )?,
        );

        let check_run_page = pages.check_runs.poll_page();
        let mut check_run_count = 0;
        let mut check_run_total = None;
        for page in pages.check_runs.pages_through_poll() {
            let base = format!(
                "/repos/{repository}/commits/{}/check-runs?filter=all",
                evidence_sha.as_str()
            );
            let path = paged_query(&base, page);
            let response: CheckRunsResponse = self.get_json(&path, &mut provider_metadata)?;
            ensure_page_bound(&response.check_runs, &path)?;
            ensure_stable_total(&mut check_run_total, response.total_count)?;
            if page == check_run_page {
                check_run_count = response.check_runs.len();
            }
            for row in response.check_runs {
                ensure_run_sha(&row.head_sha, &evidence_sha)?;
                let app_slug = row
                    .app
                    .and_then(|app| non_empty_provider_text(&app.slug, 160));
                let check_suite_id = row.check_suite.map(|suite| suite.id.opaque("check-suite"));
                let started_at = normalize_provider_timestamp(row.started_at, "check start")?;
                let completed_at =
                    normalize_provider_timestamp(row.completed_at, "check completion")?;
                check_runs.push(
                    GithubValidationCheckRun::new(
                        row.id.opaque("check-run"),
                        sanitized_provider_text(&row.name, 160),
                        GithubCommitSha::new(row.head_sha),
                        normalize_run_status(&row.status, row.conclusion.as_deref()),
                    )
                    .with_attempt_metadata(
                        app_slug,
                        check_suite_id,
                        started_at,
                        completed_at,
                    ),
                );
            }
        }
        source_states.insert(
            GithubValidationSource::CheckRuns,
            advance_counted_page(
                &mut pages.check_runs,
                check_run_page,
                check_run_count,
                check_run_total.unwrap_or_default(),
            )?,
        );

        let workflow_run_page = pages.workflow_runs.poll_page();
        let mut workflow_run_count = 0;
        let mut workflow_run_total = None;
        for page in pages.workflow_runs.pages_through_poll() {
            let base = format!(
                "/repos/{repository}/actions/runs?head_sha={}",
                evidence_sha.as_str()
            );
            let path = paged_query(&base, page);
            let response: WorkflowRunsResponse = self.get_json(&path, &mut provider_metadata)?;
            ensure_page_bound(&response.workflow_runs, &path)?;
            ensure_stable_total(&mut workflow_run_total, response.total_count)?;
            if page == workflow_run_page {
                workflow_run_count = response.workflow_runs.len();
            }
            for row in response.workflow_runs {
                ensure_run_sha(&row.head_sha, &evidence_sha)?;
                if row.run_attempt == 0 {
                    return Err(GithubPrValidationError::integrity_failed(
                        "GitHub validation workflow run attempt must be positive",
                    ));
                }
                let check_suite_id = row.check_suite_id.map(|id| id.opaque("check-suite"));
                let run_started_at =
                    normalize_provider_timestamp(row.run_started_at, "workflow start")?;
                let created_at = normalize_provider_timestamp(row.created_at, "workflow creation")?;
                let updated_at = normalize_provider_timestamp(row.updated_at, "workflow update")?;
                workflow_runs.push(
                    GithubValidationWorkflowRun::new(
                        row.id.opaque("workflow-run"),
                        sanitized_provider_text(&row.name, 160),
                        GithubCommitSha::new(row.head_sha),
                        normalize_run_status(&row.status, row.conclusion.as_deref()),
                    )
                    .with_attempt_metadata(row.run_attempt, created_at, updated_at)
                    .with_attempt_correlation(check_suite_id, run_started_at),
                );
            }
        }
        source_states.insert(
            GithubValidationSource::WorkflowRuns,
            advance_counted_page(
                &mut pages.workflow_runs,
                workflow_run_page,
                workflow_run_count,
                workflow_run_total.unwrap_or_default(),
            )?,
        );

        // Re-read the PR after collecting its endpoint families. This closes the race where the
        // branch moves after the first identity check but before the snapshot is returned.
        let pull: PullRequestResponse = self.get_json(&pull_path, &mut provider_metadata)?;
        ensure_commit_sha(&pull.head.sha, "PR head")?;
        if pull.head.sha != request.target_sha.as_str() {
            return Err(GithubPrValidationError::identity_failed(
                "GitHub PR head SHA changed before validation observation completed",
            ));
        }
        let (merge_state, merge_sha) = normalize_merge_state(&pull)?;
        if (merge_state.clone(), merge_sha.clone()) != initial_merge {
            return Err(GithubPrValidationError::identity_failed(
                "GitHub PR merge identity changed before validation observation completed",
            ));
        }

        let next_cursor = pages.has_more().then(|| pages.to_cursor()).transpose()?;
        let sources = GithubValidationSource::ALL
            .into_iter()
            .map(|source| {
                let state = source_states
                    .get(&source)
                    .copied()
                    .unwrap_or(SourcePageState::Complete);
                let (status, cursor) = match state {
                    SourcePageState::Complete => (GithubValidationSourceStatus::Complete, None),
                    SourcePageState::Next(page) => (
                        GithubValidationSourceStatus::Paginated,
                        Some(GithubValidationCursor::new(format!(
                            "{}:{page}",
                            source_label(source)
                        ))),
                    ),
                };
                GithubValidationSourceObservation::new(
                    source,
                    source_endpoint_label(source),
                    status,
                    cursor,
                )
            })
            .collect();

        let mut snapshot = GithubPrValidationSnapshot {
            target: request.target.clone(),
            target_sha: request.target_sha.clone(),
            evidence_sha,
            merge_state,
            merge_sha,
            activities,
            check_runs,
            workflow_runs,
            sources,
            next_cursor,
            provider_metadata,
        };
        snapshot.normalize();
        Ok(snapshot)
    }
}

impl GithubPrValidationAdapter {
    fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: &str,
        provider_metadata: &mut GithubValidationProviderMetadata,
    ) -> std::result::Result<T, GithubPrValidationError> {
        let response = self.api.get(endpoint).map_err(|mut error| {
            error.provider_metadata.merge(provider_metadata);
            error
        })?;
        provider_metadata.merge(&response.metadata);
        serde_json::from_str(&response.body).map_err(|_| {
            GithubPrValidationError::integrity_failed(format!(
                "failed to parse GitHub validation response for {endpoint}"
            ))
            .with_provider_metadata(provider_metadata.clone())
        })
    }

    fn post_graphql_json<T: for<'de> Deserialize<'de>>(
        &self,
        body: &str,
        provider_metadata: &mut GithubValidationProviderMetadata,
    ) -> std::result::Result<T, GithubPrValidationError> {
        let response = self.api.post("/graphql", body).map_err(|mut error| {
            error.provider_metadata.merge(provider_metadata);
            error
        })?;
        provider_metadata.merge(&response.metadata);
        let envelope =
            serde_json::from_str::<GraphqlEnvelope<T>>(&response.body).map_err(|_| {
                GithubPrValidationError::integrity_failed(
                    "failed to parse GitHub validation GraphQL response",
                )
                .with_provider_metadata(provider_metadata.clone())
            })?;
        if !envelope.errors.is_empty() {
            return Err(GithubPrValidationError::authentication_blocked(
                "GitHub validation GraphQL query was rejected",
            )
            .with_provider_metadata(provider_metadata.clone()));
        }
        envelope.data.ok_or_else(|| {
            GithubPrValidationError::integrity_failed(
                "GitHub validation GraphQL response omitted its data envelope",
            )
            .with_provider_metadata(provider_metadata.clone())
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ValidationPages {
    version: u8,
    repository: String,
    number: u64,
    target_sha: String,
    evidence_sha: String,
    reviews: PageProgress,
    issue_comments: PageProgress,
    review_threads: PageProgress,
    check_runs: PageProgress,
    workflow_runs: PageProgress,
}

fn validate_cursor_target(request: &GithubPrValidationObservationRequest) -> Result<()> {
    let Some(cursor) = request.cursor.as_ref() else {
        return Ok(());
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor.as_str())
        .context("GitHub validation cursor was malformed")?;
    let pages: ValidationPages =
        serde_json::from_slice(&bytes).context("GitHub validation cursor payload was malformed")?;
    if pages.version != CURSOR_VERSION
        || pages.repository != request.target.repository
        || pages.number != request.target.number
        || pages.target_sha != request.target_sha.as_str()
    {
        bail!("GitHub validation cursor does not match the requested PR revision")
    }
    Ok(())
}

impl ValidationPages {
    fn initial(
        request: &GithubPrValidationObservationRequest,
        evidence_sha: &GithubCommitSha,
    ) -> Self {
        Self {
            version: CURSOR_VERSION,
            repository: request.target.repository.clone(),
            number: request.target.number,
            target_sha: request.target_sha.as_str().to_string(),
            evidence_sha: evidence_sha.as_str().to_string(),
            reviews: PageProgress::initial(),
            issue_comments: PageProgress::initial(),
            review_threads: PageProgress::initial(),
            check_runs: PageProgress::initial(),
            workflow_runs: PageProgress::initial(),
        }
    }

    fn for_request(
        request: &GithubPrValidationObservationRequest,
        evidence_sha: &GithubCommitSha,
        allow_evidence_reset: bool,
    ) -> Result<Self> {
        let Some(cursor) = request.cursor.as_ref() else {
            return Ok(Self::initial(request, evidence_sha));
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor.as_str())
            .context("GitHub validation cursor was malformed")?;
        let pages: Self = serde_json::from_slice(&bytes)
            .context("GitHub validation cursor payload was malformed")?;
        if pages.version != CURSOR_VERSION
            || pages.repository != request.target.repository
            || pages.number != request.target.number
            || pages.target_sha != request.target_sha.as_str()
        {
            bail!("GitHub validation cursor does not match the requested PR revision")
        }
        if pages.evidence_sha != evidence_sha.as_str() {
            if allow_evidence_reset {
                return Ok(Self::initial(request, evidence_sha));
            }
            bail!("GitHub validation cursor does not match the requested PR revision")
        }
        if [
            pages.reviews,
            pages.issue_comments,
            pages.review_threads,
            pages.check_runs,
            pages.workflow_runs,
        ]
        .into_iter()
        .any(|progress| !progress.is_valid())
        {
            bail!("GitHub validation cursor contained an invalid page")
        }
        Ok(pages)
    }

    fn has_more(&self) -> bool {
        self.reviews.next.is_some()
            || self.issue_comments.next.is_some()
            || self.review_threads.next.is_some()
            || self.check_runs.next.is_some()
            || self.workflow_runs.next.is_some()
    }

    fn to_cursor(&self) -> Result<GithubValidationCursor> {
        let bytes =
            serde_json::to_vec(self).context("failed to encode GitHub validation cursor")?;
        Ok(GithubValidationCursor::new(URL_SAFE_NO_PAD.encode(bytes)))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct PageProgress {
    observed_through: usize,
    next: Option<usize>,
}

impl PageProgress {
    fn initial() -> Self {
        Self {
            observed_through: 0,
            next: Some(1),
        }
    }

    fn poll_page(self) -> usize {
        self.next.unwrap_or(self.observed_through)
    }

    fn pages_through_poll(self) -> std::ops::RangeInclusive<usize> {
        1..=self.poll_page()
    }

    fn is_valid(self) -> bool {
        (1..=MAX_PAGINATED_PAGES).contains(&self.observed_through)
            && self
                .next
                .is_none_or(|page| page == self.observed_through + 1 && page <= MAX_PAGINATED_PAGES)
    }
}

#[derive(Clone, Copy)]
enum SourcePageState {
    Complete,
    Next(usize),
}

fn advance_array_page(
    progress: &mut PageProgress,
    page: usize,
    count: usize,
) -> Result<SourcePageState> {
    progress.observed_through = page;
    if count == PER_PAGE {
        let next = next_page(page)?;
        progress.next = Some(next);
        Ok(SourcePageState::Next(next))
    } else {
        progress.next = None;
        Ok(SourcePageState::Complete)
    }
}

fn advance_connection_page(
    progress: &mut PageProgress,
    page: usize,
    has_next_page: bool,
) -> Result<SourcePageState> {
    if page == 0 {
        bail!("GitHub validation connection returned no observable page")
    }
    progress.observed_through = page;
    if has_next_page {
        let next = next_page(page)?;
        progress.next = Some(next);
        Ok(SourcePageState::Next(next))
    } else {
        progress.next = None;
        Ok(SourcePageState::Complete)
    }
}

fn advance_counted_page(
    progress: &mut PageProgress,
    page: usize,
    count: usize,
    total_count: usize,
) -> Result<SourcePageState> {
    let observed_through = page
        .checked_sub(1)
        .and_then(|page_index| page_index.checked_mul(PER_PAGE))
        .and_then(|offset| offset.checked_add(count))
        .ok_or_else(|| anyhow!("GitHub validation pagination count overflowed"))?;
    progress.observed_through = page;
    if observed_through < total_count {
        if count != PER_PAGE {
            bail!("GitHub validation counted response ended before its declared total")
        }
        let next = next_page(page)?;
        progress.next = Some(next);
        Ok(SourcePageState::Next(next))
    } else {
        progress.next = None;
        Ok(SourcePageState::Complete)
    }
}

fn next_page(page: usize) -> Result<usize> {
    if page >= MAX_PAGINATED_PAGES {
        bail!("GitHub validation pagination remained full after {MAX_PAGINATED_PAGES} pages")
    }
    page.checked_add(1)
        .ok_or_else(|| anyhow!("GitHub validation pagination page overflowed"))
}

fn ensure_page_bound<T>(rows: &[T], endpoint: &str) -> Result<()> {
    if rows.len() > PER_PAGE {
        bail!("GitHub validation response exceeded the {PER_PAGE}-item page bound for {endpoint}")
    }
    Ok(())
}

fn ensure_stable_total(observed: &mut Option<usize>, current: usize) -> Result<()> {
    if observed.is_some_and(|expected| expected != current) {
        bail!("GitHub validation pagination total changed during cumulative observation")
    }
    *observed = Some(current);
    Ok(())
}

fn ensure_commit_sha(value: &str, label: &str) -> Result<()> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("GitHub validation {label} SHA was malformed")
    }
    Ok(())
}

fn paged_path(base: &str, page: usize) -> String {
    format!("{base}?per_page={PER_PAGE}&page={page}")
}

fn paged_query(base: &str, page: usize) -> String {
    format!("{base}&per_page={PER_PAGE}&page={page}")
}

fn repository_parts(repository: &str) -> Result<(&str, &str), GithubPrValidationError> {
    let Some((owner, name)) = repository.split_once('/') else {
        return Err(GithubPrValidationError::identity_failed(
            "GitHub validation repository identity was malformed",
        ));
    };
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(GithubPrValidationError::identity_failed(
            "GitHub validation repository identity was malformed",
        ));
    }
    Ok((owner, name))
}

fn review_threads_graphql_body(
    owner: &str,
    name: &str,
    number: u64,
    after: Option<&str>,
) -> Result<String, GithubPrValidationError> {
    let number = i64::try_from(number).map_err(|_| {
        GithubPrValidationError::identity_failed(
            "GitHub validation pull request number exceeded GraphQL range",
        )
    })?;
    serde_json::to_string(&serde_json::json!({
        "operationName": REVIEW_THREADS_GRAPHQL_OPERATION,
        "query": REVIEW_THREADS_GRAPHQL_QUERY,
        "variables": {
            "owner": owner,
            "name": name,
            "number": number,
            "first": PER_PAGE,
            "after": after,
        }
    }))
    .map_err(|_| {
        GithubPrValidationError::integrity_failed(
            "failed to encode GitHub validation GraphQL request",
        )
    })
}

fn source_label(source: GithubValidationSource) -> &'static str {
    match source {
        GithubValidationSource::PullRequest => "pull-request",
        GithubValidationSource::Reviews => "reviews",
        GithubValidationSource::IssueComments => "issue-comments",
        GithubValidationSource::ReviewThreads => "review-threads",
        GithubValidationSource::CheckRuns => "check-runs",
        GithubValidationSource::WorkflowRuns => "workflow-runs",
    }
}

fn normalize_merge_state(
    response: &PullRequestResponse,
) -> Result<(GithubPrMergeState, Option<GithubCommitSha>)> {
    if response.merged {
        let merge_sha = response
            .merge_commit_sha
            .as_deref()
            .ok_or_else(|| anyhow!("merged GitHub PR response omitted its merge SHA"))?;
        ensure_commit_sha(merge_sha, "PR merge")?;
        return Ok((
            GithubPrMergeState::Merged,
            Some(GithubCommitSha::new(merge_sha)),
        ));
    }
    let state = match response.state.as_str() {
        "open" => GithubPrMergeState::Open,
        "closed" => GithubPrMergeState::Closed,
        other => GithubPrMergeState::Unknown(sanitized_provider_text(other, 64)),
    };
    Ok((state, None))
}

fn normalize_review(
    row: ReviewResponse,
) -> std::result::Result<Option<GithubValidationActivity>, GithubPrValidationError> {
    let Some(observed_at) = normalize_provider_timestamp(row.submitted_at, "review submission")
        .map_err(|error| GithubPrValidationError::integrity_failed(error.to_string()))?
    else {
        return Ok(None);
    };
    ensure_commit_sha(&row.commit_id, "review commit")
        .map_err(|error| GithubPrValidationError::integrity_failed(error.to_string()))?;
    Ok(Some(
        GithubValidationActivity::new(
            row.id.opaque("review"),
            GithubValidationActivityKind::Review,
            observed_at,
        )
        .with_commit_sha(GithubCommitSha::new(row.commit_id))
        .with_review_state(normalize_review_state(&row.state))
        .with_actor(normalize_actor(row.user))
        .with_body_marker(explicit_body_marker(
            row.body.as_deref().unwrap_or_default(),
        )),
    ))
}

fn normalize_issue_comment(row: IssueCommentResponse) -> GithubValidationActivity {
    GithubValidationActivity::new(
        row.id.opaque("issue-comment"),
        GithubValidationActivityKind::IssueComment,
        sanitized_provider_text(&row.updated_at, 64),
    )
    .with_actor(normalize_actor(row.user))
    .with_body_marker(explicit_body_marker(&row.body))
}

fn normalize_review_threads(
    rows: Vec<Option<ReviewThreadGraphqlNode>>,
) -> std::result::Result<Vec<GithubValidationActivity>, GithubPrValidationError> {
    let mut activities = Vec::new();
    for row in rows.into_iter().flatten() {
        let root = row
            .comments
            .nodes
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| {
                GithubPrValidationError::integrity_failed(
                    "GitHub review thread omitted its root comment",
                )
            })?;
        let commit = root.commit.ok_or_else(|| {
            GithubPrValidationError::integrity_failed(
                "GitHub review thread root omitted its commit identity",
            )
        })?;
        ensure_commit_sha(&commit.oid, "review thread commit")
            .map_err(|error| GithubPrValidationError::integrity_failed(error.to_string()))?;
        let observed_at =
            normalize_provider_timestamp(Some(root.updated_at), "review thread root update")
                .map_err(|error| GithubPrValidationError::integrity_failed(error.to_string()))?
                .expect("review thread update was supplied");
        activities.push(
            GithubValidationActivity::new(
                row.id.opaque("review-thread"),
                GithubValidationActivityKind::ReviewThread,
                observed_at,
            )
            .with_commit_sha(GithubCommitSha::new(commit.oid))
            .with_actor(normalize_graphql_actor(root.author))
            .with_body_marker(explicit_body_marker(&root.body))
            .with_thread_resolved(row.is_resolved),
        );
    }
    Ok(activities)
}

fn normalize_review_state(state: &str) -> GithubValidationReviewState {
    match state.trim().to_ascii_uppercase().as_str() {
        "APPROVED" => GithubValidationReviewState::Approved,
        "CHANGES_REQUESTED" => GithubValidationReviewState::ChangesRequested,
        "COMMENTED" => GithubValidationReviewState::Commented,
        "DISMISSED" => GithubValidationReviewState::Dismissed,
        "PENDING" => GithubValidationReviewState::Pending,
        _ => GithubValidationReviewState::Unknown(sanitized_provider_text(state, 64)),
    }
}

fn source_endpoint_label(source: GithubValidationSource) -> String {
    let transport = if source == GithubValidationSource::ReviewThreads {
        "graphql"
    } else {
        "rest"
    };
    format!("{transport}:{}", source_label(source))
}

fn normalize_actor(actor: Option<ValidationActorResponse>) -> Option<GithubValidationActor> {
    let actor = actor?;
    normalized_actor(&actor.login, &actor.kind)
}

fn normalize_graphql_actor(actor: Option<GraphqlActorResponse>) -> Option<GithubValidationActor> {
    let actor = actor?;
    normalized_actor(&actor.login, &actor.typename)
}

fn normalized_actor(login: &str, kind: &str) -> Option<GithubValidationActor> {
    let login = non_empty_provider_text(login, 160)?;
    let kind = match kind.trim().to_ascii_lowercase().as_str() {
        "user" => GithubValidationActorKind::User,
        "bot" => GithubValidationActorKind::Bot,
        "organization" => GithubValidationActorKind::Organization,
        "mannequin" => GithubValidationActorKind::Mannequin,
        _ => GithubValidationActorKind::Unknown,
    };
    Some(GithubValidationActor::new(login, kind))
}

fn explicit_body_marker(body: &str) -> GithubValidationBodyMarker {
    let mut fenced = false;
    for raw_line in body.lines() {
        if !fenced && (raw_line.starts_with("    ") || raw_line.starts_with('\t')) {
            continue;
        }
        let line = raw_line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.starts_with('>') {
            continue;
        }
        let line = line.to_ascii_lowercase();
        if explicit_command_line(&line, "@akra fix") {
            return GithubValidationBodyMarker::AkraFix;
        }
        if explicit_command_line(&line, "/akra remediate") {
            return GithubValidationBodyMarker::AkraRemediate;
        }
    }
    GithubValidationBodyMarker::None
}

fn explicit_command_line(line: &str, command: &str) -> bool {
    line == command
        || line
            .strip_prefix(command)
            .and_then(|suffix| suffix.chars().next())
            .is_some_and(char::is_whitespace)
}

fn ensure_run_sha(
    observed: &str,
    expected: &GithubCommitSha,
) -> std::result::Result<(), GithubPrValidationError> {
    ensure_commit_sha(observed, "validation run")
        .map_err(|error| GithubPrValidationError::integrity_failed(error.to_string()))?;
    if observed != expected.as_str() {
        return Err(GithubPrValidationError::identity_failed(
            "GitHub validation run was not bound to the requested target SHA",
        ));
    }
    Ok(())
}

fn sanitized_provider_id(value: &str) -> String {
    const MAX_ID_LEN: usize = 96;
    const DIGEST_LEN: usize = 16;

    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if !sanitized.is_empty() && sanitized == value && sanitized.chars().count() <= MAX_ID_LEN {
        return sanitized;
    }
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    let prefix = sanitized
        .chars()
        .take(MAX_ID_LEN - DIGEST_LEN - 1)
        .collect::<String>();
    format!("{prefix}-{}", &digest[..DIGEST_LEN])
}

fn sanitized_provider_text(value: &str, max_len: usize) -> String {
    let mut sanitized = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_control() || character.is_whitespace() {
            pending_space = !sanitized.is_empty();
            continue;
        }
        if pending_space {
            sanitized.push(' ');
            pending_space = false;
        }
        sanitized.push(character);
    }
    if sanitized.chars().count() <= max_len {
        return sanitized;
    }
    if max_len <= 3 {
        return ".".repeat(max_len);
    }
    let mut bounded = sanitized.chars().take(max_len - 3).collect::<String>();
    while bounded.ends_with(' ') {
        bounded.pop();
    }
    bounded.push_str("...");
    bounded
}

fn non_empty_provider_text(value: &str, max_len: usize) -> Option<String> {
    let value = sanitized_provider_text(value, max_len);
    (!value.is_empty()).then_some(value)
}

fn normalize_provider_timestamp(value: Option<String>, label: &str) -> Result<Option<String>> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .with_context(|| format!("GitHub validation {label} timestamp was malformed"))
                .map(|timestamp| {
                    timestamp
                        .with_timezone(&Utc)
                        .to_rfc3339_opts(SecondsFormat::AutoSi, true)
                })
        })
        .transpose()
}

fn normalize_run_status(status: &str, conclusion: Option<&str>) -> GithubValidationRunStatus {
    match status {
        "queued" | "waiting" | "requested" | "pending" => GithubValidationRunStatus::Queued,
        "in_progress" => GithubValidationRunStatus::InProgress,
        "completed" => match conclusion {
            Some("success") => GithubValidationRunStatus::Succeeded,
            Some("failure" | "timed_out" | "action_required" | "startup_failure" | "stale") => {
                GithubValidationRunStatus::Failed
            }
            Some("cancelled") => GithubValidationRunStatus::Cancelled,
            Some("skipped" | "neutral") => GithubValidationRunStatus::Skipped,
            Some(other) => GithubValidationRunStatus::Unknown(format!(
                "completed:{}",
                sanitized_provider_text(other, 64)
            )),
            None => GithubValidationRunStatus::Unknown("completed".to_string()),
        },
        other => GithubValidationRunStatus::Unknown(sanitized_provider_text(other, 64)),
    }
}

#[derive(Deserialize)]
struct PullRequestResponse {
    state: String,
    merged: bool,
    merge_commit_sha: Option<String>,
    head: PullRequestHead,
}

#[derive(Deserialize)]
struct PullRequestHead {
    sha: String,
}

#[derive(Deserialize)]
struct ReviewResponse {
    id: ProviderId,
    body: Option<String>,
    state: String,
    submitted_at: Option<String>,
    commit_id: String,
    user: Option<ValidationActorResponse>,
}

#[derive(Deserialize)]
struct IssueCommentResponse {
    id: ProviderId,
    body: String,
    updated_at: String,
    user: Option<ValidationActorResponse>,
}

#[derive(Deserialize)]
struct ValidationActorResponse {
    login: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct GraphqlEnvelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct ReviewThreadsGraphqlData {
    repository: Option<ReviewThreadsGraphqlRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadsGraphqlRepository {
    pull_request: Option<ReviewThreadsGraphqlPullRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadsGraphqlPullRequest {
    review_threads: ReviewThreadsGraphqlConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadsGraphqlConnection {
    page_info: GraphqlPageInfo,
    nodes: Vec<Option<ReviewThreadGraphqlNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlPageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadGraphqlNode {
    id: ProviderId,
    is_resolved: bool,
    comments: ReviewThreadCommentsGraphqlConnection,
}

#[derive(Deserialize)]
struct ReviewThreadCommentsGraphqlConnection {
    nodes: Vec<Option<ReviewThreadCommentGraphqlNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadCommentGraphqlNode {
    #[allow(dead_code)]
    id: ProviderId,
    body: String,
    updated_at: String,
    author: Option<GraphqlActorResponse>,
    commit: Option<GraphqlCommitResponse>,
}

#[derive(Deserialize)]
struct GraphqlActorResponse {
    login: String,
    #[serde(rename = "__typename")]
    typename: String,
}

#[derive(Deserialize)]
struct GraphqlCommitResponse {
    oid: String,
}

#[derive(Deserialize)]
struct CheckRunsResponse {
    total_count: usize,
    check_runs: Vec<CheckRunResponse>,
}

#[derive(Deserialize)]
struct WorkflowRunsResponse {
    total_count: usize,
    workflow_runs: Vec<WorkflowRunResponse>,
}

#[derive(Deserialize)]
struct CheckRunResponse {
    id: ProviderId,
    name: String,
    head_sha: String,
    status: String,
    conclusion: Option<String>,
    app: Option<CheckAppResponse>,
    check_suite: Option<CheckSuiteResponse>,
    started_at: Option<String>,
    completed_at: Option<String>,
}

#[derive(Deserialize)]
struct CheckAppResponse {
    slug: String,
}

#[derive(Deserialize)]
struct CheckSuiteResponse {
    id: ProviderId,
}

#[derive(Deserialize)]
struct WorkflowRunResponse {
    id: ProviderId,
    name: String,
    head_sha: String,
    status: String,
    conclusion: Option<String>,
    check_suite_id: Option<ProviderId>,
    #[serde(default = "default_run_attempt")]
    run_attempt: u64,
    run_started_at: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

fn default_run_attempt() -> u64 {
    1
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum ProviderId {
    String(String),
    Unsigned(u64),
}

impl ProviderId {
    fn value(&self) -> String {
        match self {
            Self::String(value) => value.clone(),
            Self::Unsigned(value) => value.to_string(),
        }
    }

    fn opaque(&self, namespace: &str) -> GithubOpaqueId {
        GithubOpaqueId::new(format!(
            "{namespace}:{}",
            sanitized_provider_id(&self.value())
        ))
    }
}

#[cfg(test)]
mod tests;
