use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use super::review_poller::GithubReviewPollerAdapter;
use crate::application::port::outbound::github_pr_validation_port::{
    GithubPrMergeState, GithubPrValidationObservationRequest, GithubPrValidationPort,
    GithubPrValidationSnapshot, GithubValidationActivity, GithubValidationActivityKind,
    GithubValidationCheckRun, GithubValidationCursor, GithubValidationRunStatus,
    GithubValidationSource, GithubValidationSourceObservation, GithubValidationSourceStatus,
    GithubValidationWorkflowRun,
};
use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId};

const PER_PAGE: usize = 100;
const MAX_PAGINATED_PAGES: usize = 20;
const CURSOR_VERSION: u8 = 1;

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
    fn get(&self, endpoint: &str) -> Result<String>;
}

struct LocalCredentialsGithubValidationApi {
    repo_root: PathBuf,
    poller: OnceLock<Result<GithubReviewPollerAdapter, String>>,
}

impl GithubValidationApi for LocalCredentialsGithubValidationApi {
    fn get(&self, endpoint: &str) -> Result<String> {
        let poller = self.poller.get_or_init(|| {
            GithubReviewPollerAdapter::from_local_github_credentials(&self.repo_root)
                .map_err(|error| error.to_string())
        });
        match poller {
            Ok(poller) => poller.fetch_validation_json(endpoint),
            Err(error) => Err(anyhow!(error.clone())),
        }
    }
}

impl GithubValidationApi for GithubReviewPollerAdapter {
    fn get(&self, endpoint: &str) -> Result<String> {
        self.fetch_validation_json(endpoint)
    }
}

impl GithubPrValidationPort for GithubPrValidationAdapter {
    fn load_validation_snapshot(
        &self,
        request: &GithubPrValidationObservationRequest,
    ) -> Result<GithubPrValidationSnapshot> {
        let mut pages = ValidationPages::for_request(request)?;
        let repository = &request.target.repository;
        let number = request.target.number;

        // Head identity is checked before any activity or run endpoint. A moved branch must never
        // produce a snapshot labelled with the stale SHA supplied by the application.
        let pull_path = format!("/repos/{repository}/pulls/{number}");
        let pull: PullRequestResponse = self.get_json(&pull_path)?;
        if pull.head.sha != request.target_sha.as_str() {
            bail!("GitHub PR head SHA changed before validation observation completed")
        }

        let mut activities = Vec::new();
        let mut check_runs = Vec::new();
        let mut workflow_runs = Vec::new();
        let mut source_states = BTreeMap::new();
        source_states.insert(
            GithubValidationSource::PullRequest,
            SourcePageState::Complete,
        );

        if let Some(page) = pages.reviews {
            let path = paged_path(&format!("/repos/{repository}/pulls/{number}/reviews"), page);
            let rows: Vec<ReviewResponse> = self.get_json(&path)?;
            ensure_page_bound(&rows, &path)?;
            let count = rows.len();
            activities.extend(rows.into_iter().filter_map(normalize_review));
            source_states.insert(
                GithubValidationSource::Reviews,
                advance_array_page(&mut pages.reviews, page, count)?,
            );
        } else {
            source_states.insert(GithubValidationSource::Reviews, SourcePageState::Complete);
        }

        if let Some(page) = pages.issue_comments {
            let path = paged_path(
                &format!("/repos/{repository}/issues/{number}/comments"),
                page,
            );
            let rows: Vec<IssueCommentResponse> = self.get_json(&path)?;
            ensure_page_bound(&rows, &path)?;
            let count = rows.len();
            activities.extend(rows.into_iter().map(normalize_issue_comment));
            source_states.insert(
                GithubValidationSource::IssueComments,
                advance_array_page(&mut pages.issue_comments, page, count)?,
            );
        } else {
            source_states.insert(
                GithubValidationSource::IssueComments,
                SourcePageState::Complete,
            );
        }

        if let Some(page) = pages.review_threads {
            let path = paged_path(
                &format!("/repos/{repository}/pulls/{number}/comments"),
                page,
            );
            let rows: Vec<ReviewCommentResponse> = self.get_json(&path)?;
            ensure_page_bound(&rows, &path)?;
            let count = rows.len();
            activities.extend(normalize_review_threads(rows));
            source_states.insert(
                GithubValidationSource::ReviewThreads,
                advance_array_page(&mut pages.review_threads, page, count)?,
            );
        } else {
            source_states.insert(
                GithubValidationSource::ReviewThreads,
                SourcePageState::Complete,
            );
        }

        if let Some(page) = pages.check_runs {
            let path = paged_path(
                &format!(
                    "/repos/{repository}/commits/{}/check-runs",
                    request.target_sha.as_str()
                ),
                page,
            );
            let response: CheckRunsResponse = self.get_json(&path)?;
            ensure_page_bound(&response.check_runs, &path)?;
            for row in response.check_runs {
                ensure_run_sha(&row.head_sha, &request.target_sha)?;
                check_runs.push(GithubValidationCheckRun::new(
                    row.id.opaque("check-run"),
                    row.name,
                    GithubCommitSha::new(row.head_sha),
                    normalize_run_status(&row.status, row.conclusion.as_deref()),
                ));
            }
            source_states.insert(
                GithubValidationSource::CheckRuns,
                advance_counted_page(
                    &mut pages.check_runs,
                    page,
                    check_runs.len(),
                    response.total_count,
                )?,
            );
        } else {
            source_states.insert(GithubValidationSource::CheckRuns, SourcePageState::Complete);
        }

        if let Some(page) = pages.workflow_runs {
            let base = format!(
                "/repos/{repository}/actions/runs?head_sha={}",
                request.target_sha.as_str()
            );
            let path = paged_query(&base, page);
            let response: WorkflowRunsResponse = self.get_json(&path)?;
            ensure_page_bound(&response.workflow_runs, &path)?;
            for row in response.workflow_runs {
                ensure_run_sha(&row.head_sha, &request.target_sha)?;
                workflow_runs.push(GithubValidationWorkflowRun::new(
                    row.id.opaque("workflow-run"),
                    row.name,
                    GithubCommitSha::new(row.head_sha),
                    normalize_run_status(&row.status, row.conclusion.as_deref()),
                ));
            }
            source_states.insert(
                GithubValidationSource::WorkflowRuns,
                advance_counted_page(
                    &mut pages.workflow_runs,
                    page,
                    workflow_runs.len(),
                    response.total_count,
                )?,
            );
        } else {
            source_states.insert(
                GithubValidationSource::WorkflowRuns,
                SourcePageState::Complete,
            );
        }

        // Re-read the PR after collecting its endpoint families. This closes the race where the
        // branch moves after the first identity check but before the snapshot is returned.
        let pull: PullRequestResponse = self.get_json(&pull_path)?;
        if pull.head.sha != request.target_sha.as_str() {
            bail!("GitHub PR head SHA changed before validation observation completed")
        }
        let (merge_state, merge_sha) = normalize_merge_state(pull);

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
                    format!("rest:{}", source_label(source)),
                    status,
                    cursor,
                )
            })
            .collect();

        let mut snapshot = GithubPrValidationSnapshot {
            target: request.target.clone(),
            target_sha: request.target_sha.clone(),
            merge_state,
            merge_sha,
            activities,
            check_runs,
            workflow_runs,
            sources,
            next_cursor,
        };
        snapshot.normalize();
        Ok(snapshot)
    }
}

