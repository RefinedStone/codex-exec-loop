use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::domain::parallel_mode::{ParallelModeCapabilitySnapshot, ParallelModeCapabilityState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubAutomationCapabilities {
    pub push_remote: ParallelModeCapabilitySnapshot,
    pub gh_binary: ParallelModeCapabilitySnapshot,
    pub gh_auth: ParallelModeCapabilitySnapshot,
}

impl GithubAutomationCapabilities {
    pub fn new(
        push_remote: ParallelModeCapabilitySnapshot,
        gh_binary: ParallelModeCapabilitySnapshot,
        gh_auth: ParallelModeCapabilitySnapshot,
    ) -> Self {
        Self {
            push_remote,
            gh_binary,
            gh_auth,
        }
    }

    pub fn push_ready(&self) -> bool {
        self.push_remote.state == ParallelModeCapabilityState::Ready
    }

    pub fn github_ready(&self) -> bool {
        self.gh_binary.state == ParallelModeCapabilityState::Ready
            && self.gh_auth.state == ParallelModeCapabilityState::Ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAutomationPullRequest {
    pub number: u64,
    pub url: String,
    pub state: String,
    pub base_branch: String,
    pub head_branch: String,
    pub is_draft: bool,
}

impl GithubAutomationPullRequest {
    pub fn new(
        number: u64,
        url: impl Into<String>,
        state: impl Into<String>,
        base_branch: impl Into<String>,
        head_branch: impl Into<String>,
        is_draft: bool,
    ) -> Self {
        Self {
            number,
            url: url.into(),
            state: state.into(),
            base_branch: base_branch.into(),
            head_branch: head_branch.into(),
            is_draft,
        }
    }
}

pub trait GithubAutomationPort: Send + Sync {
    fn inspect_capabilities(&self, repo_root: &str) -> GithubAutomationCapabilities;

    fn push_branch(&self, repo_root: &str, branch_name: &str, force_with_lease: bool)
    -> Result<()>;

    fn ensure_pull_request(
        &self,
        repo_root: &str,
        base_branch: &str,
        head_branch: &str,
        title: &str,
        body: &str,
    ) -> Result<GithubAutomationPullRequest>;

    fn inspect_pull_request(
        &self,
        repo_root: &str,
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest>;

    fn push_integration_branch(&self, repo_root: &str, branch_name: &str) -> Result<()>;

    fn close_pull_request(&self, repo_root: &str, pr_number: u64) -> Result<()>;
}
