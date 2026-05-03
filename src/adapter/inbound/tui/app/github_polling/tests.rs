use super::*;
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::domain::github_review::{
    GithubPullRequestActivityKind, GithubPullRequestActivitySnapshot,
};
use anyhow::Result;
use std::sync::{Arc, Mutex};

// The fake port proves bootstrap wiring without reaching GitHub. The call
// counter catches eager polling during setup, while the cloned snapshot keeps
// service-load assertions deterministic.
struct FakeGithubReviewPollerPort {
    calls: Arc<Mutex<usize>>,
    snapshot: GithubPullRequestActivitySnapshot,
}
impl GithubReviewPollerPort for FakeGithubReviewPollerPort {
    fn load_pull_request_activity(
        &self,
        _target: &GithubPullRequestTarget,
    ) -> Result<GithubPullRequestActivitySnapshot> {
        *self.calls.lock().expect("calls mutex poisoned") += 1;
        Ok(self.snapshot.clone())
    }
}

// Bootstrap has three front doors: explicit PR env, branch auto-discovery, and
// disabled. Malformed PR identifiers should become setup errors so shell chrome
// can tell operators what to fix instead of silently turning polling off.
#[test]
fn bootstrap_stays_disabled_without_pull_request_env() {
    let bootstrap =
        GithubReviewPollingBootstrap::from_discovery_result(None, || Ok(None), Instant::now());

    assert!(matches!(
        bootstrap.state,
        GithubReviewPollingState::Disabled
    ));
    assert!(bootstrap.service.is_none());
}

#[test]
fn bootstrap_surfaces_invalid_pull_request_value() {
    let bootstrap = GithubReviewPollingBootstrap::from_env_values(
        Some("not-a-pr".to_string()),
        None,
        || unreachable!("service loader should not run"),
        Instant::now(),
    );
    match bootstrap.state {
        GithubReviewPollingState::SetupError { target, message } => {
            assert!(target.is_none());
            assert!(message.contains("owner/repo#123"));
        }
        other => panic!("expected setup error state, got {other:?}"),
    }
}

#[test]
fn bootstrap_rejects_pull_request_value_with_invalid_repository_shape() {
    let bootstrap = GithubReviewPollingBootstrap::from_env_values(
        Some("owner/repo/extra#42".to_string()),
        None,
        || unreachable!("service loader should not run"),
        Instant::now(),
    );
    match bootstrap.state {
        GithubReviewPollingState::SetupError { target, message } => {
            assert!(target.is_none());
            assert!(message.contains("owner/repo#123"));
        }
        other => panic!("expected setup error state, got {other:?}"),
    }
}

// Active polling starts with an immediate request so a freshly opened review
// lane sees GitHub activity right away. After the first result, the state keeps
// previous poll metadata for delta comparisons and waits for the configured
// interval before asking again.
#[test]
fn active_state_schedules_immediately_then_waits_for_interval() {
    let target = GithubPullRequestTarget::new("acme/widgets", 42);
    let config = GithubReviewPollingConfig {
        target: target.clone(),
        interval: Duration::from_secs(30),
    };
    let start = Instant::now();
    let mut state = GithubReviewPollingState::active(config, start);
    let first_request = state
        .take_due_request(start)
        .expect("initial request should be due");
    assert_eq!(first_request.target, target);
    assert!(first_request.previous_state.is_none());

    state.record_result(
        start + Duration::from_secs(1),
        Ok(sample_poll_result("2026-04-08T09:00:00Z")),
    );

    assert!(
        state
            .take_due_request(start + Duration::from_secs(15))
            .is_none()
    );
    assert!(
        state
            .take_due_request(start + Duration::from_secs(31))
            .is_some()
    );
}

