use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use crate::adapter::outbound::github_review_poller_adapter::GithubReviewPollerAdapter;
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivitySnapshot, GithubPullRequestPollResult,
    GithubPullRequestPollState, GithubPullRequestTarget, truncate_notice_text,
};

use super::{BackgroundMessage, NativeTuiApp};

const GITHUB_PULL_REQUEST_ENV_VAR: &str = "CODEX_EXEC_LOOP_GITHUB_PR";
const GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR: &str = "CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS";
const GITHUB_POLL_BASE_BRANCH: &str = "prerelease";
const DEFAULT_GITHUB_POLL_INTERVAL_SECONDS: u64 = 60;
const MAX_STATUS_DETAIL_LENGTH: usize = 48;

#[derive(Clone)]
pub(super) struct GithubReviewPollingBootstrap {
    pub(super) service: Option<GithubReviewPollerService>,
    pub(super) state: GithubReviewPollingState,
}

impl GithubReviewPollingBootstrap {
    pub(super) fn from_environment(repo_root: &Path, now: Instant) -> Self {
        let pull_request_value = std::env::var(GITHUB_PULL_REQUEST_ENV_VAR).ok();
        let interval_seconds_value = std::env::var(GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR).ok();
        if pull_request_value
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            return Self::from_env_values(
                pull_request_value,
                interval_seconds_value,
                || Self::load_service(repo_root),
                now,
            );
        }

        Self::from_discovery_result(
            interval_seconds_value,
            || Self::load_service_for_current_branch(repo_root),
            now,
        )
    }

    fn from_env_values<F>(
        pull_request_value: Option<String>,
        interval_seconds_value: Option<String>,
        service_loader: F,
        now: Instant,
    ) -> Self
    where
        F: FnOnce() -> Result<GithubReviewPollerService>,
    {
        let Some(raw_target) = pull_request_value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Self {
                service: None,
                state: GithubReviewPollingState::Disabled,
            };
        };

        let target = match parse_pull_request_target(raw_target) {
            Ok(target) => target,
            Err(error) => {
                return Self {
                    service: None,
                    state: GithubReviewPollingState::SetupError {
                        target: None,
                        message: error.to_string(),
                    },
                };
            }
        };
        let interval = match parse_poll_interval(interval_seconds_value.as_deref()) {
            Ok(interval) => interval,
            Err(error) => {
                return Self {
                    service: None,
                    state: GithubReviewPollingState::SetupError {
                        target: Some(target),
                        message: error.to_string(),
                    },
                };
            }
        };

        match service_loader() {
            Ok(service) => Self {
                service: Some(service),
                state: GithubReviewPollingState::active(
                    GithubReviewPollingConfig { target, interval },
                    now,
                ),
            },
            Err(error) => Self {
                service: None,
                state: GithubReviewPollingState::SetupError {
                    target: Some(target),
                    message: error.to_string(),
                },
            },
        }
    }

    fn load_service(repo_root: &Path) -> Result<GithubReviewPollerService> {
        let adapter = GithubReviewPollerAdapter::from_refinedstone_credentials(repo_root)?;
        let port: Arc<dyn GithubReviewPollerPort> = Arc::new(adapter);
        Ok(GithubReviewPollerService::new(port))
    }

    fn from_discovery_result<F>(
        interval_seconds_value: Option<String>,
        discovery_loader: F,
        now: Instant,
    ) -> Self
    where
        F: FnOnce() -> Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>>,
    {
        let interval = match parse_poll_interval(interval_seconds_value.as_deref()) {
            Ok(interval) => interval,
            Err(error) => {
                return Self {
                    service: None,
                    state: GithubReviewPollingState::SetupError {
                        target: None,
                        message: error.to_string(),
                    },
                };
            }
        };

        match discovery_loader() {
            Ok(Some((target, service))) => Self {
                service: Some(service),
                state: GithubReviewPollingState::active(
                    GithubReviewPollingConfig { target, interval },
                    now,
                ),
            },
            Ok(None) => Self {
                service: None,
                state: GithubReviewPollingState::Disabled,
            },
            Err(error) => Self {
                service: None,
                state: GithubReviewPollingState::SetupError {
                    target: None,
                    message: error.to_string(),
                },
            },
        }
    }

    fn load_service_for_current_branch(
        repo_root: &Path,
    ) -> Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>> {
        let adapter = match GithubReviewPollerAdapter::from_refinedstone_credentials(repo_root) {
            Ok(adapter) => adapter,
            Err(_) => return Ok(None),
        };
        let Some(target) = adapter
            .find_open_pull_request_for_current_branch(repo_root, GITHUB_POLL_BASE_BRANCH)?
        else {
            return Ok(None);
        };
        let port: Arc<dyn GithubReviewPollerPort> = Arc::new(adapter);

        Ok(Some((target, GithubReviewPollerService::new(port))))
    }
}

