use super::*;
use crate::domain::github_review::{
    GithubPullRequestActivityKind, GithubPullRequestActivitySnapshot,
};

fn poll_correlation(generation: u64) -> GithubReviewPollCorrelation {
    GithubReviewPollCorrelation::new(generation, GithubPullRequestTarget::new("acme/widgets", 42))
}

fn start_poll(
    state: &mut GithubReviewPollingState,
    now: Instant,
    generation: u64,
) -> GithubReviewPollCorrelation {
    assert!(state.poll_due(now));
    let correlation = poll_correlation(generation);
    state.record_poll_started(correlation.clone());
    correlation
}

// Bootstrap is deliberately pure: it parses environment values into a pending
// setup description and cannot touch Git, credentials, or GitHub.
#[test]
fn bootstrap_without_pull_request_env_waits_for_first_frame_discovery() {
    let bootstrap = GithubReviewPollingBootstrap::from_env_values(None, None);

    let GithubReviewPollingState::PendingFirstFrame { config } = bootstrap.state else {
        panic!("expected pending first-frame state");
    };
    assert_eq!(config.setup_mode, GithubReviewPollingSetupMode::Discover);
    assert_eq!(
        config.interval,
        Duration::from_secs(DEFAULT_GITHUB_POLL_INTERVAL_SECONDS)
    );
}

#[test]
fn bootstrap_explicit_target_waits_for_first_frame() {
    let bootstrap = GithubReviewPollingBootstrap::from_env_values(
        Some(" acme/widgets # 42 ".to_string()),
        Some("15".to_string()),
    );
    let GithubReviewPollingState::PendingFirstFrame { config } = bootstrap.state else {
        panic!("expected pending first-frame state");
    };
    assert_eq!(
        config.setup_mode,
        GithubReviewPollingSetupMode::Explicit {
            target: GithubPullRequestTarget::new("acme/widgets", 42),
        }
    );
    assert_eq!(config.interval, Duration::from_secs(15));
}

