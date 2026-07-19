use crate::domain::github_review::GithubPullRequestTarget;

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
