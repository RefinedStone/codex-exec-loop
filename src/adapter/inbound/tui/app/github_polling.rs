use super::NativeTuiApp;
use crate::core::app::{
    AppCommand, AppEvent, CoreInput, GithubReviewPollCorrelation,
    GithubReviewPollingSetupCorrelation, GithubReviewPollingSetupMode,
    GithubReviewPollingSetupRequest, GithubReviewPollingSetupResult,
};
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivitySnapshot, GithubPullRequestPollResult,
    GithubPullRequestTarget, truncate_notice_text,
};
use anyhow::{Result, anyhow, bail};
use std::time::{Duration, Instant};

// GitHub review polling is an optional shell-side watcher for the active PR
// lane. Explicit env configuration wins, otherwise the adapter may discover an
// open PR for the current branch against the parallel-mode integration branch.
const GITHUB_PULL_REQUEST_ENV_VAR: &str = "CODEX_EXEC_LOOP_GITHUB_PR";
const GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR: &str = "CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS";
const DEFAULT_GITHUB_POLL_INTERVAL_SECONDS: u64 = 60;
const MAX_STATUS_DETAIL_LENGTH: usize = 48;

// Bootstrap parses process configuration only. Git, GitHub credential, and
// network discovery are deferred to a Core effect after the first delivered frame.
#[derive(Clone)]
pub(super) struct GithubReviewPollingBootstrap {
    pub(super) state: GithubReviewPollingState,
}
impl GithubReviewPollingBootstrap {
    pub(super) fn from_environment() -> Self {
        let pull_request_value = std::env::var(GITHUB_PULL_REQUEST_ENV_VAR).ok();
        let interval_seconds_value = std::env::var(GITHUB_POLL_INTERVAL_SECONDS_ENV_VAR).ok();
        Self::parse_env_values(pull_request_value, interval_seconds_value)
    }

    #[cfg(test)]
    pub(super) fn from_env_values(
        pull_request_value: Option<String>,
        interval_seconds_value: Option<String>,
    ) -> Self {
        Self::parse_env_values(pull_request_value, interval_seconds_value)
    }

