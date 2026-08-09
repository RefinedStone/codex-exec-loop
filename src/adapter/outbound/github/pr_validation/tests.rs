use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};

use super::{GithubPrValidationAdapter, GithubValidationApi, GithubValidationApiResponse};
use crate::application::port::outbound::github_pr_validation_port::{
    GithubPrMergeState, GithubPrValidationError, GithubPrValidationErrorClass,
    GithubPrValidationObservationRequest, GithubPrValidationPort, GithubValidationActivityKind,
    GithubValidationActorKind, GithubValidationBodyMarker, GithubValidationProviderMetadata,
    GithubValidationReviewState, GithubValidationRunStatus, GithubValidationSource,
    GithubValidationSourceStatus,
};
use crate::domain::github_review::{GithubCommitSha, GithubPullRequestTarget};
use crate::domain::parallel_mode::PostMergeValidationContract;

const SHA: &str = "1111111111111111111111111111111111111111";
const MERGE_SHA: &str = "2222222222222222222222222222222222222222";

struct FixtureApi {
    responses: BTreeMap<String, String>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FixtureApi {
    fn new(responses: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut responses = responses.into_iter().collect::<BTreeMap<_, _>>();
        responses
            .entry("POST /graphql".to_string())
            .or_insert_with(empty_review_threads_graphql_response);
        Self {
            responses,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn request_log(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.requests)
    }
}

impl GithubValidationApi for FixtureApi {
    fn get(
        &self,
        endpoint: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        self.requests
            .lock()
            .expect("request lock")
            .push(endpoint.to_string());
        self.responses
            .get(endpoint)
            .cloned()
            .map(|body| GithubValidationApiResponse {
                body,
                metadata: GithubValidationProviderMetadata::default(),
            })
            .ok_or_else(|| {
                GithubPrValidationError::integrity_failed(format!(
                    "unexpected fixture endpoint: {endpoint}"
                ))
            })
    }

    fn post(
        &self,
        endpoint: &str,
        _body: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        let key = format!("POST {endpoint}");
        self.requests
            .lock()
            .expect("request lock")
            .push(key.clone());
        self.responses
            .get(&key)
            .cloned()
            .map(|body| GithubValidationApiResponse {
                body,
                metadata: GithubValidationProviderMetadata::default(),
            })
            .ok_or_else(|| {
                GithubPrValidationError::integrity_failed(format!(
                    "unexpected fixture endpoint: {key}"
                ))
            })
    }
}

struct SequencedGraphqlApi {
    fixture: FixtureApi,
    graphql_responses: Mutex<VecDeque<String>>,
    graphql_bodies: Arc<Mutex<Vec<String>>>,
}

impl SequencedGraphqlApi {
    fn new(fixture: FixtureApi, responses: impl IntoIterator<Item = String>) -> Self {
        Self {
            fixture,
            graphql_responses: Mutex::new(responses.into_iter().collect()),
            graphql_bodies: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn graphql_bodies(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.graphql_bodies)
    }
}

impl GithubValidationApi for SequencedGraphqlApi {
    fn get(
        &self,
        endpoint: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        self.fixture.get(endpoint)
    }

    fn post(
        &self,
        endpoint: &str,
        body: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        if endpoint != "/graphql" {
            return Err(GithubPrValidationError::integrity_failed(
                "unexpected sequenced fixture endpoint",
            ));
        }
        self.fixture
            .requests
            .lock()
            .expect("request lock")
            .push("POST /graphql".to_string());
        self.graphql_bodies
            .lock()
            .expect("GraphQL body lock")
            .push(body.to_string());
        self.graphql_responses
            .lock()
            .expect("GraphQL response lock")
            .pop_front()
            .map(|body| GithubValidationApiResponse {
                body,
                metadata: GithubValidationProviderMetadata::default(),
            })
            .ok_or_else(|| {
                GithubPrValidationError::integrity_failed(
                    "sequenced GraphQL fixture exhausted its responses",
                )
            })
    }
}

struct MovingHeadApi {
    fixture: FixtureApi,
    pull_reads: AtomicUsize,
}

struct MetadataErrorApi {
    calls: AtomicUsize,
}

impl GithubValidationApi for MetadataErrorApi {
    fn get(
        &self,
        _endpoint: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        let first_reset = Utc.with_ymd_and_hms(2026, 8, 10, 0, 1, 0).unwrap();
        let final_reset = Utc.with_ymd_and_hms(2026, 8, 10, 0, 2, 0).unwrap();
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(GithubValidationApiResponse {
                body: format!(
                    r#"{{"state":"open","merged":false,"merge_commit_sha":null,"head":{{"sha":"{SHA}"}}}}"#
                ),
                metadata: GithubValidationProviderMetadata {
                    rate_limit_remaining: Some(100),
                    rate_limit_reset_at: Some(first_reset),
                },
            });
        }
        Err(
            GithubPrValidationError::retryable("GitHub HTTP 503").with_provider_metadata(
                GithubValidationProviderMetadata {
                    rate_limit_remaining: Some(80),
                    rate_limit_reset_at: Some(final_reset),
                },
            ),
        )
    }
}

impl GithubValidationApi for MovingHeadApi {
    fn get(
        &self,
        endpoint: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        if endpoint == "/repos/acme/widgets/pulls/42"
            && self.pull_reads.fetch_add(1, Ordering::SeqCst) == 1
        {
            return Ok(GithubValidationApiResponse {
                body: r#"{"state":"open","merged":false,"merge_commit_sha":null,"head":{"sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#.to_string(),
                metadata: GithubValidationProviderMetadata::default(),
            });
        }
        self.fixture.get(endpoint)
    }

    fn post(
        &self,
        endpoint: &str,
        body: &str,
    ) -> std::result::Result<GithubValidationApiResponse, GithubPrValidationError> {
        self.fixture.post(endpoint, body)
    }
}

fn empty_review_threads_graphql_response() -> String {
    r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}}"#.to_string()
}

