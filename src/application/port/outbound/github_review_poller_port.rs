use anyhow::Result;

use crate::domain::github_review::{GithubPullRequestActivitySnapshot, GithubPullRequestTarget};

pub trait GithubReviewPollerPort: Send + Sync {
    fn load_pull_request_activity(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<GithubPullRequestActivitySnapshot>;
}