#[test]
fn bootstrap_surfaces_parse_errors_without_starting_setup() {
    for (raw_target, expected) in [
        ("not-a-pr", "owner/repo#123"),
        ("owner/repo/extra#42", "owner/repo#123"),
        ("acme/widgets#not-a-number", "numeric PR number"),
        ("acme/widgets#0", "greater than zero"),
    ] {
        let bootstrap =
            GithubReviewPollingBootstrap::from_env_values(Some(raw_target.to_string()), None);
        match bootstrap.state {
            GithubReviewPollingState::SetupError {
                target, message, ..
            } => {
                assert!(target.is_none());
                assert!(message.contains(expected), "{message}");
            }
            other => panic!("expected setup error state, got {other:?}"),
        }
    }

    let bad_interval = GithubReviewPollingBootstrap::from_env_values(
        Some("acme/widgets#42".to_string()),
        Some("abc".to_string()),
    );
    match bad_interval.state {
        GithubReviewPollingState::SetupError {
            target, message, ..
        } => {
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
    );
    match zero_interval.state {
        GithubReviewPollingState::SetupError {
            target, message, ..
        } => {
            assert_eq!(
                target,
                Some(GithubPullRequestTarget::new("acme/widgets", 42))
            );
            assert!(message.contains("greater than zero"), "{message}");
        }
        other => panic!("expected setup error state, got {other:?}"),
    }
}

#[test]
fn state_transitions_pending_to_discovering_to_terminal_outcomes() {
    let start = Instant::now();
    let mut state =
        GithubReviewPollingBootstrap::from_env_values(None, Some("15".to_string())).state;
    let request = state
        .setup_request_for_workspace("/workspace-a")
        .expect("pending state should request setup");
    assert_eq!(request.mode, GithubReviewPollingSetupMode::Discover);

    let correlation = GithubReviewPollingSetupCorrelation::new(1, "/workspace-a");
    state.record_setup_started(correlation.clone());
    assert_eq!(state.status_label(), "discovering");
    assert!(state.setup_request_for_workspace("/workspace-a").is_none());

    state.record_setup_completion(
        start,
        GithubReviewPollingSetupCorrelation::new(2, "/workspace-a"),
        Ok(GithubReviewPollingSetupResult::Disabled),
    );
    assert_eq!(
        state.status_label(),
        "discovering",
        "stale completion ignored"
    );

    state.record_setup_completion(
        start,
        correlation,
        Ok(GithubReviewPollingSetupResult::Disabled),
    );
    assert_eq!(state.status_label(), "off");
    assert!(state.setup_request_for_workspace("/workspace-a").is_none());
    assert!(state.setup_request_for_workspace("/workspace-b").is_some());
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
    assert!(state.poll_due(start));
    assert!(state.poll_due(start), "due checks must not self-admit");
    let first_correlation = start_poll(&mut state, start, 1);
    assert!(!state.poll_due(start));

    state.record_poll_completion(
        start + Duration::from_millis(500),
        poll_correlation(2),
        Err("stale completion".to_string()),
    );
    let GithubReviewPollingState::Active(runtime) = &state else {
        panic!("expected active state");
    };
    assert_eq!(
        runtime.pending_correlation.as_ref(),
        Some(&first_correlation)
    );
    assert!(runtime.last_error.is_none());

    state.record_poll_completion(
        start + Duration::from_secs(1),
        first_correlation,
        Ok(sample_poll_result("2026-04-08T09:00:00Z")),
    );

    assert!(!state.poll_due(start + Duration::from_secs(15)));
    assert!(state.poll_due(start + Duration::from_secs(31)));
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
    let correlation = start_poll(&mut state, start, 1);
    state.record_poll_completion(
        start + Duration::from_secs(1),
        correlation,
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
    let mut disabled = GithubReviewPollingState::Disabled {
        config: None,
        workspace_directory: None,
    };
    assert_eq!(disabled.status_label(), "off");
    assert!(disabled.recent_change_summary(40).is_none());
    assert!(!disabled.poll_due(Instant::now()));
    disabled.record_poll_completion(
        Instant::now(),
        poll_correlation(1),
        Err("ignored because polling is disabled".to_string()),
    );
    assert_eq!(disabled.status_label(), "off");

    let setup_with_target = GithubReviewPollingState::SetupError {
        config: None,
        workspace_directory: None,
        target: Some(GithubPullRequestTarget::new("acme/widgets", 42)),
        message: "   credentials are missing and the message should be trimmed   ".to_string(),
    };
    assert_eq!(
        setup_with_target.status_label(),
        "setup failed acme/widgets#42 (credentials are missing and the message shoul...)"
    );

    let setup_without_target = GithubReviewPollingState::SetupError {
        config: None,
        workspace_directory: None,
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
    assert!(active.poll_due(start));
    assert_eq!(active.status_label(), "starting acme/widgets#42");
    active.record_poll_started(poll_correlation(1));
    assert_eq!(active.status_label(), "polling acme/widgets#42");
    assert!(!active.poll_due(start));
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
    let correlation = start_poll(&mut state, start, 1);
    state.record_poll_completion(
        start + Duration::from_secs(1),
        correlation,
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
    let correlation = start_poll(&mut state, start, 1);
    state.record_poll_completion(
        start + Duration::from_secs(1),
        correlation,
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
    let first_correlation = start_poll(&mut state, start, 1);
    let first_snapshot_event = event(
        101,
        GithubPullRequestActivityKind::Review,
        "2026-04-08T10:00:00Z",
    )
    .with_state("APPROVED");
    state.record_poll_completion(
        start + Duration::from_secs(1),
        first_correlation,
        Ok(poll_result(
            vec![first_snapshot_event.clone()],
            vec![first_snapshot_event.clone()],
        )),
    );
    let second_correlation = start_poll(&mut state, start + Duration::from_secs(31), 2);

    state.record_poll_completion(
        start + Duration::from_secs(32),
        second_correlation,
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
    let correlation = start_poll(&mut state, start, 1);
    state.record_poll_completion(
        start + Duration::from_secs(1),
        correlation,
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
    let correlation = start_poll(&mut state, start, 1);
    state.record_poll_completion(
        start + Duration::from_secs(1),
        correlation,
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
