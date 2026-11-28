use super::{
    IssueCollectionItem, IssueCommentCommandRecord, IssueCommentCommandService,
    IssueCommentRequestAcceptance as Acceptance, IssueCommentRequestBasis, IssueCommentRequestKey,
    IssueCommentRequestStatus as Status, Result, StoredCommandContext, action, issue_work_order_id,
};
use crate::domain::issue_automation::comment_command::IssueCommentRequestDetail;

const REQUEST_LIFETIME_MS: u64 = 15 * 60 * 1000;

impl IssueCommentCommandService {
    pub(in crate::application::service::issue_comment_command) fn request_basis(
        &self,
        item: &IssueCollectionItem,
        record: &IssueCommentCommandRecord,
        context: &StoredCommandContext,
    ) -> Result<Option<IssueCommentRequestBasis>> {
        if action(record.kind).is_none() {
            return Ok(None);
        }
        let (Some(execution), Some(observation_revision), Some(tracked_revision)) = (
            &self.execution,
            item.observation_revision,
            item.tracked_revision,
        ) else {
            return Ok(None);
        };
        let observed_at =
            u64::try_from(chrono::DateTime::parse_from_rfc3339(&context.at)?.timestamp_millis())?;
        Ok(Some(IssueCommentRequestBasis {
            observation_revision,
            tracked_revision,
            binding_run_revision: item.binding_run_revision,
            authorization_epoch: execution
                .control(&self.workspace, &issue_work_order_id(&item.issue))?
                .authorization_epoch,
            expires_at_unix_ms: observed_at.saturating_add(REQUEST_LIFETIME_MS),
        }))
    }

    pub(in crate::application::service) fn recorded_requests(
        &self,
        item: &IssueCollectionItem,
    ) -> Result<Vec<IssueCommentRequestDetail>> {
        self.records
            .recorded_requests(&self.workspace, &item.issue)?
            .into_iter()
            .map(|record| {
                let key = key(&record);
                let (status, receipt) = match self.records.request_state(&key)? {
                    Some((basis, state @ (Acceptance::Accepted | Acceptance::Uncertain))) => {
                        match self.applied_receipt(&key, &basis, &record) {
                            Ok(Some(receipt)) => (Status::Applied, Some(receipt)),
                            Ok(None) if state == Acceptance::Accepted => (
                                match self.status(&basis, item, &record)? {
                                    Status::Pending => Status::Applying,
                                    status => status,
                                },
                                None,
                            ),
                            Ok(None) | Err(_) => (Status::Uncertain, None),
                        }
                    }
                    Some((basis, Acceptance::Pending)) => {
                        (self.status(&basis, item, &record)?, None)
                    }
                    Some((basis, Acceptance::Refused)) => (
                        match self.status(&basis, item, &record)? {
                            Status::Expired => Status::Expired,
                            _ => Status::Stale,
                        },
                        None,
                    ),
                    None => (Status::DisplayOnly, None),
                };
                Ok(IssueCommentRequestDetail {
                    record,
                    status,
                    receipt,
                })
            })
            .collect()
    }
}

fn key(record: &IssueCommentCommandRecord) -> IssueCommentRequestKey {
    IssueCommentRequestKey {
        issue: record.issue.clone(),
        comment_id: record.comment_id.clone(),
    }
}

impl IssueCommentCommandService {
    fn status(
        &self,
        basis: &IssueCommentRequestBasis,
        item: &IssueCollectionItem,
        record: &IssueCommentCommandRecord,
    ) -> Result<Status> {
        let status = self.issue_status(basis, item, record)?;
        if status == Status::Pending
            && self
                .execution()?
                .control(&self.workspace, &issue_work_order_id(&item.issue))?
                .authorization_epoch
                != basis.authorization_epoch
        {
            return Ok(Status::Stale);
        }
        Ok(status)
    }
}
