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
    GithubPullRequestPollState, GithubPullRequestTarget,
};

use super::{BackgroundMessage, NativeTuiApp};

const GITHUB_PULL_REQUEST_ENV_VAR: &str = "CODEX_EXEC_LOOP_GITHUB_PR";
const GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR: &str = "CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS";
const DEFAULT_GITHUB_POLL_INTERVAL_SECONDS: u64 = 60;
const MAX_STATUS_DETAIL_LENGTH: usize = 48;

#[derive(Clone)]
pub(super) struct GithubReviewPollingBootstrap {
    pub(super) service: Option<GithubReviewPollerService>,
    pub(super) state: GithubReviewPollingState,
}

impl GithubReviewPollingBootstrap {
    pub(super) fn from_environment(repo_root: &Path, now: Instant) -> Self {
        Self::from_env_values(
            std::env::var(GITHUB_PULL_REQUEST_ENV_VAR).ok(),
            std::env::var(GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR).ok(),
            || Self::load_service(repo_root),
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

        format!("watching {target}")
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
                self.snapshot = Some(result.snapshot);
                self.recent_changes = result.changes;
                self.poll_state = Some(result.next_state);
                self.last_error = None;
            }
            Err(error) => {
                self.recent_changes.clear();
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
    if repository.is_empty() || !repository.contains('/') {
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

    let mut truncated = String::new();
    for character in message.chars().take(MAX_STATUS_DETAIL_LENGTH - 3) {
        truncated.push(character);
    }
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
        let bootstrap = GithubReviewPollingBootstrap::from_env_values(
            None,
            None,
            || unreachable!("service loader should not run"),
            Instant::now(),
        );

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
}
