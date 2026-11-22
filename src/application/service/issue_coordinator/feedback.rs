//! 피드백은 실행기의 권한을 소유하지 않는다. 최초 표식 확인과 읽기 전용 복구만 연결한다.
use super::{IssueCoordinatorService, Reporter};
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::application::service::issue_feedback::IssueFeedbackService;
use crate::domain::issue_automation::coordinator::{IssueWorkBlockReason, IssueWorkCheckpoint};
use crate::domain::issue_work::IssueReference;
use anyhow::Result;
use std::sync::Arc;

impl IssueCoordinatorService {
    pub fn with_feedback(mut self, feedback: Arc<IssueFeedbackService>) -> Self {
        self.feedback = Some(feedback);
        self
    }

    pub fn with_publication_journal(
        mut self,
        journal: Arc<dyn IssuePublishIntentJournalPort>,
    ) -> Self {
        self.publication_journal = Some(journal);
        self
    }

    /// 표시용 저장 사실만 읽는다. 원격 확인·intent 상태 변경·실행 허가에 사용하지 않는다.
    pub(crate) fn feedback_detail(
        &self,
        issue: &IssueReference,
    ) -> Result<Option<crate::domain::issue_automation::collection::IssueFeedbackDetail>> {
        use crate::domain::issue_automation::collection::{
            IssueExternalState, IssueFeedbackDetail, IssueFeedbackDetailReason,
        };

        let Some(feedback) = &self.feedback else {
            return Ok(None);
        };
        let Some(snapshot) = feedback.snapshot(issue)? else {
            return Ok(None);
        };
        let unresolved = feedback.unresolved(issue)?;
        let needs_confirmation = unresolved.iter().any(|record| record.state.is_unresolved());
        let integration = snapshot
            .execution
            .as_ref()
            .and_then(|execution| execution.progress.integration.clone());
        if snapshot.state.revision == 0
            && snapshot.publication_attempts == 0
            && !needs_confirmation
            && integration.is_none()
        {
            return Ok(None);
        }
        Ok(Some(IssueFeedbackDetail {
            revision: snapshot.state.revision,
            publication_attempts: snapshot.publication_attempts,
            integration,
            external_state: IssueExternalState::from_current(snapshot.item.current.as_ref()),
            comment_id: snapshot
                .state
                .comment_owner
                .as_ref()
                .map(|owner| owner.comment_id.clone()),
            reason: needs_confirmation.then_some(IssueFeedbackDetailReason::FeedbackUnconfirmed),
        }))
    }

    pub(super) fn reconcile_feedback(&self, cancel: &IssueCollectionCancellation) -> Result<()> {
        if let Some(feedback) = &self.feedback {
            feedback.reconcile_pending(cancel)?;
        }
        Ok(())
    }

    pub(super) fn sync_issue_feedback(
        &self,
        issue: &IssueReference,
        now_unix_ms: u64,
        cancel: &IssueCollectionCancellation,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        if let Some(feedback) = &self.feedback {
            let close_decision_id = feedback.snapshot(issue)?.and_then(|snapshot| {
                if snapshot.state.close_proof.is_some() {
                    return None;
                }
                snapshot
                    .authorized_close()
                    .map(|approval| approval.request.id().clone())
            });
            if let Some(decision_id) = close_decision_id {
                // Bind the recheck to the approval captured before resync. A later answer waits for the next poll.
                if !self.fresh_resync(issue, cancel)? {
                    return Ok(());
                }
                super::execution::current_assessment(self, issue, committed)?;
                feedback.sync_issue_after_close_recheck(
                    issue,
                    now_unix_ms,
                    cancel,
                    &decision_id,
                )?;
            } else {
                feedback.sync_issue(issue, now_unix_ms, cancel)?;
            }
        }
        Ok(())
    }

    /// 원격 피드백은 cancellation commit gate 밖에서 끝낸다. 허가는 이후 현재 승인을 다시 읽어 발급한다.
    pub(super) fn prepare_feedback_for_grant(
        &self,
        work: &IssueWorkCheckpoint,
        cancel: &IssueCollectionCancellation,
        committed: &Reporter<'_>,
    ) -> Result<bool> {
        let Some(feedback) = &self.feedback else {
            return Ok(true);
        };
        let ready = self.revoke_on_current_error(
            Some(work),
            feedback.ensure_first_grant(&work.order.issue, &work.order.work_order_id, cancel),
        )?;
        if !ready {
            self.block_work(
                work,
                IssueWorkBlockReason::FeedbackUnconfirmed,
                cancel,
                committed,
            )?;
            return Ok(false);
        }
        self.revoke_on_current_error(Some(work), self.fresh_resync(&work.order.issue, cancel))
    }
}
