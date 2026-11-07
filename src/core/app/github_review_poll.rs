use crate::domain::github_review::GithubPullRequestTarget;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubReviewPollingSetupMode {
    Explicit { target: GithubPullRequestTarget },
    Discover,
}

impl GithubReviewPollingSetupMode {
    pub fn explicit_target(&self) -> Option<&GithubPullRequestTarget> {
        match self {
            Self::Explicit { target } => Some(target),
            Self::Discover => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubReviewPollingSetupRequest {
    pub workspace_directory: String,
    pub mode: GithubReviewPollingSetupMode,
    // core 가 tick 에서 다음 폴링 시각을 정할 때 쓴다 (plan 18 K1-1).
    pub poll_interval: Duration,
}

impl GithubReviewPollingSetupRequest {
    pub fn new(
        workspace_directory: impl Into<String>,
        mode: GithubReviewPollingSetupMode,
        poll_interval: Duration,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            mode,
            poll_interval,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubReviewPollingSetupCorrelation {
    pub generation: u64,
    pub workspace_directory: String,
}

impl GithubReviewPollingSetupCorrelation {
    pub fn new(generation: u64, workspace_directory: impl Into<String>) -> Self {
        Self {
            generation,
            workspace_directory: workspace_directory.into(),
        }
    }
}

pub(crate) fn github_review_polling_target_is_valid(target: &GithubPullRequestTarget) -> bool {
    let mut repository_parts = target.repository.split('/');
    repository_parts
        .next()
        .is_some_and(|owner| !owner.is_empty())
        && repository_parts.next().is_some_and(|name| !name.is_empty())
        && repository_parts.next().is_none()
        && target.number > 0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubReviewPollingSetupResult {
    Active { target: GithubPullRequestTarget },
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubReviewPollCorrelation {
    pub generation: u64,
    pub target: GithubPullRequestTarget,
}

impl GithubReviewPollCorrelation {
    pub fn new(generation: u64, target: GithubPullRequestTarget) -> Self {
        Self { generation, target }
    }
}
