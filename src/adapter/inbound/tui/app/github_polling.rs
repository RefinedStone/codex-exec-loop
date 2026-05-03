use super::{BackgroundMessage, NativeTuiApp};
use crate::adapter::outbound::github::GithubReviewPollerAdapter;
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivitySnapshot, GithubPullRequestPollResult,
    GithubPullRequestPollState, GithubPullRequestTarget, truncate_notice_text,
};
use anyhow::{Result, anyhow, bail};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// GitHub review polling is an optional shell-side watcher for the active PR
// lane. Explicit env configuration wins, otherwise the adapter may discover an
// open PR for the current branch against prerelease.
const GITHUB_PULL_REQUEST_ENV_VAR: &str = "CODEX_EXEC_LOOP_GITHUB_PR";
const GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR: &str = "CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS";
const GITHUB_POLL_BASE_BRANCH: &str = "prerelease";
const DEFAULT_GITHUB_POLL_INTERVAL_SECONDS: u64 = 60;
const MAX_STATUS_DETAIL_LENGTH: usize = 48;

// Bootstrap returns both the service handle and the initial reducer state so
// NativeTuiApp can keep setup failures visible without keeping a half-built
// outbound adapter around.
#[derive(Clone)]
pub(super) struct GithubReviewPollingBootstrap {
    pub(super) service: Option<GithubReviewPollerService>,
    pub(super) state: GithubReviewPollingState,
}
impl GithubReviewPollingBootstrap {
    pub(super) fn from_environment(repo_root: &Path, now: Instant) -> Self {
        let pull_request_value = std::env::var(GITHUB_PULL_REQUEST_ENV_VAR).ok();
        let interval_seconds_value = std::env::var(GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR).ok();

        // Explicit PR configuration is deterministic for CI and review lanes;
        // branch discovery is best-effort and disables itself when credentials
        // or a matching open PR are absent.
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
        // Missing local credentials are not a TUI setup error during automatic
        // discovery because most local runs should remain quiet unless polling
        // was explicitly requested.
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

// The state machine is deliberately TUI-facing: Disabled is quiet, SetupError
// is operator-visible, and Active owns scheduling plus delta state.
#[derive(Debug, Clone)]
pub(super) enum GithubReviewPollingState {
    Disabled,
    SetupError {
        target: Option<GithubPullRequestTarget>,
        message: String,
    },
    Active(Box<GithubReviewPollingRuntimeState>),
}
impl GithubReviewPollingState {
    pub(super) fn active(config: GithubReviewPollingConfig, now: Instant) -> Self {
        Self::Active(Box::new(GithubReviewPollingRuntimeState::new(config, now)))
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

// Runtime state separates full snapshot, previous poll_state, one-shot recent
// change notices, and durable last_error so polling can retry without
// re-announcing old review activity.
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
            // The first poll starts immediately after bootstrap; later polls
            // are spaced from record_result so slow requests do not overlap.
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

        // Mark the request before spawning the worker. The TUI tick loop can
        // call this method frequently, so the in-flight bit is the concurrency
        // guard rather than the background thread handle.
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
                // The service returns deltas relative to poll_state; the TUI
                // keeps both a compact status notice and the full latest
                // change list for the shell banner.
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

// NativeTuiApp owns only the runtime bridge: it decides when a request is due,
// spawns the service call off the render path, and feeds the result back through
// the same background-message reducer used by the rest of the TUI.
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
    pub(super) fn maybe_start_github_review_poll(&mut self, now: Instant) -> bool {
        let Some(request) = self.github_review_polling_state.take_due_request(now) else {
            return false;
        };
        let Some(service) = self.github_review_poller_service.clone() else {
            // Bootstrap keeps Active state paired with a service. Reaching this
            // path means the app state was mutated out of band, so do not report
            // that a worker was started.
            return false;
        };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = service
                .poll(&request.target, request.previous_state.as_ref())
                .map_err(|error| error.to_string());
            let _ = tx.send(BackgroundMessage::GithubReviewPollLoaded(result));
        });
        true
    }
    pub(super) fn record_github_review_poll_result(
        &mut self,
        now: Instant,
        result: Result<GithubPullRequestPollResult, String>,
    ) {
        self.github_review_polling_state.record_result(now, result);
    }
}

// Env configuration is intentionally narrow and human-readable because it is
// used in ad hoc review lanes as well as tests: owner/repo#number plus an
// optional positive second interval.
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

// Status labels share a single terminal row with shell state, so diagnostics are
// clipped at the boundary instead of relying on caller-specific truncation.
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
#[path = "github_polling/tests.rs"]
mod tests;