    fn parse_env_values(
        pull_request_value: Option<String>,
        interval_seconds_value: Option<String>,
    ) -> Self {
        let explicit_target = match pull_request_value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(parse_pull_request_target)
            .transpose()
        {
            Ok(target) => target,
            Err(error) => {
                return Self {
                    state: GithubReviewPollingState::SetupError {
                        config: None,
                        workspace_directory: None,
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
                    state: GithubReviewPollingState::SetupError {
                        config: None,
                        workspace_directory: None,
                        target: explicit_target,
                        message: error.to_string(),
                    },
                };
            }
        };
        let setup_mode = match explicit_target {
            Some(target) => GithubReviewPollingSetupMode::Explicit { target },
            None => GithubReviewPollingSetupMode::Discover,
        };
        Self {
            state: GithubReviewPollingState::PendingFirstFrame {
                config: GithubReviewPollingEnvironmentConfig {
                    setup_mode,
                    interval,
                },
            },
        }
    }

    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self {
            state: GithubReviewPollingState::Disabled {
                config: None,
                workspace_directory: None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GithubReviewPollingEnvironmentConfig {
    pub(super) setup_mode: GithubReviewPollingSetupMode,
    pub(super) interval: Duration,
}

// Setup is UI-visible, while Core owns its generation/workspace admission and
// composition owns provider execution.
#[derive(Debug, Clone)]
pub(super) enum GithubReviewPollingState {
    PendingFirstFrame {
        config: GithubReviewPollingEnvironmentConfig,
    },
    Discovering {
        config: GithubReviewPollingEnvironmentConfig,
        correlation: GithubReviewPollingSetupCorrelation,
    },
    Disabled {
        config: Option<GithubReviewPollingEnvironmentConfig>,
        workspace_directory: Option<String>,
    },
    SetupError {
        config: Option<GithubReviewPollingEnvironmentConfig>,
        workspace_directory: Option<String>,
        target: Option<GithubPullRequestTarget>,
        message: String,
    },
    Active(Box<GithubReviewPollingRuntimeState>),
}
impl GithubReviewPollingState {
    fn active_with_setup(
        environment_config: GithubReviewPollingEnvironmentConfig,
        setup_correlation: GithubReviewPollingSetupCorrelation,
        target: GithubPullRequestTarget,
        now: Instant,
    ) -> Self {
        Self::Active(Box::new(GithubReviewPollingRuntimeState::new(
            GithubReviewPollingConfig {
                target,
                interval: environment_config.interval,
            },
            environment_config,
            setup_correlation,
            now,
        )))
    }

    #[cfg(test)]
    pub(super) fn active(config: GithubReviewPollingConfig, now: Instant) -> Self {
        let target = config.target.clone();
        Self::active_with_setup(
            GithubReviewPollingEnvironmentConfig {
                setup_mode: GithubReviewPollingSetupMode::Explicit {
                    target: target.clone(),
                },
                interval: config.interval,
            },
            GithubReviewPollingSetupCorrelation::new(1, "/workspace"),
            target,
            now,
        )
    }
    pub(super) fn status_label(&self) -> String {
        match self {
            Self::PendingFirstFrame { .. } => "setup pending".to_string(),
            Self::Discovering { .. } => "discovering".to_string(),
            Self::Disabled { .. } => "off".to_string(),
            Self::SetupError {
                target, message, ..
            } => {
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
    fn environment_config(&self) -> Option<&GithubReviewPollingEnvironmentConfig> {
        match self {
            Self::PendingFirstFrame { config } | Self::Discovering { config, .. } => Some(config),
            Self::Disabled { config, .. } | Self::SetupError { config, .. } => config.as_ref(),
            Self::Active(state) => Some(&state.environment_config),
        }
    }

    fn configured_workspace_directory(&self) -> Option<&str> {
        match self {
            Self::PendingFirstFrame { .. } => None,
            Self::Discovering { correlation, .. } => Some(correlation.workspace_directory.as_str()),
            Self::Disabled {
                workspace_directory,
                ..
            }
            | Self::SetupError {
                workspace_directory,
                ..
            } => workspace_directory.as_deref(),
            Self::Active(state) => Some(state.setup_correlation.workspace_directory.as_str()),
        }
    }

    fn setup_request_for_workspace(
        &self,
        workspace_directory: &str,
    ) -> Option<GithubReviewPollingSetupRequest> {
        let config = self.environment_config()?;
        if self.configured_workspace_directory() == Some(workspace_directory) {
            return None;
        }
        Some(GithubReviewPollingSetupRequest::new(
            workspace_directory,
            config.setup_mode.clone(),
        ))
    }

    fn record_setup_started(&mut self, correlation: GithubReviewPollingSetupCorrelation) {
        let Some(config) = self.environment_config().cloned() else {
            return;
        };
        *self = Self::Discovering {
            config,
            correlation,
        };
    }

    fn record_setup_completion(
        &mut self,
        now: Instant,
        correlation: GithubReviewPollingSetupCorrelation,
        result: Result<GithubReviewPollingSetupResult, String>,
    ) {
        let Self::Discovering {
            config,
            correlation: pending,
        } = self
        else {
            return;
        };
        if pending != &correlation {
            return;
        }
        let config = config.clone();
        let explicit_target = config.setup_mode.explicit_target().cloned();
        *self = match result {
            Ok(GithubReviewPollingSetupResult::Active { target }) => {
                Self::active_with_setup(config, correlation, target, now)
            }
            Ok(GithubReviewPollingSetupResult::Disabled) => Self::Disabled {
                config: Some(config),
                workspace_directory: Some(correlation.workspace_directory),
            },
            Err(message) => Self::SetupError {
                config: Some(config),
                workspace_directory: Some(correlation.workspace_directory),
                target: explicit_target,
                message,
            },
        };
    }

    pub(super) fn poll_due(&self, now: Instant) -> bool {
        let Self::Active(state) = self else {
            return false;
        };
        state.poll_due(now)
    }

    pub(super) fn record_poll_started(&mut self, correlation: GithubReviewPollCorrelation) {
        let Self::Active(state) = self else {
            return;
        };
        state.record_poll_started(correlation);
    }

    pub(super) fn record_poll_completion(
        &mut self,
        now: Instant,
        correlation: GithubReviewPollCorrelation,
        result: Result<GithubPullRequestPollResult, String>,
    ) {
        let Self::Active(state) = self else {
            return;
        };
        state.record_poll_completion(now, correlation, result);
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GithubReviewPollingConfig {
    pub(super) target: GithubPullRequestTarget,
    pub(super) interval: Duration,
}

// Runtime state separates the full snapshot, one-shot recent change notices,
// and durable last_error so polling can retry without re-announcing old review
// activity. Core owns the poll cursor and single-flight admission.
#[derive(Debug, Clone)]
pub(super) struct GithubReviewPollingRuntimeState {
    pub(super) config: GithubReviewPollingConfig,
    pub(super) environment_config: GithubReviewPollingEnvironmentConfig,
    pub(super) setup_correlation: GithubReviewPollingSetupCorrelation,
    pub(super) snapshot: Option<GithubPullRequestActivitySnapshot>,
    pub(super) recent_changes: Vec<GithubPullRequestActivityEvent>,
    recent_change_notice: Option<String>,
    pub(super) last_error: Option<String>,
    next_poll_at: Instant,
    pub(super) pending_correlation: Option<GithubReviewPollCorrelation>,
}
impl GithubReviewPollingRuntimeState {
    fn new(
        config: GithubReviewPollingConfig,
        environment_config: GithubReviewPollingEnvironmentConfig,
        setup_correlation: GithubReviewPollingSetupCorrelation,
        now: Instant,
    ) -> Self {
        Self {
            config,
            environment_config,
            setup_correlation,
            snapshot: None,
            recent_changes: Vec::new(),
            recent_change_notice: None,
            last_error: None,
            // The first poll starts immediately after bootstrap; later polls
            // are spaced from completion so slow requests do not overlap.
            next_poll_at: now,
            pending_correlation: None,
        }
    }
    fn status_label(&self) -> String {
        let target = format_target_label(&self.config.target);
        if self.pending_correlation.is_some() {
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
    fn poll_due(&self, now: Instant) -> bool {
        self.pending_correlation.is_none() && now >= self.next_poll_at
    }

    fn record_poll_started(&mut self, correlation: GithubReviewPollCorrelation) {
        if correlation.target == self.config.target && self.pending_correlation.is_none() {
            self.pending_correlation = Some(correlation);
        }
    }

    fn record_poll_completion(
        &mut self,
        now: Instant,
        correlation: GithubReviewPollCorrelation,
        result: Result<GithubPullRequestPollResult, String>,
    ) {
        if self.pending_correlation.as_ref() != Some(&correlation) {
            return;
        }
        self.pending_correlation = None;
        self.next_poll_at = now + self.config.interval;
        match result {
            Ok(result) => {
                // Core supplies deltas relative to its cursor; the TUI keeps a
                // compact status notice and the full latest change list.
                let recent_change_notice = Self::build_recent_change_notice(&result.changes);
                self.snapshot = Some(result.snapshot);
                self.recent_changes = result.changes;
                self.recent_change_notice = recent_change_notice;
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

// NativeTuiApp owns the interval and presentation projection. Core owns poll
// admission, cursor continuity, and provider execution.
impl NativeTuiApp {
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

    pub(super) fn maybe_start_github_review_polling_setup(
        &mut self,
        workspace_directory: &str,
    ) -> bool {
        let Some(request) = self
            .github_review_polling_state
            .setup_request_for_workspace(workspace_directory)
        else {
            return false;
        };
        let outcome = self
            .client_runtime
            .dispatch_client_event(CoreInput::Command(AppCommand::SetupGithubReviewPolling(
                request,
            )));
        let started = outcome
            .events
            .iter()
            .any(|event| matches!(event, AppEvent::GithubReviewPollingSetupStarted { .. }));
        self.apply_core_dispatch_outcome(outcome);
        started
    }

    pub(super) fn record_github_review_polling_setup_started(
        &mut self,
        correlation: GithubReviewPollingSetupCorrelation,
    ) {
        self.github_review_polling_state
            .record_setup_started(correlation);
    }

    pub(super) fn record_github_review_polling_setup_completion(
        &mut self,
        now: Instant,
        correlation: GithubReviewPollingSetupCorrelation,
        result: Result<GithubReviewPollingSetupResult, String>,
    ) {
        self.github_review_polling_state
            .record_setup_completion(now, correlation, result);
    }

    pub(super) fn maybe_start_github_review_poll(&mut self, now: Instant) -> bool {
        if !self.github_review_polling_state.poll_due(now) {
            return false;
        }
        let outcome = self
            .client_runtime
            .dispatch_client_event(CoreInput::Command(AppCommand::PollGithubReview));
        let started = outcome
            .events
            .iter()
            .any(|event| matches!(event, AppEvent::GithubReviewPollStarted { .. }));
        self.apply_core_dispatch_outcome(outcome);
        started
    }

    pub(super) fn record_github_review_poll_started(
        &mut self,
        correlation: GithubReviewPollCorrelation,
    ) {
        self.github_review_polling_state
            .record_poll_started(correlation);
    }

    pub(super) fn record_github_review_poll_completion(
        &mut self,
        now: Instant,
        correlation: GithubReviewPollCorrelation,
        result: Result<GithubPullRequestPollResult, String>,
    ) {
        self.github_review_polling_state
            .record_poll_completion(now, correlation, result);
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