// Poll failures should not collapse the watcher. The error stays on active
// runtime state where the TUI can surface it, and the same state can still be
// retried on the next interval.
#[test]
fn active_state_keeps_last_error_visible() {
    let config = GithubReviewPollingConfig {
        target: GithubPullRequestTarget::new("acme/widgets", 42),
        interval: Duration::from_secs(30),
    };
    let start = Instant::now();
    let mut state = GithubReviewPollingState::active(config, start);
    let _ = state.take_due_request(start);
    state.record_result(
        start + Duration::from_secs(1),
        Err("curl timed out while contacting github".to_string()),
    );
    match state {
        GithubReviewPollingState::Active(runtime) => {
            assert_eq!(
                runtime.last_error.as_deref(),
                Some("curl timed out while contacting github")
            );
            assert!(
                runtime.status_label().contains("error acme/widgets#42"),
                "unexpected status label: {}",
                runtime.status_label()
            );
        }
        other => panic!("expected active state, got {other:?}"),
    }
}

// Recent-change copy is driven by the polling service's delta, not by every
// event in the snapshot. That keeps old reviews/comments from being re-announced
// while still summarizing the newest change in the shell tail.
#[test]
fn active_state_surfaces_single_recent_change_notice() {
    let config = GithubReviewPollingConfig {
        target: GithubPullRequestTarget::new("acme/widgets", 42),
        interval: Duration::from_secs(30),
    };
    let start = Instant::now();
    let mut state = GithubReviewPollingState::active(config, start);
    let _ = state.take_due_request(start);
    state.record_result(
        start + Duration::from_secs(1),
        Ok(poll_result(
            vec![
                event(
                    201,
                    GithubPullRequestActivityKind::ReviewComment,
                    "2026-04-08T10:30:00Z",
                )
                .with_path("src/adapter/inbound/tui/app/shell_presentation.rs"),
            ],
            vec![
                event(
                    201,
                    GithubPullRequestActivityKind::ReviewComment,
                    "2026-04-08T10:30:00Z",
                )
                .with_path("src/adapter/inbound/tui/app/shell_presentation.rs"),
            ],
        )),
    );
    let GithubReviewPollingState::Active(runtime) = state else {
        panic!("expected active state");
    };
    assert_eq!(
        runtime.status_label(),
        "changes acme/widgets#42 (comment on shell_presentation.rs by reviewer)"
    );
}

#[test]
fn active_state_summarizes_multiple_recent_changes() {
    let config = GithubReviewPollingConfig {
        target: GithubPullRequestTarget::new("acme/widgets", 42),
        interval: Duration::from_secs(30),
    };
    let start = Instant::now();
    let mut state = GithubReviewPollingState::active(config, start);
    let _ = state.take_due_request(start);
    state.record_result(
        start + Duration::from_secs(1),
        Ok(poll_result(
            vec![
                event(
                    100,
                    GithubPullRequestActivityKind::IssueComment,
                    "2026-04-08T09:00:00Z",
                ),
                event(
                    101,
                    GithubPullRequestActivityKind::Review,
                    "2026-04-08T10:00:00Z",
                )
                .with_state("APPROVED"),
            ],
            vec![
                event(
                    100,
                    GithubPullRequestActivityKind::IssueComment,
                    "2026-04-08T09:00:00Z",
                ),
                event(
                    101,
                    GithubPullRequestActivityKind::Review,
                    "2026-04-08T10:00:00Z",
                )
                .with_state("APPROVED"),
            ],
        )),
    );
    let GithubReviewPollingState::Active(runtime) = state else {
        panic!("expected active state");
    };
    assert_eq!(
        runtime.status_label(),
        "changes acme/widgets#42 (2 new; latest approved review by reviewer)"
    );
}