fn review_thread_node(index: usize) -> serde_json::Value {
    serde_json::json!({
        "id": format!("PRRT_stable_{index:03}"),
        "isResolved": false,
        "comments": {
            "nodes": [{
                "id": format!("PRRC_root_{index:03}"),
                "body": "thread root",
                "updatedAt": "2026-08-07T10:00:02Z",
                "author": {"login": "reviewer", "__typename": "User"},
                "commit": {"oid": SHA}
            }]
        }
    })
}

fn review_threads_page(
    nodes: Vec<serde_json::Value>,
    has_next_page: bool,
    end_cursor: Option<&str>,
) -> String {
    serde_json::json!({
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": {
                            "hasNextPage": has_next_page,
                            "endCursor": end_cursor
                        },
                        "nodes": nodes
                    }
                }
            }
        }
    })
    .to_string()
}

fn request(
    cursor: Option<
        crate::application::port::outbound::github_pr_validation_port::GithubValidationCursor,
    >,
) -> GithubPrValidationObservationRequest {
    GithubPrValidationObservationRequest::new(
        GithubPullRequestTarget::new("acme/widgets", 42),
        GithubCommitSha::new(SHA),
        cursor,
    )
}

fn endpoint(path: &str) -> String {
    format!("/repos/acme/widgets/{path}")
}

#[test]
fn provider_error_preserves_metadata_accumulated_before_a_later_endpoint_failure() {
    let adapter = GithubPrValidationAdapter::with_api(MetadataErrorApi {
        calls: AtomicUsize::new(0),
    });
    let error = adapter
        .load_validation_snapshot(&request(None))
        .expect_err("the second endpoint should fail");
    assert_eq!(error.provider_metadata.rate_limit_remaining, Some(80));
    assert_eq!(
        error.provider_metadata.rate_limit_reset_at,
        Some(Utc.with_ymd_and_hms(2026, 8, 10, 0, 2, 0).unwrap())
    );
}

