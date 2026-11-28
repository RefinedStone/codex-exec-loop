use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use super::{IssueCommentCommandService, is_open_tracked_github_issue};
use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationDisplay, IssueConfirmationError, IssueConfirmationPreview,
    IssueConfirmationTarget,
};
use crate::application::port::outbound::issue_work_execution_port::{
    ConfirmedIssueControl, IssueWorkExecutionPort,
};
use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::comment_command::{
    IssueCommentCommandKind, IssueCommentCommandRecord, IssueCommentRequestAcceptance,
    IssueCommentRequestBasis, IssueCommentRequestKey, IssueCommentRequestStatus,
    StoredIssueCommentRequest,
};
use crate::domain::issue_automation::coordinator::issue_work_order_id;
use crate::domain::issue_automation::intervention::IssueInterventionReceipt;
use crate::domain::issue_automation::work_control::IssueWorkControlAction;

mod projection;

impl IssueCommentCommandService {
    pub fn with_request_execution(mut self, execution: Arc<dyn IssueWorkExecutionPort>) -> Self {
        self.execution = Some(execution);
        self
    }

    pub fn prepare_confirmation(
        &self,
        key: &IssueCommentRequestKey,
    ) -> Result<IssueConfirmationPreview> {
        let (request, record) = self.pending_request(key)?;
        Ok(IssueConfirmationPreview {
            target: self.target(&request)?,
            display: IssueConfirmationDisplay {
                message: "기록된 이슈 댓글 요청을 한 번 적용할까요?".into(),
                detail: format!(
                    "이슈: {}\n댓글: {}\n작성자: {}\n요청: {:?}",
                    key.issue.issue_id, key.comment_id, record.author_id, record.kind
                ),
                confirm_label: "요청 적용".into(),
            },
        })
    }

    pub fn confirm_request(
        &self,
        key: &IssueCommentRequestKey,
        context: &StoredCommandContext,
    ) -> Result<IssueInterventionReceipt> {
        let _guard = self
            .request_gate
            .lock()
            .map_err(|_| anyhow!("comment request gate is unavailable"))?;
        let confirmation = context
            .human_confirmation
            .as_ref()
            .ok_or(IssueConfirmationError::Required)?;
        let (request, record) = self.pending_request(key)?;
        confirmation.require_target(&self.target(&request)?)?;
        if !self.records.claim_request(&request)? {
            bail!("the comment request has already been accepted");
        }
        self.execute_accepted(&request, &record)?
            .ok_or_else(|| anyhow!("the comment request expired or became stale"))
    }

