//! Deadline checks use the host's supplied UTC clock and the ordinary current-basis fences.
use super::*;
use crate::application::service::issue_collection::IssueCollectionCancellation;

const DEADLINE_BATCH_SIZE: usize = 256;

impl IssueDecisionService {
    /// Reads the next future deadline without reassessing issues or calling a provider.
    /// Reached deadlines retry on the ordinary tracking interval, never a busy host loop.
    pub(crate) fn next_deadline_at(&self, now_unix_ms: u64) -> Result<Option<u64>> {
        let mut after = None;
        let mut next = None;
        loop {
            let page = self.store.issue_decision_deadlines(
                &self.workspace_dir,
                after.as_ref(),
                DEADLINE_BATCH_SIZE,
            )?;
            if page.is_empty() {
                break;
            }
            for deadline in &page {
                let remind_at = if deadline.reminded {
                    None
                } else {
                    deadline.remind_at.as_deref()
                };
                for at in [
                    deadline.record.request.definition().due_at.as_deref(),
                    remind_at,
                ] {
                    if let Some(at) = timestamp_ms(at).filter(|at| *at > now_unix_ms) {
                        next = Some(next.map_or(at, |previous: u64| previous.min(at)));
                    }
                }
            }
            after = page
                .last()
                .map(|deadline| deadline.record.request.id().clone());
            if page.len() < DEADLINE_BATCH_SIZE {
                break;
            }
        }
        Ok(next)
    }

    /// No model or provider call is made here. Committed answers are reported independently
    /// so an unrelated unreadable request cannot hide a successful earlier transition.
    pub(crate) fn process_deadlines(
        &self,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        committed: &dyn Fn(IssueDecisionCommit),
    ) -> Result<()> {
        let mut after = None;
        let mut first_error = None;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let page = self.store.issue_decision_deadlines(
                &self.workspace_dir,
                after.as_ref(),
                DEADLINE_BATCH_SIZE,
            )?;
            if page.is_empty() {
                break;
            }
            for deadline in &page {
                if cancel.is_cancelled() {
                    break;
                }
                let due = reached(
                    deadline.record.request.definition().due_at.as_deref(),
                    now_unix_ms,
                );
                let remind =
                    !deadline.reminded && reached(deadline.remind_at.as_deref(), now_unix_ms);
                if (due || remind)
                    && let Err(error) = self.process_deadline(
                        deadline.record.request.id(),
                        cancel,
                        now_unix_ms,
                        due,
                        committed,
                    )
                {
                    first_error.get_or_insert(error);
                }
            }
            after = page
                .last()
                .map(|deadline| deadline.record.request.id().clone());
            if page.len() < DEADLINE_BATCH_SIZE {
                break;
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn process_deadline(
        &self,
        id: &IssueDecisionId,
        cancel: &IssueCollectionCancellation,
        now_unix_ms: u64,
        due: bool,
        committed: &dyn Fn(IssueDecisionCommit),
    ) -> Result<()> {
        for _ in 0..DECISION_CAS_ATTEMPTS {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let Some(stored) = self.store.get_issue_decision(&self.workspace_dir, id)? else {
                return Ok(());
            };
            if stored.request.state() != IssueDecisionState::Open {
                return Ok(());
            }
            let (fence, item) = self.current_item(stored.request.issue())?;
            let current = self.triage.assess_current(&item)?;
            let fence = IssueDecisionFence {
                triage: fence,
                decision_revision: stored.revision,
            };
            // Work questions and close requests can outlive the execution context that asked them.
            let obsolete = if !self.work_context_current(stored.request.definition())? {
                let mut record = stored.request.to_record();
                record.state = IssueDecisionState::Superseded;
                Some(IssueDecisionRequest::hydrate(record)?)
            } else {
                None
            };
            let saved = cancel.commit_if_active(|| {
                if let Some(obsolete) = &obsolete {
                    self.store
                        .save_issue_decision(&self.workspace_dir, fence, &current, obsolete)
                } else {
                    self.store.timeout_issue_decision(
                        &self.workspace_dir,
                        fence,
                        &current,
                        id,
                        now_unix_ms,
                    )
                }
            })?;
            match saved {
                Some(IssueDecisionSaveResult::Conflict) => continue,
                Some(IssueDecisionSaveResult::Saved(commit)) => {
                    if commit.changed {
                        committed((*commit).clone());
                    }
                    if !due && commit.record.request.state() == IssueDecisionState::Open {
                        cancel.commit_if_active(|| {
                            self.store.remind_issue_decision(
                                &self.workspace_dir,
                                fence,
                                &current,
                                id,
                                now_unix_ms,
                            )
                        })?;
                    }
                    return Ok(());
                }
                None => return Ok(()),
            }
        }
        bail!("the decision deadline basis changed concurrently")
    }
}

fn reached(at: Option<&str>, now_unix_ms: u64) -> bool {
    timestamp_ms(at).is_some_and(|at| now_unix_ms >= at)
}

fn timestamp_ms(at: Option<&str>) -> Option<u64> {
    at.and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
}