#[derive(Debug, Clone)]
pub(super) enum GithubReviewPollingState {
    Disabled,
    SetupError {
        target: Option<GithubPullRequestTarget>,
        message: String,
    },
    Active(GithubReviewPollingRuntimeState),
}

impl GithubReviewPollingState {
    pub(super) fn active(config: GithubReviewPollingConfig, now: Instant) -> Self {
        Self::Active(GithubReviewPollingRuntimeState::new(config, now))
    }

    pub(super) fn status_label(&self) -> String {
        match self {
            Self::Disabled => "off".to_string(),
            Self::SetupError { target, message } => {
                let detail = truncate_status_detail(message);
                match target {
                    Some(target) => {
                        format!("setup failed {} ({detail})", format_target_label(target))
                    }
                    None => format!("setup failed ({detail})"),
                }
            }
            Self::Active(state) => state.status_label(),
        }
    }

    pub(super) fn recent_change_summary(&self, max_total_len: usize) -> Option<String> {
        let Self::Active(state) = self else {
            return None;
        };
        state.recent_change_summary(max_total_len)
    }

    pub(super) fn take_due_request(&mut self, now: Instant) -> Option<GithubReviewPollRequest> {
        let Self::Active(state) = self else {
            return None;
        };
        state.take_due_request(now)
    }

