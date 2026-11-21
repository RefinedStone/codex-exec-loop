//! Defensive review of one revision of the PR already selected by the watcher.

use std::sync::Arc;

use crate::application::port::outbound::pr_review_port::{
    CommentReservation, DefensivePrReviewBackend, MAX_DIFF_BYTES, PrReviewGithubPort,
    PrReviewJournalPort, ReviewInput, ReviewProgress, ReviewRevision,
};
use crate::domain::{
    github_review::GithubPullRequestTarget,
    pr_review::{ReviewMode, ReviewRecord, ReviewResult, Severity},
};
use anyhow::{Result, ensure};

#[derive(Clone)]
pub struct PrReviewService {
    journal: Arc<dyn PrReviewJournalPort>,
    github: Arc<dyn PrReviewGithubPort>,
    backend: Option<Arc<dyn DefensivePrReviewBackend>>,
    repository: String,
}

impl PrReviewService {
    pub fn new(
        repository: String,
        journal: Arc<dyn PrReviewJournalPort>,
        github: Arc<dyn PrReviewGithubPort>,
        backend: Option<Arc<dyn DefensivePrReviewBackend>>,
    ) -> Self {
        Self {
            journal,
            github,
            backend,
            repository,
        }
    }

    pub fn poll(&self, target: &GithubPullRequestTarget) -> Result<()> {
        let mode = self.journal.mode(&self.repository)?;
        if !target.repository.eq_ignore_ascii_case(&self.repository) || mode == ReviewMode::Off {
            return Ok(());
        }
        let Some(backend) = &self.backend else {
            anyhow::bail!("defensive PR review backend unavailable");
        };
        ensure!(
            backend.configured(),
            "defensive PR review backend unavailable"
        );
        let Some(revision) = self.github.revision(target)? else {
            return Ok(());
        };
        ensure!(
            valid_sha(&revision.head) && valid_sha(&revision.base),
            "invalid PR revision"
        );
        let head = &revision.head;
        if !(self.journal.claim(
            target,
            head,
            mode == ReviewMode::On && backend.comments_allowed(),
        )? || backend.resumes_durable_job() && self.journal.pending(target, head)?)
        {
            return Ok(());
        }
        let Some(result) = self.review(target, &revision, backend.as_ref()) else {
            return Ok(());
        };
        if !self.journal.finish(target, head, &result)? {
            return Ok(());
        }
        if let ReviewRecord::Reviewed { result, .. } = result
            && mode == ReviewMode::On
            && backend.comments_allowed()
            && self.journal.mode(&self.repository)? == ReviewMode::On
            && self.matches_revision(target, &revision)?
            && let CommentReservation::Write(existing) =
                self.journal.reserve_comment(target, head)?
        {
            let comment = render_comment(&result);
            // Reserve durably before the network write. An error or crash leaves
            // an unresolved reservation and suppresses subsequent writes to this PR.
            let id = self
                .github
                .write_comment(target, existing, &comment)
                .map_err(|_| anyhow::anyhow!("PR review comment outcome unknown"))?;
            self.journal.record_comment(target, head, id)?;
        }
        Ok(())
    }

    fn review(
        &self,
        target: &GithubPullRequestTarget,
        revision: &ReviewRevision,
        backend: &dyn DefensivePrReviewBackend,
    ) -> Option<ReviewRecord> {
        let diff = match self.github.diff(target) {
            Ok(Some(diff)) if diff.len() <= MAX_DIFF_BYTES => diff,
            Ok(_) => return Some(ReviewRecord::DiffTooLarge),
            Err(_) => return Some(ReviewRecord::SourceUnavailable),
        };
        match self.matches_revision(target, revision) {
            Ok(true) => {}
            Ok(false) => return Some(ReviewRecord::Cancelled),
            Err(_) => return Some(ReviewRecord::SourceUnavailable),
        }
        if revision.title.len() + revision.body.len() > 64 * 1024 {
            return Some(ReviewRecord::SourceUnavailable);
        }
        let untrusted = ReviewInput {
            title: revision.title.clone(),
            body: revision.body.clone(),
            diff,
        };
        if !matches!(
            self.journal.mode(&self.repository),
            Ok(ReviewMode::Observe | ReviewMode::On)
        ) {
            return Some(ReviewRecord::Cancelled);
        }
        let output = match backend.review(target, &revision.head, untrusted) {
            Ok(ReviewProgress::Complete(output)) => output,
            Ok(ReviewProgress::Pending) => return None,
            Ok(ReviewProgress::ResultInvalid) => return Some(ReviewRecord::ResultInvalid),
            Err(_) => return Some(ReviewRecord::BackendFailed),
        };
        if output.backend.is_empty()
            || output.backend.len() > 128
            || output.backend.chars().any(char::is_control)
        {
            return Some(ReviewRecord::ResultInvalid);
        }
        let Some(result) = ReviewResult::parse_json(&output.json) else {
            return Some(ReviewRecord::ResultInvalid);
        };
        Some(match self.matches_revision(target, revision) {
            Ok(true) => ReviewRecord::Reviewed {
                backend: output.backend,
                result,
            },
            Ok(false) => ReviewRecord::Cancelled,
            Err(_) => ReviewRecord::SourceUnavailable,
        })
    }

    fn matches_revision(
        &self,
        target: &GithubPullRequestTarget,
        expected: &ReviewRevision,
    ) -> Result<bool> {
        Ok(self
            .github
            .revision(target)?
            .is_some_and(|current| current.head == expected.head && current.base == expected.base))
    }
}

fn valid_sha(sha: &str) -> bool {
    sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

fn render_comment(result: &ReviewResult) -> String {
    // Only host-generated aggregate data crosses the GitHub write boundary.
    // Free-form model text and filenames can contain secrets, links or mentions.
    let counts = result
        .findings
        .iter()
        .fold([0u32; 4], |mut counts, finding| {
            counts[match finding.severity {
                Severity::Low => 0,
                Severity::Medium => 1,
                Severity::High => 2,
                Severity::Critical => 3,
            }] += 1;
            counts
        });
    format!(
        "### Akra defensive review\n\nFindings: {} critical, {} high, {} medium, {} low.\n\nDetailed findings are available in Akra.",
        counts[3], counts[2], counts[1], counts[0]
    )
}

#[cfg(test)]
mod tests;
