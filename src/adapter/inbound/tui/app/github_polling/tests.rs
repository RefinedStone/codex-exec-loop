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

#[test]
fn bootstrap_env_edges_cover_empty_values_parse_errors_and_loader_failure() {
    let disabled = GithubReviewPollingBootstrap::from_env_values(
        Some("   ".to_string()),
        Some("15".to_string()),
        || unreachable!("service loader should not run for blank PR env"),
        Instant::now(),
    );
    assert!(matches!(disabled.state, GithubReviewPollingState::Disabled));

    for (raw_target, expected) in [
        ("acme/widgets#not-a-number", "numeric PR number"),
        ("acme/widgets#0", "greater than zero"),
    ] {
        let bootstrap = GithubReviewPollingBootstrap::from_env_values(
            Some(raw_target.to_string()),
            None,
            || unreachable!("service loader should not run for invalid target"),
            Instant::now(),
        );
        match bootstrap.state {
            GithubReviewPollingState::SetupError { target, message } => {
                assert!(target.is_none());
                assert!(message.contains(expected), "{message}");
            }
            other => panic!("expected setup error state, got {other:?}"),
        }
    }

    let bad_interval = GithubReviewPollingBootstrap::from_env_values(
        Some("acme/widgets#42".to_string()),
        Some("abc".to_string()),
        || unreachable!("service loader should not run for invalid interval"),
        Instant::now(),
    );
    match bad_interval.state {
        GithubReviewPollingState::SetupError { target, message } => {
            assert_eq!(
                target,
                Some(GithubPullRequestTarget::new("acme/widgets", 42))
            );
            assert!(message.contains("positive whole number"), "{message}");
        }
        other => panic!("expected setup error state, got {other:?}"),
    }

    let zero_interval = GithubReviewPollingBootstrap::from_env_values(
        Some("acme/widgets#42".to_string()),
        Some("0".to_string()),
        || unreachable!("service loader should not run for zero interval"),
        Instant::now(),
    );
    match zero_interval.state {
        GithubReviewPollingState::SetupError { target, message } => {
            assert_eq!(
                target,
                Some(GithubPullRequestTarget::new("acme/widgets", 42))
            );
            assert!(message.contains("greater than zero"), "{message}");
        }
        other => panic!("expected setup error state, got {other:?}"),
    }

    let loader_error = GithubReviewPollingBootstrap::from_env_values(
        Some("acme/widgets#42".to_string()),
        Some("15".to_string()),
        || Err(anyhow::anyhow!("github credentials unavailable")),
        Instant::now(),
    );
    match loader_error.state {
        GithubReviewPollingState::SetupError { target, message } => {
            assert_eq!(
                target,
                Some(GithubPullRequestTarget::new("acme/widgets", 42))
            );
            assert_eq!(message, "github credentials unavailable");
        }
        other => panic!("expected setup error state, got {other:?}"),
    }
}

#[test]
fn bootstrap_discovery_surfaces_interval_and_loader_errors() {
    let bad_interval = GithubReviewPollingBootstrap::from_discovery_result(
        Some("abc".to_string()),
        || unreachable!("discovery loader should not run for invalid interval"),
        Instant::now(),
    );
    match bad_interval.state {
        GithubReviewPollingState::SetupError { target, message } => {
            assert!(target.is_none());
            assert!(message.contains("positive whole number"), "{message}");
        }
        other => panic!("expected setup error state, got {other:?}"),
    }

    let discovery_error = GithubReviewPollingBootstrap::from_discovery_result(
        None,
        || Err(anyhow::anyhow!("gh pr lookup failed")),
        Instant::now(),
    );
    match discovery_error.state {
        GithubReviewPollingState::SetupError { target, message } => {
            assert!(target.is_none());
            assert_eq!(message, "gh pr lookup failed");
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

#[test]
fn state_copy_covers_disabled_setup_starting_polling_and_ignored_updates() {
    let mut disabled = GithubReviewPollingState::Disabled;
    assert_eq!(disabled.status_label(), "off");
    assert!(disabled.recent_change_summary(40).is_none());
    assert!(disabled.take_due_request(Instant::now()).is_none());
    disabled.record_result(
        Instant::now(),
        Err("ignored because polling is disabled".to_string()),
    );
    assert_eq!(disabled.status_label(), "off");

    let setup_with_target = GithubReviewPollingState::SetupError {
        target: Some(GithubPullRequestTarget::new("acme/widgets", 42)),
        message: "   credentials are missing and the message should be trimmed   ".to_string(),
    };
    assert_eq!(
        setup_with_target.status_label(),
        "setup failed acme/widgets#42 (credentials are missing and the message shoul...)"
    );

    let setup_without_target = GithubReviewPollingState::SetupError {
        target: None,
        message: "bad env".to_string(),
    };
    assert_eq!(
        setup_without_target.status_label(),
        "setup failed (bad env)"
    );

    let config = GithubReviewPollingConfig {
        target: GithubPullRequestTarget::new("acme/widgets", 42),
        interval: Duration::from_secs(30),
    };
    let start = Instant::now();
    let mut active = GithubReviewPollingState::active(config, start);
    assert_eq!(active.status_label(), "starting acme/widgets#42");
    assert!(active.take_due_request(start).is_some());
    assert_eq!(active.status_label(), "polling acme/widgets#42");
    assert!(active.take_due_request(start).is_none());
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

#[test]
fn active_state_exposes_multiple_recent_change_summary() {
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
        runtime.recent_change_summary(48).as_deref(),
        Some("2 new, latest review approved by reviewer: Lo...")
    );
}

#[test]
fn parse_helpers_accept_trimmed_values_and_truncate_status_details() {
    let target =
        parse_pull_request_target(" acme/widgets # 42 ").expect("trimmed target should parse");
    assert_eq!(target, GithubPullRequestTarget::new("acme/widgets", 42));
    assert_eq!(
        parse_poll_interval(Some(" 7 ")).unwrap(),
        Duration::from_secs(7)
    );
    assert_eq!(
        parse_poll_interval(Some(" ")).unwrap(),
        Duration::from_secs(DEFAULT_GITHUB_POLL_INTERVAL_SECONDS)
    );
    assert_eq!(truncate_status_detail("  short detail  "), "short detail");

    let long = "x".repeat(80);
    let truncated = truncate_status_detail(&long);
    assert_eq!(truncated.chars().count(), MAX_STATUS_DETAIL_LENGTH);
    assert!(truncated.ends_with("..."));
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
