use super::production::{delegation_route, delegation_runtime, delegation_store};
use crate::adapter::outbound::openai_review::ReviewCall;
use crate::application::port::outbound::delegation_store_port::DelegationStorePort;
use crate::application::port::outbound::pr_review_port::{
    DefensivePrReviewBackend, PrReviewModelCall, ReviewInput, ReviewOutput, ReviewProgress,
};
use crate::domain::delegation::{
    EndReason, State,
    policy::{OPENAI_TEXT, Route},
};
use crate::domain::github_review::GithubPullRequestTarget;
use crate::domain::pr_review::ReviewRecord;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(super) struct RoutedPrReview {
    workspace: String,
    repository: String,
}

impl RoutedPrReview {
    pub(super) fn new(workspace: &str, repository: &str) -> Self {
        Self {
            workspace: workspace.into(),
            repository: repository.into(),
        }
    }
}

impl DefensivePrReviewBackend for RoutedPrReview {
    fn configured(&self) -> bool {
        delegation_route(&self.workspace, "pr_review")
            .is_ok_and(|r| r.backends.iter().any(|b| b.kind == OPENAI_TEXT))
            && crate::adapter::outbound::openai_review::has_key(&self.repository)
    }
    fn resumes_durable_job(&self) -> bool {
        true
    }

    fn comments_allowed(&self) -> bool {
        delegation_route(&self.workspace, "pr_review").is_ok_and(|r| r.pr_comments_allowed)
    }

    fn review(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
        input: ReviewInput,
    ) -> Result<ReviewProgress> {
        ensure!(
            target.repository.eq_ignore_ascii_case(&self.repository),
            "PR review repository mismatch"
        );
        let route = delegation_route(&self.workspace, "pr_review")?;
        ensure!(
            route.backends.iter().any(|b| b.kind == OPENAI_TEXT),
            "isolated review backend unavailable"
        );
        let call = ReviewCall::new(&self.repository, &input)?;
        let runtime = delegation_runtime(&self.workspace)?;
        let journal = crate::adapter::outbound::db::sqlite_pr_review_journal::ReviewJournal::new(
            runtime.clone(),
        );
        let store = delegation_store(runtime, false).context("delegation store unavailable")?;
        let prefix = format!(
            "{}:{}:",
            self.repository.to_ascii_lowercase(),
            target.number
        );
        for job in store.active()? {
            if job.route.requires_defensive_isolation
                && job.view.caller == "pr_review"
                && let Some(old_head) = job.instruction.strip_prefix(&prefix)
                && matches!(
                    journal.result(target, old_head)?,
                    Some(ReviewRecord::Cancelled)
                )
            {
                store.cancel(&job.view.id)?;
            }
        }
        drive_review(store, route, target, head, &call)
    }
}

fn drive_review(
    store: Arc<dyn DelegationStorePort>,
    route: Route,
    target: &GithubPullRequestTarget,
    head: &str,
    call: &dyn PrReviewModelCall,
) -> Result<ReviewProgress> {
    // Source data and credentials live only in this watcher call. Store a stable
    // revision identity to recover the same job after a poller or kernel restart.
    let revision = format!(
        "{}:{}:{}",
        target.repository.to_ascii_lowercase(),
        target.number,
        head
    );
    let key = format!("pr_review:{:x}", Sha256::digest(revision.as_bytes()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(crate::adapter::inbound::gateway::expire_pr_reviews(&store))?;
    let job = store.submit_with_route(&key, &revision, "pr_review", Ok(route))?;
    if !job.state.terminal() {
        runtime.block_on(crate::adapter::inbound::gateway::advance_pr_review(
            store.clone(),
            &job.id,
            call,
        ))?;
    }
    let job = store.find(&job.id)?.context("review delegation missing")?;
    match job.state {
        State::Succeeded => Ok(ReviewProgress::Complete(ReviewOutput {
            backend: job
                .attempts
                .last()
                .context("review attempt missing")?
                .backend
                .clone(),
            json: store.result(&job.id)?.context("review result missing")?,
        })),
        State::Failed
            if job.attempts.last().and_then(|a| a.reason) == Some(EndReason::ResultInvalid) =>
        {
            Ok(ReviewProgress::ResultInvalid)
        }
        State::Failed | State::Expired | State::Cancelled | State::AwaitingHuman => {
            anyhow::bail!("review delegation did not complete")
        }
        _ => Ok(ReviewProgress::Pending),
    }
}

#[cfg(test)]
mod tests;