    pub(super) fn record_result(
        &mut self,
        now: Instant,
        result: Result<GithubPullRequestPollResult, String>,
    ) {
        let Self::Active(state) = self else {
            return;
        };
        state.record_result(now, result);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GithubReviewPollingConfig {
    pub(super) target: GithubPullRequestTarget,
    pub(super) interval: Duration,
}

#[derive(Debug, Clone)]
pub(super) struct GithubReviewPollingRuntimeState {
    pub(super) config: GithubReviewPollingConfig,
    pub(super) snapshot: Option<GithubPullRequestActivitySnapshot>,
    pub(super) recent_changes: Vec<GithubPullRequestActivityEvent>,
    recent_change_notice: Option<String>,
    pub(super) poll_state: Option<GithubPullRequestPollState>,
    pub(super) last_error: Option<String>,
    next_poll_at: Instant,
    poll_in_flight: bool,
}

impl GithubReviewPollingRuntimeState {
    fn new(config: GithubReviewPollingConfig, now: Instant) -> Self {
        Self {
            config,
            snapshot: None,
            recent_changes: Vec::new(),
            recent_change_notice: None,
            poll_state: None,
            last_error: None,
            next_poll_at: now,
            poll_in_flight: false,
        }
    }

    fn status_label(&self) -> String {
        let target = format_target_label(&self.config.target);
        if self.poll_in_flight {
            return format!("polling {target}");
        }
        if let Some(error) = self.last_error.as_deref() {
            return format!("error {target} ({})", truncate_status_detail(error));
        }
        if self.snapshot.is_none() {
            return format!("starting {target}");
        }
        if let Some(notice) = self.recent_change_notice.as_deref() {
            return format!("changes {target} ({})", truncate_status_detail(notice));
        }

        format!("watching {target}")
    }

    fn build_recent_change_notice(
        recent_changes: &[GithubPullRequestActivityEvent],
    ) -> Option<String> {
        let latest_change = recent_changes.last()?;
        if recent_changes.len() == 1 {
            return Some(latest_change.notice_label());
        }

        Some(format!(
            "{} new; latest {}",
            recent_changes.len(),
            latest_change.notice_label()
        ))
    }
    fn recent_change_summary(&self, max_total_len: usize) -> Option<String> {
        let latest_change = self.recent_changes.last()?;
        if self.recent_changes.len() == 1 {
            Some(latest_change.notice_summary(max_total_len))
        } else {
            Some(truncate_notice_text(
                &format!(
                    "{} new, latest {}",
                    self.recent_changes.len(),
                    latest_change.notice_summary(max_total_len)
                ),
                max_total_len,
            ))
        }
    }

    fn take_due_request(&mut self, now: Instant) -> Option<GithubReviewPollRequest> {
        if self.poll_in_flight || now < self.next_poll_at {
            return None;
        }

        self.poll_in_flight = true;
        Some(GithubReviewPollRequest {
            target: self.config.target.clone(),
            previous_state: self.poll_state.clone(),
        })
    }

    fn record_result(&mut self, now: Instant, result: Result<GithubPullRequestPollResult, String>) {
        self.poll_in_flight = false;
        self.next_poll_at = now + self.config.interval;

        match result {
            Ok(result) => {
                let recent_change_notice = Self::build_recent_change_notice(&result.changes);
                self.snapshot = Some(result.snapshot);
                self.recent_changes = result.changes;
                self.recent_change_notice = recent_change_notice;
                self.poll_state = Some(result.next_state);
                self.last_error = None;
            }
            Err(error) => {
                self.recent_changes.clear();
                self.recent_change_notice = None;
                self.last_error = Some(error);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct GithubReviewPollRequest {
    target: GithubPullRequestTarget,
    previous_state: Option<GithubPullRequestPollState>,
}

impl NativeTuiApp {
    pub(super) fn configure_github_review_polling(
        &mut self,
        bootstrap: GithubReviewPollingBootstrap,
    ) {
        self.github_review_poller_service = bootstrap.service;
        self.github_review_polling_state = bootstrap.state;
    }

    pub(super) fn github_review_polling_status_label(&self) -> String {
        self.github_review_polling_state.status_label()
    }

    pub(super) fn github_review_recent_changes_summary(
        &self,
        max_total_len: usize,
    ) -> Option<String> {
        self.github_review_polling_state
            .recent_change_summary(max_total_len)
    }

    pub(super) fn maybe_start_github_review_poll(&mut self, now: Instant) {
        let Some(request) = self.github_review_polling_state.take_due_request(now) else {
            return;
        };
        let Some(service) = self.github_review_poller_service.clone() else {
            return;
        };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = service
                .poll(&request.target, request.previous_state.as_ref())
                .map_err(|error| error.to_string());
            let _ = tx.send(BackgroundMessage::GithubReviewPollLoaded(result));
        });
    }

    pub(super) fn record_github_review_poll_result(
        &mut self,
        now: Instant,
        result: Result<GithubPullRequestPollResult, String>,
    ) {
        self.github_review_polling_state.record_result(now, result);
    }
}

fn parse_pull_request_target(value: &str) -> Result<GithubPullRequestTarget> {
    let Some((repository, number_text)) = value.trim().split_once('#') else {
        bail!("{GITHUB_PULL_REQUEST_ENV_VAR} must look like owner/repo#123, got {value}");
    };
    let repository = repository.trim();
    let repository_parts = repository.split('/').collect::<Vec<_>>();
    if repository_parts.len() != 2
        || repository_parts[0].is_empty()
        || repository_parts[1].is_empty()
    {
        bail!("{GITHUB_PULL_REQUEST_ENV_VAR} must look like owner/repo#123, got {value}");
    }

    let number = number_text
        .trim()
        .parse::<u64>()
        .map_err(|_| anyhow!("{GITHUB_PULL_REQUEST_ENV_VAR} must use a numeric PR number"))?;
    if number == 0 {
        bail!("{GITHUB_PULL_REQUEST_ENV_VAR} must use a PR number greater than zero");
    }

    Ok(GithubPullRequestTarget::new(repository, number))
}

fn parse_poll_interval(value: Option<&str>) -> Result<Duration> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Duration::from_secs(DEFAULT_GITHUB_POLL_INTERVAL_SECONDS));
    };
    let seconds = value.parse::<u64>().map_err(|_| {
        anyhow!("{GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR} must be a positive whole number")
    })?;
    if seconds == 0 {
        bail!("{GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR} must be greater than zero");
    }

    Ok(Duration::from_secs(seconds))
}

fn truncate_status_detail(message: &str) -> String {
    let message = message.trim();
    if message.chars().count() <= MAX_STATUS_DETAIL_LENGTH {
        return message.to_string();
    }

    let mut truncated: String = message.chars().take(MAX_STATUS_DETAIL_LENGTH - 3).collect();
    truncated.push_str("...");
    truncated
}

fn format_target_label(target: &GithubPullRequestTarget) -> String {
    format!("{}#{}", target.repository, target.number)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use anyhow::Result;

    use super::*;
    use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
    use crate::domain::github_review::{
        GithubPullRequestActivityKind, GithubPullRequestActivitySnapshot,
    };

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
                    .with_path("native/src/adapter/inbound/tui/app/shell_presentation.rs"),
                ],
                vec![
                    event(
                        201,
                        GithubPullRequestActivityKind::ReviewComment,
                        "2026-04-08T10:30:00Z",
                    )
                    .with_path("native/src/adapter/inbound/tui/app/shell_presentation.rs"),
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
}