fn complete_fixture() -> FixtureApi {
    FixtureApi::new([
        (
            endpoint("pulls/42"),
            format!(
                r#"{{"state":"closed","merged":true,"merge_commit_sha":"{MERGE_SHA}","head":{{"sha":"{SHA}"}}}}"#
            ),
        ),
        (
            endpoint("pulls/42/reviews?per_page=100&page=1"),
            format!(
                r#"[{{"id":9007199254740993,"body":"LGTM","state":"COMMENTED","submitted_at":"2026-08-07T10:00:03Z","commit_id":"{SHA}","user":{{"login":"reviewer","type":"User"}}}}]"#
            ),
        ),
        (
            endpoint("issues/42/comments?per_page=100&page=1"),
            r#"[{"id":8,"body":"ordinary conversation","updated_at":"2026-08-07T10:00:01Z","user":{"login":"operator","type":"User"}}]"#.to_string(),
        ),
        (
            "POST /graphql".to_string(),
            format!(
                r#"{{"data":{{"repository":{{"pullRequest":{{"reviewThreads":{{
                    "pageInfo":{{"hasNextPage":false,"endCursor":null}},
                    "nodes":[{{"id":"PRRT_stable_10","isResolved":false,"comments":{{"nodes":[{{
                        "id":"PRRC_root_10","body":"please fix this","updatedAt":"2026-08-07T10:00:02Z",
                        "author":{{"login":"reviewer","__typename":"User"}},"commit":{{"oid":"{SHA}"}}
                    }},{{
                        "id":"PRRC_reply_10","body":"a reordered reply must not become a second finding","updatedAt":"2026-08-07T10:00:04Z",
                        "author":{{"login":"other-reviewer","__typename":"User"}},"commit":{{"oid":"{SHA}"}}
                    }}]}}}}]
                }}}}}}}}}}"#
            ),
        ),
        (
            endpoint(&format!(
                "commits/{MERGE_SHA}/check-runs?filter=all&per_page=100&page=1"
            )),
            format!(
                r#"{{"total_count":2,"check_runs":[
                    {{"id":22,"name":"zeta","head_sha":"{MERGE_SHA}","status":"completed","conclusion":"failure"}},
                    {{"id":21,"name":"alpha","head_sha":"{MERGE_SHA}","status":"completed","conclusion":"success"}}
                ]}}"#
            ),
        ),
        (
            endpoint(&format!(
                "actions/runs?head_sha={MERGE_SHA}&per_page=100&page=1"
            )),
            format!(
                r#"{{"total_count":2,"workflow_runs":[
                    {{"id":32,"name":"release","head_sha":"{MERGE_SHA}","status":"completed","conclusion":"cancelled"}},
                    {{"id":31,"name":"ci","head_sha":"{MERGE_SHA}","status":"in_progress","conclusion":null}}
                ]}}"#
            ),
        ),
    ])
}