    pub fn recover_requests(&self) -> Result<()> {
        let _guard = self
            .request_gate
            .lock()
            .map_err(|_| anyhow!("comment request gate is unavailable"))?;
        for request in self.records.accepted_requests(&self.workspace)? {
            let record = self.record(&request.key)?;
            if let Err(error) = self.execute_accepted(&request, &record) {
                if self
                    .records
                    .request_state(&request.key)?
                    .is_some_and(|(_, state)| state == IssueCommentRequestAcceptance::Uncertain)
                {
                    continue;
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn pending_request(
        &self,
        key: &IssueCommentRequestKey,
    ) -> Result<(StoredIssueCommentRequest, IssueCommentCommandRecord)> {
        let record = self.record(key)?;
        let Some((basis, IssueCommentRequestAcceptance::Pending)) =
            self.records.request_state(key)?
        else {
            bail!("the comment request is not pending");
        };
        if self.current_status(&basis, key, &record)? != IssueCommentRequestStatus::Pending
            || self
                .execution()?
                .control(&self.workspace, &issue_work_order_id(&key.issue))?
                .authorization_epoch
                != basis.authorization_epoch
        {
            bail!("the comment request expired or became stale");
        }
        Ok((
            StoredIssueCommentRequest {
                key: key.clone(),
                basis,
            },
            record,
        ))
    }

    fn record(&self, key: &IssueCommentRequestKey) -> Result<IssueCommentCommandRecord> {
        if key.issue.project_scope.workspace_dir != self.workspace {
            bail!("the comment request belongs to another workspace");
        }
        self.records
            .issue_comment_command(&self.workspace, &key.issue, &key.comment_id)?
            .ok_or_else(|| anyhow!("the comment request was not found"))
    }

    fn target(&self, request: &StoredIssueCommentRequest) -> Result<IssueConfirmationTarget> {
        Ok(IssueConfirmationTarget::CommentRequest {
            workspace: self.workspace.clone(),
            command_id: request.key.command_id()?,
            basis: request.basis.clone(),
        })
    }

    fn execution(&self) -> Result<&dyn IssueWorkExecutionPort> {
        self.execution
            .as_deref()
            .ok_or_else(|| anyhow!("comment request execution is unavailable"))
    }

    fn current_status(
        &self,
        basis: &IssueCommentRequestBasis,
        key: &IssueCommentRequestKey,
        record: &IssueCommentCommandRecord,
    ) -> Result<IssueCommentRequestStatus> {
        match self.records.current_request_issue(key)? {
            Some(item) => self.issue_status(basis, &item, record),
            None => Ok(IssueCommentRequestStatus::Stale),
        }
    }

    fn issue_status(
        &self,
        basis: &IssueCommentRequestBasis,
        item: &IssueCollectionItem,
        record: &IssueCommentCommandRecord,
    ) -> Result<IssueCommentRequestStatus> {
        if action(record.kind).is_none() {
            return Ok(IssueCommentRequestStatus::DisplayOnly);
        }
        let stale = !is_open_tracked_github_issue(item)
            || item.binding_run_blocked
            || item.observation_revision != Some(basis.observation_revision)
            || item.tracked_revision != Some(basis.tracked_revision)
            || item.binding_run_revision != basis.binding_run_revision
            || !self.trusted_actor_ids().contains(&record.author_id);
        if u64::try_from(chrono::Utc::now().timestamp_millis())? >= basis.expires_at_unix_ms {
            return Ok(IssueCommentRequestStatus::Expired);
        }
        if stale {
            return Ok(IssueCommentRequestStatus::Stale);
        }
        Ok(IssueCommentRequestStatus::Pending)
    }

    fn applied_receipt(
        &self,
        key: &IssueCommentRequestKey,
        basis: &IssueCommentRequestBasis,
        record: &IssueCommentCommandRecord,
    ) -> Result<Option<IssueInterventionReceipt>> {
        let Some((action, label)) = action(record.kind) else {
            return Ok(None);
        };
        let command_id = key.command_id()?;
        let work_order_id = issue_work_order_id(&key.issue);
        Ok(self
            .execution()?
            .control_command_result(&self.workspace, &work_order_id, &command_id, action)?
            .map(|control| {
                receipt(
                    key,
                    basis,
                    (
                        command_id,
                        work_order_id,
                        label,
                        control.authorization_epoch,
                    ),
                )
            }))
    }

    fn execute_accepted(
        &self,
        request: &StoredIssueCommentRequest,
        record: &IssueCommentCommandRecord,
    ) -> Result<Option<IssueInterventionReceipt>> {
        let prior = self
            .applied_receipt(&request.key, &request.basis, record)
            .or_else(|error| {
                self.records.close_accepted_request(
                    &request.key,
                    IssueCommentRequestAcceptance::Uncertain,
                )?;
                Err(error)
            })?;
        if let Some(receipt) = prior {
            return Ok(Some(receipt));
        }
        let Some((action, label)) = action(record.kind) else {
            return Ok(None);
        };
        let work_order_id = issue_work_order_id(&request.key.issue);
        let command_id = request.key.command_id()?;
        let applied = self
            .execution()?
            .apply_confirmed_control(
                &self.workspace,
                &ConfirmedIssueControl {
                    work_order_id: &work_order_id,
                    command_id: &command_id,
                    action,
                    expected_epoch: request.basis.authorization_epoch,
                    authorize: &|| {
                        Ok(self.current_status(&request.basis, &request.key, record)?
                            == IssueCommentRequestStatus::Pending)
                    },
                },
            )
            .or_else(|error| {
                self.records.close_accepted_request(
                    &request.key,
                    IssueCommentRequestAcceptance::Uncertain,
                )?;
                Err(error)
            })?;
        let Some(control) = applied else {
            self.records
                .close_accepted_request(&request.key, IssueCommentRequestAcceptance::Refused)?;
            return Ok(None);
        };
        Ok(Some(receipt(
            &request.key,
            &request.basis,
            (
                command_id,
                work_order_id,
                label,
                control.authorization_epoch,
            ),
        )))
    }
}

fn action(kind: IssueCommentCommandKind) -> Option<(IssueWorkControlAction, &'static str)> {
    match kind {
        IssueCommentCommandKind::Release => {
            Some((IssueWorkControlAction::Release, "release_issue"))
        }
        IssueCommentCommandKind::Retriage => {
            Some((IssueWorkControlAction::Retriage, "retriage_issue"))
        }
        IssueCommentCommandKind::Hold | IssueCommentCommandKind::Approve => None,
    }
}

fn receipt(
    key: &IssueCommentRequestKey,
    basis: &IssueCommentRequestBasis,
    (command_id, work_order_id, label, epoch): (String, String, &str, u64),
) -> IssueInterventionReceipt {
    IssueInterventionReceipt {
        issue: key.issue.clone(),
        command_id,
        action: label.into(),
        work_order_id: Some(work_order_id),
        authorization_epoch: epoch,
        changed: epoch != basis.authorization_epoch,
        delivery: None,
        handoff: None,
    }
}
