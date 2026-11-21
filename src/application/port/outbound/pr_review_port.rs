//! Source, journal, and isolated backend capabilities used by PR review.

use std::time::Duration;

use anyhow::Result;
use futures_util::future::BoxFuture;

use crate::domain::{
    delegation::EndReason,
    github_review::GithubPullRequestTarget,
    pr_review::{ReviewMode, ReviewRecord},
};

pub(crate) const MAX_DIFF_BYTES: usize = 96 * 1024;

pub struct ReviewInput {
    pub title: String,
    pub body: String,
    pub diff: String,
}

pub struct ReviewRevision {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
}

/// Implement only for a backend with enforced isolation that prevents tool use,
/// repository execution, approval, network actions, and other external effects.
/// A prompt instruction or a read-only filesystem sandbox is insufficient.
pub trait DefensivePrReviewBackend: Send + Sync {
    fn configured(&self) -> bool {
        true
    }
    fn review(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
        untrusted: ReviewInput,
    ) -> Result<ReviewProgress>;
    fn resumes_durable_job(&self) -> bool {
        false
    }
    fn comments_allowed(&self) -> bool {
        true
    }
}

pub struct ReviewOutput {
    pub backend: String,
    pub json: String,
}

pub enum ReviewProgress {
    Pending,
    ResultInvalid,
    Complete(ReviewOutput),
}

/// A prepared, tool-free model request whose PR data stays with the watcher.
pub(crate) trait PrReviewModelCall: Send + Sync {
    fn complete<'a>(
        &'a self,
        model: &'a str,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<String, EndReason>>;
}

pub trait PrReviewGithubPort: Send + Sync {
    fn revision(&self, target: &GithubPullRequestTarget) -> Result<Option<ReviewRevision>>;
    /// `None` denotes a diff that exceeds [`MAX_DIFF_BYTES`].
    fn diff(&self, target: &GithubPullRequestTarget) -> Result<Option<String>>;
    /// On failure the write outcome is unknown. The caller must not repeat it.
    fn write_comment(
        &self,
        target: &GithubPullRequestTarget,
        existing: Option<u64>,
        body: &str,
    ) -> Result<u64>;
}

pub trait PrReviewJournalPort: Send + Sync {
    fn mode(&self, repository: &str) -> Result<ReviewMode>;
    fn claim(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
        may_comment: bool,
    ) -> Result<bool>;
    fn pending(&self, target: &GithubPullRequestTarget, head: &str) -> Result<bool>;
    fn finish(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
        result: &ReviewRecord,
    ) -> Result<bool>;
    fn reserve_comment(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
    ) -> Result<CommentReservation>;
    fn record_comment(&self, target: &GithubPullRequestTarget, head: &str, id: u64) -> Result<()>;
}

pub enum CommentReservation {
    Suppressed,
    Write(Option<u64>),
}