#[test]
fn collects_sha_bound_merge_activity_checks_and_workflows_in_canonical_order() {
    let api = complete_fixture();
    let adapter = GithubPrValidationAdapter::with_api(api);

    let snapshot = adapter
        .load_validation_snapshot(&request(None))
        .expect("fixture snapshot should load");

    assert_eq!(
        snapshot.target,
        GithubPullRequestTarget::new("acme/widgets", 42)
    );
    assert_eq!(snapshot.target_sha.as_str(), SHA);
    assert_eq!(snapshot.evidence_sha.as_str(), MERGE_SHA);
    assert_eq!(snapshot.merge_state, GithubPrMergeState::Merged);
    assert_eq!(
        snapshot.merge_sha.as_ref().map(GithubCommitSha::as_str),
        Some(MERGE_SHA)
    );
    assert_eq!(
        snapshot
            .activities
            .iter()
            .map(|activity| (activity.kind, activity.id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (
                GithubValidationActivityKind::IssueComment,
                "issue-comment:8"
            ),
            (
                GithubValidationActivityKind::ReviewThread,
                "review-thread:PRRT_stable_10"
            ),
            (
                GithubValidationActivityKind::Review,
                "review:9007199254740993"
            ),
        ]
    );
    let issue = snapshot
        .activities
        .iter()
        .find(|activity| activity.kind == GithubValidationActivityKind::IssueComment)
        .expect("issue comment activity");
    assert_eq!(issue.body_marker, GithubValidationBodyMarker::None);
    assert_eq!(
        issue.actor.as_ref().map(|actor| actor.kind),
        Some(GithubValidationActorKind::User)
    );
    let review = snapshot
        .activities
        .iter()
        .find(|activity| activity.kind == GithubValidationActivityKind::Review)
        .expect("review activity");
    assert_eq!(
        review.review_state,
        Some(GithubValidationReviewState::Commented)
    );
    assert_eq!(review.body_marker, GithubValidationBodyMarker::None);
    let thread = snapshot
        .activities
        .iter()
        .find(|activity| activity.kind == GithubValidationActivityKind::ReviewThread)
        .expect("review thread activity");
    assert_eq!(thread.thread_resolved, Some(false));
    assert_eq!(
        thread.actor.as_ref().map(|actor| actor.login.as_str()),
        Some("reviewer")
    );
    let thread_source = snapshot
        .sources
        .iter()
        .find(|source| source.source == GithubValidationSource::ReviewThreads)
        .expect("review thread source");
    assert_eq!(thread_source.endpoint, "graphql:review-threads");
    assert_eq!(
        snapshot
            .check_runs
            .iter()
            .map(|run| (run.name.as_str(), &run.status))
            .collect::<Vec<_>>(),
        vec![
            ("alpha", &GithubValidationRunStatus::Succeeded),
            ("zeta", &GithubValidationRunStatus::Failed),
        ]
    );
    assert_eq!(snapshot.workflow_runs[0].name, "ci");
    assert_eq!(
        snapshot.workflow_runs[0].status,
        GithubValidationRunStatus::InProgress
    );
    assert_eq!(
        snapshot.workflow_runs[1].status,
        GithubValidationRunStatus::Cancelled
    );
    assert!(snapshot.next_cursor.is_none());
    assert!(snapshot.observations_complete());
    assert!(!snapshot.is_successfully_complete(
        &PostMergeValidationContract::production_v1(),
        &snapshot.evidence_sha,
    ));
    assert_eq!(snapshot.sources.len(), GithubValidationSource::ALL.len());
    assert!(snapshot.sources.iter().all(|source| {
        source.status == GithubValidationSourceStatus::Complete && source.next_cursor.is_none()
    }));
}

#[test]
fn pr2100_push_fixture_treats_successful_required_and_skipped_optional_checks_as_complete() {
    const SOURCE: &str = "2c28eca0dfc2ebd4681a6552b95859a72859c01e";
    const EVIDENCE: &str = "6037d07b8e9a6cc8614610569b044f95f6fa9075";
    let api = FixtureApi::new([
        (
            endpoint("pulls/42"),
            format!(
                r#"{{"state":"closed","merged":true,"merge_commit_sha":"{EVIDENCE}","head":{{"sha":"{SOURCE}"}}}}"#
            ),
        ),
        (
            endpoint("pulls/42/reviews?per_page=100&page=1"),
            "[]".to_string(),
        ),
        (
            endpoint("issues/42/comments?per_page=100&page=1"),
            "[]".to_string(),
        ),
        (
            endpoint(&format!(
                "commits/{EVIDENCE}/check-runs?filter=all&per_page=100&page=1"
            )),
            include_str!("fixtures/pr2100-check-runs.json").to_string(),
        ),
        (
            endpoint(&format!(
                "actions/runs?head_sha={EVIDENCE}&per_page=100&page=1"
            )),
            include_str!("fixtures/pr2100-workflow-runs.json").to_string(),
        ),
    ]);
    let request = GithubPrValidationObservationRequest::new(
        GithubPullRequestTarget::new("acme/widgets", 42),
        GithubCommitSha::new(SOURCE),
        None,
    );
    let snapshot = GithubPrValidationAdapter::with_api(api)
        .load_validation_snapshot(&request)
        .expect("captured PR #2100 push evidence should normalize");
    let contract = PostMergeValidationContract::legacy_ci_gate_v1();

    let decision = snapshot.evaluate_post_merge_contract(&contract);
    assert!(decision.is_successful());
    assert_eq!(snapshot.workflow_runs[0].run_attempt, 1);
    assert_eq!(
        snapshot.workflow_runs[0]
            .check_suite_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("check-suite:84760241806")
    );
    assert_eq!(
        snapshot.workflow_runs[0].run_started_at.as_deref(),
        Some("2026-08-08T02:33:51Z")
    );
    let gate = snapshot
        .check_runs
        .iter()
        .find(|run| run.name == "CI Gate")
        .expect("captured required gate");
    assert_eq!(gate.app_slug.as_deref(), Some("github-actions"));
    assert_eq!(
        gate.check_suite_id.as_ref().map(|id| id.as_str()),
        Some("check-suite:84760241806")
    );
    assert_eq!(gate.started_at.as_deref(), Some("2026-08-08T02:34:50Z"));
    assert_eq!(
        snapshot.workflow_runs[0].updated_at.as_deref(),
        Some("2026-08-08T02:35:00Z")
    );
    assert!(snapshot.check_runs.iter().any(|run| {
        run.name == "Rust Tests" && run.status == GithubValidationRunStatus::Skipped
    }));
    assert!(snapshot.is_successfully_complete(&contract, &GithubCommitSha::new(EVIDENCE)));
}

#[test]
fn closed_unmerged_pr_uses_distributor_attested_evidence_sha() {
    let api = FixtureApi::new([
        (
            endpoint("pulls/42"),
            format!(
                r#"{{"state":"closed","merged":false,"merge_commit_sha":null,"head":{{"sha":"{SHA}"}}}}"#
            ),
        ),
        (
            endpoint("pulls/42/reviews?per_page=100&page=1"),
            "[]".to_string(),
        ),
        (
            endpoint("issues/42/comments?per_page=100&page=1"),
            "[]".to_string(),
        ),
        (
            endpoint(&format!(
                "commits/{MERGE_SHA}/check-runs?filter=all&per_page=100&page=1"
            )),
            format!(
                r#"{{"total_count":1,"check_runs":[{{"id":21,"name":"Post-Merge Gate","head_sha":"{MERGE_SHA}","status":"completed","conclusion":"success"}}]}}"#
            ),
        ),
        (
            endpoint(&format!(
                "actions/runs?head_sha={MERGE_SHA}&per_page=100&page=1"
            )),
            r#"{"total_count":0,"workflow_runs":[]}"#.to_string(),
        ),
    ]);
    let adapter = GithubPrValidationAdapter::with_api(api);
    let observation = request(None).with_evidence_sha(Some(GithubCommitSha::new(MERGE_SHA)));

    let snapshot = adapter
        .load_validation_snapshot(&observation)
        .expect("distributor evidence should remain observable after PR close");

    assert_eq!(snapshot.merge_state, GithubPrMergeState::Closed);
    assert!(snapshot.merge_sha.is_none());
    assert_eq!(snapshot.evidence_sha.as_str(), MERGE_SHA);
    assert_eq!(snapshot.check_runs.len(), 1);
    assert_eq!(snapshot.check_runs[0].target_sha.as_str(), MERGE_SHA);
}

#[test]
fn cursor_polls_return_stable_cumulative_evidence_across_all_sources() {
    let mut responses = complete_fixture().responses;
    let reviews = (0..100)
        .map(|id| {
            format!(
                r#"{{"id":{},"body":"","state":"COMMENTED","submitted_at":"2026-08-07T10:00:00Z","commit_id":"{SHA}","user":{{"login":"reviewer","type":"User"}}}}"#,
                1000 + id
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    responses.insert(
        endpoint("pulls/42/reviews?per_page=100&page=1"),
        format!("[{reviews}]"),
    );
    responses.insert(
        endpoint("pulls/42/reviews?per_page=100&page=2"),
        format!(r#"[{{"id":2000,"body":"","state":"COMMENTED","submitted_at":"2026-08-07T10:01:00Z","commit_id":"{SHA}","user":{{"login":"reviewer","type":"User"}}}}]"#),
    );
    let first_api = FixtureApi::new(responses.clone());
    let first = GithubPrValidationAdapter::with_api(first_api)
        .load_validation_snapshot(&request(None))
        .expect("first page should load");
    let cursor = first
        .next_cursor
        .clone()
        .expect("full page should continue");
    assert_eq!(first.activities.len(), 102);
    let review_source = first
        .sources
        .iter()
        .find(|source| source.source == GithubValidationSource::Reviews)
        .expect("review provenance");
    assert_eq!(
        review_source.status,
        GithubValidationSourceStatus::Paginated
    );
    assert!(review_source.next_cursor.is_some());

    let resumed_api = FixtureApi::new(responses);
    let resumed_requests = resumed_api.request_log();
    let resumed_adapter = GithubPrValidationAdapter::with_api(resumed_api);
    let second = resumed_adapter
        .load_validation_snapshot(&request(Some(cursor.clone())))
        .expect("cursor should resume");

    assert_eq!(second.activities.len(), 103);
    assert!(
        second
            .activities
            .iter()
            .any(|activity| activity.id.as_str() == "review:1000")
    );
    assert!(
        second
            .activities
            .iter()
            .any(|activity| activity.id.as_str() == "review:2000")
    );
    assert!(second.next_cursor.is_none());
    assert!(second.observations_complete());
    let requested = resumed_requests.lock().expect("request lock").clone();
    assert_eq!(
        requested,
        vec![
            endpoint("pulls/42"),
            endpoint("pulls/42/reviews?per_page=100&page=1"),
            endpoint("pulls/42/reviews?per_page=100&page=2"),
            endpoint("issues/42/comments?per_page=100&page=1"),
            "POST /graphql".to_string(),
            endpoint(&format!(
                "commits/{MERGE_SHA}/check-runs?filter=all&per_page=100&page=1"
            )),
            endpoint(&format!(
                "actions/runs?head_sha={MERGE_SHA}&per_page=100&page=1"
            )),
            endpoint("pulls/42"),
        ]
    );

    let replay_api = FixtureApi::new(complete_fixture().responses);
    let replay_error = GithubPrValidationAdapter::with_api(replay_api)
        .load_validation_snapshot(&GithubPrValidationObservationRequest::new(
            GithubPullRequestTarget::new("acme/other", 42),
            GithubCommitSha::new(SHA),
            Some(cursor),
        ))
        .expect_err("cursor must be target-bound");
    assert!(
        replay_error
            .to_string()
            .contains("does not match the requested PR revision")
    );
}

#[test]
fn review_thread_pagination_refetches_stable_thread_ids_without_reply_duplicates() {
    let first_page = (0..100).map(review_thread_node).collect::<Vec<_>>();
    let first_api = SequencedGraphqlApi::new(
        complete_fixture(),
        [review_threads_page(
            first_page.clone(),
            true,
            Some("cursor-100"),
        )],
    );
    let first = GithubPrValidationAdapter::with_api(first_api)
        .load_validation_snapshot(&request(None))
        .expect("first review thread page should load");
    let cursor = first
        .next_cursor
        .expect("a continued review thread connection should retain a cursor");
    assert_eq!(
        first
            .activities
            .iter()
            .filter(|activity| activity.kind == GithubValidationActivityKind::ReviewThread)
            .count(),
        100
    );

    let mut reordered_first_page = first_page;
    reordered_first_page.reverse();
    let resumed_api = SequencedGraphqlApi::new(
        complete_fixture(),
        [
            review_threads_page(reordered_first_page, true, Some("cursor-100")),
            review_threads_page(vec![review_thread_node(100)], false, None),
        ],
    );
    let bodies = resumed_api.graphql_bodies();
    let resumed = GithubPrValidationAdapter::with_api(resumed_api)
        .load_validation_snapshot(&request(Some(cursor)))
        .expect("review thread pagination should resume cumulatively");
    let thread_ids = resumed
        .activities
        .iter()
        .filter(|activity| activity.kind == GithubValidationActivityKind::ReviewThread)
        .map(|activity| activity.id.as_str().to_string())
        .collect::<Vec<_>>();
    assert_eq!(thread_ids.len(), 101);
    assert_eq!(
        thread_ids.iter().collect::<BTreeSet<_>>().len(),
        thread_ids.len(),
        "reply order and cumulative refetch must not duplicate a thread finding"
    );
    assert!(
        thread_ids
            .iter()
            .any(|id| id == "review-thread:PRRT_stable_000")
    );
    assert!(
        thread_ids
            .iter()
            .any(|id| id == "review-thread:PRRT_stable_100")
    );
    assert!(resumed.next_cursor.is_none());

    let bodies = bodies.lock().expect("GraphQL body lock");
    assert_eq!(bodies.len(), 2);
    let first_body: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    let second_body: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert!(first_body["variables"]["after"].is_null());
    assert_eq!(second_body["variables"]["after"], "cursor-100");
}

#[test]
fn graphql_review_thread_errors_fail_closed_without_leaking_provider_copy() {
    let mut responses = complete_fixture().responses;
    responses.insert(
        "POST /graphql".to_string(),
        r#"{"data":null,"errors":[{"message":"token ghp_secret_canary denied"}]}"#.to_string(),
    );
    let error = GithubPrValidationAdapter::with_api(FixtureApi::new(responses))
        .load_validation_snapshot(&request(None))
        .expect_err("GraphQL errors must not be treated as an empty review thread result");

    assert_eq!(
        error.class,
        GithubPrValidationErrorClass::AuthenticationBlocked
    );
    assert_eq!(
        error.to_string(),
        "GitHub validation GraphQL query was rejected"
    );
    assert!(!error.to_string().contains("ghp_secret_canary"));
}

#[test]
fn merged_evidence_restarts_pagination_from_pre_merge_cursor() {
    let mut open_responses = complete_fixture().responses;
    let merged_check_runs = open_responses
        .remove(&endpoint(&format!(
            "commits/{MERGE_SHA}/check-runs?filter=all&per_page=100&page=1"
        )))
        .expect("merged check fixture")
        .replace(MERGE_SHA, SHA);
    let merged_workflow_runs = open_responses
        .remove(&endpoint(&format!(
            "actions/runs?head_sha={MERGE_SHA}&per_page=100&page=1"
        )))
        .expect("merged workflow fixture")
        .replace(MERGE_SHA, SHA);
    open_responses.insert(
        endpoint("pulls/42"),
        format!(
            r#"{{"state":"open","merged":false,"merge_commit_sha":null,"head":{{"sha":"{SHA}"}}}}"#
        ),
    );
    open_responses.insert(
        endpoint(&format!(
            "commits/{SHA}/check-runs?filter=all&per_page=100&page=1"
        )),
        merged_check_runs,
    );
    open_responses.insert(
        endpoint(&format!("actions/runs?head_sha={SHA}&per_page=100&page=1")),
        merged_workflow_runs,
    );
    let reviews = (0..100)
        .map(|id| {
            format!(
                r#"{{"id":{},"body":"","state":"COMMENTED","submitted_at":"2026-08-07T10:00:00Z","commit_id":"{SHA}","user":{{"login":"reviewer","type":"User"}}}}"#,
                1000 + id
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    open_responses.insert(
        endpoint("pulls/42/reviews?per_page=100&page=1"),
        format!("[{reviews}]"),
    );
    open_responses.insert(
        endpoint("pulls/42/reviews?per_page=100&page=2"),
        format!(r#"[{{"id":2000,"body":"","state":"COMMENTED","submitted_at":"2026-08-07T10:01:00Z","commit_id":"{SHA}","user":{{"login":"reviewer","type":"User"}}}}]"#),
    );

    let first = GithubPrValidationAdapter::with_api(FixtureApi::new(open_responses))
        .load_validation_snapshot(&request(None))
        .expect("open pagination should load");
    let cursor = first.next_cursor.expect("open page should continue");

    let mut merged_responses = complete_fixture().responses;
    merged_responses.insert(
        endpoint("pulls/42/reviews?per_page=100&page=1"),
        format!("[{reviews}]"),
    );
    merged_responses.insert(
        endpoint("pulls/42/reviews?per_page=100&page=2"),
        format!(r#"[{{"id":2000,"body":"","state":"COMMENTED","submitted_at":"2026-08-07T10:01:00Z","commit_id":"{SHA}","user":{{"login":"reviewer","type":"User"}}}}]"#),
    );
    let merged_api = FixtureApi::new(merged_responses);
    let merged_requests = merged_api.request_log();
    let merged = GithubPrValidationAdapter::with_api(merged_api)
        .load_validation_snapshot(&request(Some(cursor)))
        .expect("merged evidence should restart pagination");

    assert_eq!(merged.evidence_sha.as_str(), MERGE_SHA);
    assert!(merged.next_cursor.is_some());
    let requested = merged_requests.lock().expect("request lock").clone();
    assert!(requested.contains(&endpoint("pulls/42/reviews?per_page=100&page=1")));
    assert!(!requested.contains(&endpoint("pulls/42/reviews?per_page=100&page=2")));
}

#[test]
fn head_sha_mismatch_fails_before_collecting_revision_evidence() {
    let api = FixtureApi::new([(
        endpoint("pulls/42"),
        r#"{"state":"open","merged":false,"merge_commit_sha":null,"head":{"sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#.to_string(),
    )]);
    let requests = api.request_log();
    let adapter = GithubPrValidationAdapter::with_api(api);

    let error = adapter
        .load_validation_snapshot(&request(None))
        .expect_err("moved head must invalidate the observation");

    assert!(error.to_string().contains("PR head SHA changed"));
    assert!(!error.to_string().contains(SHA));
    assert_eq!(
        requests.lock().expect("request lock").as_slice(),
        [endpoint("pulls/42")]
    );
}

#[test]
fn head_sha_mismatch_after_collection_discards_all_observed_evidence() {
    let adapter = GithubPrValidationAdapter::with_api(MovingHeadApi {
        fixture: complete_fixture(),
        pull_reads: AtomicUsize::new(0),
    });

    let error = adapter
        .load_validation_snapshot(&request(None))
        .expect_err("head movement during collection must invalidate the observation");

    assert!(error.to_string().contains("PR head SHA changed"));
    assert!(!error.to_string().contains(SHA));
}

#[test]
fn pagination_refuses_to_continue_past_the_trusted_page_bound() {
    let error = super::next_page(super::MAX_PAGINATED_PAGES)
        .expect_err("a full final page must not create an unbounded cursor");

    assert!(error.to_string().contains("remained full after 20 pages"));
}

#[test]
fn rejects_check_or_workflow_rows_not_bound_to_the_requested_sha() {
    for bad_endpoint in ["check", "workflow"] {
        let mut responses = complete_fixture().responses;
        let wrong_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        if bad_endpoint == "check" {
            responses.insert(
                endpoint(&format!(
                    "commits/{MERGE_SHA}/check-runs?filter=all&per_page=100&page=1"
                )),
                format!(r#"{{"total_count":1,"check_runs":[{{"id":1,"name":"ci","head_sha":"{wrong_sha}","status":"completed","conclusion":"success"}}]}}"#),
            );
        } else {
            responses.insert(
                endpoint(&format!("actions/runs?head_sha={MERGE_SHA}&per_page=100&page=1")),
                format!(r#"{{"total_count":1,"workflow_runs":[{{"id":1,"name":"ci","head_sha":"{wrong_sha}","status":"completed","conclusion":"success"}}]}}"#),
            );
        }
        let error = GithubPrValidationAdapter::with_api(FixtureApi::new(responses))
            .load_validation_snapshot(&request(None))
            .expect_err("foreign revision evidence must fail closed");
        assert!(
            error
                .to_string()
                .contains("was not bound to the requested target SHA")
        );
        assert_eq!(error.class, GithubPrValidationErrorClass::IdentityFailed);
        assert!(!error.to_string().contains(wrong_sha));
    }
}

#[test]
fn body_marker_accepts_only_unquoted_unfenced_explicit_commands() {
    for body in [
        "> @akra fix",
        "```\n/akra remediate\n```",
        "    @akra fix",
        "\t/akra remediate",
        "please consider @akra fix later",
        "@akra fixture",
    ] {
        assert_eq!(
            super::explicit_body_marker(body),
            GithubValidationBodyMarker::None,
            "non-command body must stay non-actionable: {body:?}"
        );
    }
    assert_eq!(
        super::explicit_body_marker("context\n  @AKRA FIX now"),
        GithubValidationBodyMarker::AkraFix
    );
    assert_eq!(
        super::explicit_body_marker("/akra remediate"),
        GithubValidationBodyMarker::AkraRemediate
    );
}
