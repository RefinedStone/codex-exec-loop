use crate::domain::github_review::GithubPullRequestTarget;

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
}

impl GithubReviewPollingSetupRequest {
    pub fn new(workspace_directory: impl Into<String>, mode: GithubReviewPollingSetupMode) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            mode,
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