impl GithubPrValidationAdapter {
    fn get_json<T: for<'de> Deserialize<'de>>(&self, endpoint: &str) -> Result<T> {
        let body = self.api.get(endpoint)?;
        serde_json::from_str(&body)
            .with_context(|| format!("failed to parse GitHub validation response for {endpoint}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ValidationPages {
    version: u8,
    repository: String,
    number: u64,
    target_sha: String,
    reviews: Option<usize>,
    issue_comments: Option<usize>,
    review_threads: Option<usize>,
    check_runs: Option<usize>,
    workflow_runs: Option<usize>,
}

impl ValidationPages {
    fn for_request(request: &GithubPrValidationObservationRequest) -> Result<Self> {
        let Some(cursor) = request.cursor.as_ref() else {
            return Ok(Self {
                version: CURSOR_VERSION,
                repository: request.target.repository.clone(),
                number: request.target.number,
                target_sha: request.target_sha.as_str().to_string(),
                reviews: Some(1),
                issue_comments: Some(1),
                review_threads: Some(1),
                check_runs: Some(1),
                workflow_runs: Some(1),
            });
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
        if [
            pages.reviews,
            pages.issue_comments,
            pages.review_threads,
            pages.check_runs,
            pages.workflow_runs,
        ]
        .into_iter()
        .flatten()
        .any(|page| !(2..=MAX_PAGINATED_PAGES).contains(&page))
        {
            bail!("GitHub validation cursor contained an invalid page")
        }
        Ok(pages)
    }

    fn has_more(&self) -> bool {
        self.reviews.is_some()
            || self.issue_comments.is_some()
            || self.review_threads.is_some()
            || self.check_runs.is_some()
            || self.workflow_runs.is_some()
    }

    fn to_cursor(&self) -> Result<GithubValidationCursor> {
        let bytes =
            serde_json::to_vec(self).context("failed to encode GitHub validation cursor")?;
        Ok(GithubValidationCursor::new(URL_SAFE_NO_PAD.encode(bytes)))
    }
}

#[derive(Clone, Copy)]
enum SourcePageState {
    Complete,
    Next(usize),
}

fn advance_array_page(
    slot: &mut Option<usize>,
    page: usize,
    count: usize,
) -> Result<SourcePageState> {
    if count == PER_PAGE {
        let next = next_page(page)?;
        *slot = Some(next);
        Ok(SourcePageState::Next(next))
    } else {
        *slot = None;
        Ok(SourcePageState::Complete)
    }
}

fn advance_counted_page(
    slot: &mut Option<usize>,
    page: usize,
    count: usize,
    total_count: usize,
) -> Result<SourcePageState> {
    let observed_through = page
        .checked_sub(1)
        .and_then(|page_index| page_index.checked_mul(PER_PAGE))
        .and_then(|offset| offset.checked_add(count))
        .ok_or_else(|| anyhow!("GitHub validation pagination count overflowed"))?;
    if observed_through < total_count {
        if count != PER_PAGE {
            bail!("GitHub validation counted response ended before its declared total")
        }
        let next = next_page(page)?;
        *slot = Some(next);
        Ok(SourcePageState::Next(next))
    } else {
        *slot = None;
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

fn paged_path(base: &str, page: usize) -> String {
    format!("{base}?per_page={PER_PAGE}&page={page}")
}

fn paged_query(base: &str, page: usize) -> String {
    format!("{base}&per_page={PER_PAGE}&page={page}")
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
    response: PullRequestResponse,
) -> (GithubPrMergeState, Option<GithubCommitSha>) {
    if response.merged {
        return (
            GithubPrMergeState::Merged,
            response.merge_commit_sha.map(GithubCommitSha::new),
        );
    }
    let state = match response.state.as_str() {
        "open" => GithubPrMergeState::Open,
        "closed" => GithubPrMergeState::Closed,
        other => GithubPrMergeState::Unknown(other.to_string()),
    };
    (state, None)
}

fn normalize_review(row: ReviewResponse) -> Option<GithubValidationActivity> {
    row.submitted_at.map(|observed_at| {
        GithubValidationActivity::new(
            row.id.opaque("review"),
            GithubValidationActivityKind::Review,
            observed_at,
        )
        .with_commit_sha(GithubCommitSha::new(row.commit_id))
    })
}

fn normalize_issue_comment(row: IssueCommentResponse) -> GithubValidationActivity {
    GithubValidationActivity::new(
        row.id.opaque("issue-comment"),
        GithubValidationActivityKind::IssueComment,
        row.updated_at,
    )
}

fn normalize_review_threads(rows: Vec<ReviewCommentResponse>) -> Vec<GithubValidationActivity> {
    let mut threads = BTreeMap::<String, (GithubValidationActivity, bool)>::new();
    let mut comments = Vec::new();
    for row in rows {
        let is_root = row.in_reply_to_id.is_none();
        let root = row.in_reply_to_id.as_ref().unwrap_or(&row.id);
        let thread_id = root.label("review-thread");
        let thread = GithubValidationActivity::new(
            GithubOpaqueId::new(thread_id.clone()),
            GithubValidationActivityKind::ReviewThread,
            row.updated_at.clone(),
        )
        .with_commit_sha(GithubCommitSha::new(row.commit_id.clone()));
        match threads.get(&thread_id) {
            // The root comment is the canonical thread activity. A reply may synthesize a thread
            // only when its root is absent from this page; it must never replace root identity.
            Some((_, current_is_root)) if *current_is_root || !is_root => {}
            _ => {
                threads.insert(thread_id, (thread, is_root));
            }
        }
        if !is_root {
            comments.push(
                GithubValidationActivity::new(
                    row.id.opaque("review-comment"),
                    GithubValidationActivityKind::ReviewComment,
                    row.updated_at,
                )
                .with_commit_sha(GithubCommitSha::new(row.commit_id)),
            );
        }
    }
    threads
        .into_values()
        .map(|(thread, _)| thread)
        .chain(comments)
        .collect()
}

fn ensure_run_sha(observed: &str, expected: &GithubCommitSha) -> Result<()> {
    if observed != expected.as_str() {
        bail!("GitHub validation run was not bound to the requested target SHA")
    }
    Ok(())
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
            Some(other) => GithubValidationRunStatus::Unknown(format!("completed:{other}")),
            None => GithubValidationRunStatus::Unknown("completed".to_string()),
        },
        other => GithubValidationRunStatus::Unknown(other.to_string()),
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
    submitted_at: Option<String>,
    commit_id: String,
}

#[derive(Deserialize)]
struct IssueCommentResponse {
    id: ProviderId,
    updated_at: String,
}

#[derive(Deserialize)]
struct ReviewCommentResponse {
    id: ProviderId,
    updated_at: String,
    commit_id: String,
    in_reply_to_id: Option<ProviderId>,
}

#[derive(Deserialize)]
struct CheckRunsResponse {
    total_count: usize,
    check_runs: Vec<RunResponse>,
}

#[derive(Deserialize)]
struct WorkflowRunsResponse {
    total_count: usize,
    workflow_runs: Vec<RunResponse>,
}

#[derive(Deserialize)]
struct RunResponse {
    id: ProviderId,
    name: String,
    head_sha: String,
    status: String,
    conclusion: Option<String>,
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

    fn label(&self, namespace: &str) -> String {
        format!("{namespace}:{}", self.value())
    }

    fn opaque(&self, namespace: &str) -> GithubOpaqueId {
        GithubOpaqueId::new(self.label(namespace))
    }
}

#[cfg(test)]
mod tests;