#[test]
fn active_state_clears_recent_change_notice_after_quiet_poll() {
    let config = GithubReviewPollingConfig {
        target: GithubPullRequestTarget::new("acme/widgets", 42),
        interval: Duration::from_secs(30),
    };
    let start = Instant::now();
    let mut state = GithubReviewPollingState::active(config, start);
    let _ = state.take_due_request(start);
    let first_snapshot_event = event(
        101,
        GithubPullRequestActivityKind::Review,
        "2026-04-08T10:00:00Z",
    )
    .with_state("APPROVED");
    state.record_result(
        start + Duration::from_secs(1),
        Ok(poll_result(
            vec![first_snapshot_event.clone()],
            vec![first_snapshot_event.clone()],
        )),
    );
    let second_request = state
        .take_due_request(start + Duration::from_secs(31))
        .expect("follow-up poll should be due");
    assert!(second_request.previous_state.is_some());

    state.record_result(
        start + Duration::from_secs(32),
        Ok(poll_result(vec![first_snapshot_event], Vec::new())),
    );
    let GithubReviewPollingState::Active(runtime) = state else {
        panic!("expected active state");
    };
    assert_eq!(runtime.status_label(), "watching acme/widgets#42");
}

#[test]
fn active_state_exposes_compact_recent_change_summary() {
    let config = GithubReviewPollingConfig {
        target: GithubPullRequestTarget::new("acme/widgets", 42),
        interval: Duration::from_secs(30),
    };
    let start = Instant::now();
    let mut state = GithubReviewPollingState::active(config, start);
    let _ = state.take_due_request(start);
    state.record_result(
        start + Duration::from_secs(1),
        Ok(sample_poll_result("2026-04-08T09:00:00Z")),
    );
    let GithubReviewPollingState::Active(runtime) = state else {
        panic!("expected active state");
    };
    assert_eq!(
        runtime.recent_change_summary(24).as_deref(),
        Some("review commented by r...")
    );
}

// A valid explicit configuration must produce both active state and a service
// handle. Loading through the handle verifies the outbound boundary is wired,
// while the call count proves bootstrap itself did not poll early.
#[test]
fn bootstrap_creates_service_when_configuration_is_valid() {
    let calls = Arc::new(Mutex::new(0usize));
    let target = GithubPullRequestTarget::new("acme/widgets", 42);
    let snapshot = GithubPullRequestActivitySnapshot {
        target: target.clone(),
        title: "Track review state".to_string(),
        url: "https://example.invalid/pr/42".to_string(),
        head_branch: "feature/test".to_string(),
        base_branch: "prerelease".to_string(),
        events: vec![GithubPullRequestActivityEvent {
            id: 100,
            kind: GithubPullRequestActivityKind::Review,
            submitted_at: "2026-04-08T09:00:00Z".to_string(),
            author_login: "reviewer".to_string(),
            body: "Looks good".to_string(),
            state: Some("COMMENTED".to_string()),
            url: "https://example.invalid/pr/42#review-100".to_string(),
            path: None,
        }],
    };
    let bootstrap = GithubReviewPollingBootstrap::from_env_values(
        Some("acme/widgets#42".to_string()),
        Some("15".to_string()),
        || {
            let port: Arc<dyn GithubReviewPollerPort> = Arc::new(FakeGithubReviewPollerPort {
                calls: calls.clone(),
                snapshot: snapshot.clone(),
            });
            Ok(GithubReviewPollerService::new(port))
        },
        Instant::now(),
    );
    let service = bootstrap
        .service
        .expect("service should be present for valid config");
    let snapshot = service
        .load_snapshot(&target)
        .expect("snapshot should load through the service");
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(*calls.lock().expect("calls mutex poisoned"), 1);
}

