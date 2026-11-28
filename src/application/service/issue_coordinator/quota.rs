//! Quota retry never operates on a work order or a claimed arbiter attempt.
use std::sync::Arc;

use anyhow::Result;

use super::{IssueCollectionCancellation, IssueCoordinatorFact, IssueCoordinatorService, Reporter};
use crate::application::port::outbound::issue_account_quota_port::{
    IssueAccountQuota, IssueAccountQuotaPort, IssueQuotaHold, IssueQuotaLane, IssueQuotaWait,
    IssueQuotaWaitChange,
};

impl IssueCoordinatorService {
    pub fn with_account_quota(mut self, quota: Arc<dyn IssueAccountQuotaPort>) -> Self {
        self.account_quota = Some(quota);
        self
    }

    pub fn account_quota_wait(&self, lane: IssueQuotaLane) -> Result<Option<IssueQuotaWait>> {
        Ok(self
            .store
            .issue_quota_wait(&self.workspace, lane)?
            .filter(|wait| wait.hold.is_some()))
    }

    pub fn retry_account_quota(&self, lane: IssueQuotaLane, revision: u64) -> Result<bool> {
        let _guard = self.lock_tick_gate()?;
        let Some(wait) = self.account_quota_wait(lane)? else {
            return Ok(false);
        };
        if wait.revision != revision
            || !matches!(
                wait.hold,
                Some(IssueQuotaHold::Exhausted | IssueQuotaHold::Unknown)
            )
        {
            return Ok(false);
        }
        self.store.change_issue_quota_wait(
            &self.workspace,
            IssueQuotaWaitChange {
                lane,
                expected_revision: revision,
                hold: Some(IssueQuotaHold::RetryQueued),
            },
        )
    }

    pub(super) fn check_account_quota(
        &self,
        lane: IssueQuotaLane,
        cancel: &IssueCollectionCancellation,
        (committed, read): (&Reporter<'_>, impl FnOnce() -> Option<IssueAccountQuota>),
    ) -> Result<bool> {
        if self.account_quota.is_none() {
            // In-process simulations without a configured executor do not need an RPC.
            return Ok(true);
        }
        let wait = self.store.issue_quota_wait(&self.workspace, lane)?;
        let prior_hold = wait.and_then(|state| state.hold);
        if matches!(
            prior_hold,
            Some(IssueQuotaHold::Unknown | IssueQuotaHold::Exhausted)
        ) {
            return Ok(false);
        }
        // Consume the queued revision before RPC. A crash or lost response leaves Unknown,
        // so replay of the original command cannot issue a second read.
        let mut revision = wait.map_or(0, |state| state.revision);
        let queued = prior_hold == Some(IssueQuotaHold::RetryQueued);
        if queued {
            let claimed = cancel.commit_if_active(|| {
                self.store.change_issue_quota_wait(
                    &self.workspace,
                    IssueQuotaWaitChange {
                        lane,
                        expected_revision: revision,
                        hold: Some(IssueQuotaHold::Unknown),
                    },
                )
            })?;
            if claimed != Some(true) {
                return Ok(false);
            }
            revision += 1;
        }
        let hold = match read().unwrap_or(IssueAccountQuota::Unknown) {
            IssueAccountQuota::Allowed => None,
            IssueAccountQuota::Exhausted => Some(IssueQuotaHold::Exhausted),
            IssueAccountQuota::Unknown => Some(IssueQuotaHold::Unknown),
        };
        if !queued && hold.is_none() {
            return Ok(true);
        }
        if queued && hold == Some(IssueQuotaHold::Unknown) {
            committed(vec![IssueCoordinatorFact::AccountQuotaChanged]);
            return Ok(false);
        }
        let resolved = cancel.commit_if_active(|| {
            self.store.change_issue_quota_wait(
                &self.workspace,
                IssueQuotaWaitChange {
                    lane,
                    expected_revision: revision,
                    hold,
                },
            )
        })?;
        if resolved == Some(true) || queued {
            committed(vec![IssueCoordinatorFact::AccountQuotaChanged]);
        }
        Ok(resolved == Some(true) && hold.is_none())
    }
}