// Auto-discovery is the no-env path for local review lanes. It should preserve
// the discovered target and requested interval while returning the same service
// shape as explicit configuration.
#[test]
fn bootstrap_auto_discovers_current_branch_pull_request_when_available() {
    let target = GithubPullRequestTarget::new("acme/widgets", 42);
    let bootstrap = GithubReviewPollingBootstrap::from_discovery_result(
        Some("15".to_string()),
        || {
            let port: Arc<dyn GithubReviewPollerPort> = Arc::new(FakeGithubReviewPollerPort {
                calls: Arc::new(Mutex::new(0usize)),
                snapshot: GithubPullRequestActivitySnapshot {
                    target: target.clone(),
                    title: "Track review state".to_string(),
                    url: "https://example.invalid/pr/42".to_string(),
                    head_branch: "feature/test".to_string(),
                    base_branch: "prerelease".to_string(),
                    events: Vec::new(),
                },
            });
            Ok(Some((target.clone(), GithubReviewPollerService::new(port))))
        },
        Instant::now(),
    );
    let GithubReviewPollingState::Active(runtime) = bootstrap.state else {
        panic!("expected active polling state");
    };
    assert_eq!(runtime.config.target, target);
    assert_eq!(runtime.config.interval, Duration::from_secs(15));
    assert!(bootstrap.service.is_some());
}

// Fixtures keep full snapshot events separate from recent changes because the
// runtime stores one for future comparison and shows the other as a one-shot
// notice. Tests can therefore model a quiet poll over unchanged PR state.
fn sample_poll_result(timestamp: &str) -> GithubPullRequestPollResult {
    let target = GithubPullRequestTarget::new("acme/widgets", 42);
    let snapshot = GithubPullRequestActivitySnapshot {
        target,
        title: "Track review state".to_string(),
        url: "https://example.invalid/pr/42".to_string(),
        head_branch: "feature/test".to_string(),
        base_branch: "prerelease".to_string(),
        events: vec![GithubPullRequestActivityEvent {
            id: 100,
            kind: GithubPullRequestActivityKind::Review,
            submitted_at: timestamp.to_string(),
            author_login: "reviewer".to_string(),
            body: "Looks good".to_string(),
            state: Some("COMMENTED".to_string()),
            url: "https://example.invalid/pr/42#review-100".to_string(),
            path: None,
        }],
    };

    GithubPullRequestPollResult {
        next_state: snapshot.poll_state(),
        changes: snapshot.events.clone(),
        snapshot,
    }
}

fn poll_result(
    events: Vec<GithubPullRequestActivityEvent>,
    changes: Vec<GithubPullRequestActivityEvent>,
) -> GithubPullRequestPollResult {
    let snapshot = GithubPullRequestActivitySnapshot {
        target: GithubPullRequestTarget::new("acme/widgets", 42),
        title: "Track review state".to_string(),
        url: "https://example.invalid/pr/42".to_string(),
        head_branch: "feature/test".to_string(),
        base_branch: "prerelease".to_string(),
        events,
    };

    GithubPullRequestPollResult {
        next_state: snapshot.poll_state(),
        changes,
        snapshot,
    }
}

// Event builders keep each scenario focused on the business signal under test:
// activity kind, timestamp ordering, optional file path, and optional review
// state for labels such as "approved review".
fn event(
    id: u64,
    kind: GithubPullRequestActivityKind,
    submitted_at: &str,
) -> GithubPullRequestActivityEvent {
    GithubPullRequestActivityEvent {
        id,
        kind,
        submitted_at: submitted_at.to_string(),
        author_login: "reviewer".to_string(),
        body: "Looks good".to_string(),
        state: None,
        url: format!("https://example.invalid/pr/42#{id}"),
        path: None,
    }
}

// These mutators are test-only builder conveniences. Keeping them local avoids
// adding production builder APIs to the domain event just to make fixtures short.
trait GithubPullRequestActivityEventTestExt {
    fn with_path(self, path: &str) -> Self;
    fn with_state(self, state: &str) -> Self;
}
impl GithubPullRequestActivityEventTestExt for GithubPullRequestActivityEvent {
    fn with_path(mut self, path: &str) -> Self {
        self.path = Some(path.to_string());
        self
    }
    fn with_state(mut self, state: &str) -> Self {
        self.state = Some(state.to_string());
        self
    }
}
