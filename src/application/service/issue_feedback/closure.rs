//! 외부 닫기는 통합 사실과 독립적이며, 현재 사람 승인과 단일 전송 일지가 필요하다.
use super::*;
use crate::domain::issue_automation::policy::IssueAutomationCloseWhen;
use crate::domain::issue_automation::source::IssueRemoteState;

impl IssueFeedbackService {
    pub(super) fn close_integrated_issue(
        &self,
        snapshot: &IssueFeedbackSnapshot,
        expected_decision_id: &crate::domain::issue_automation::decision::IssueDecisionId,
        now_unix_ms: u64,
        cancel: &IssueCollectionCancellation,
        lease: &dyn IssuePublicationLease,
    ) -> Result<()> {
        let Some((automation, disclosure)) = self.write_context(snapshot)? else {
            return Ok(());
        };
        if automation.policy.close.when != IssueAutomationCloseWhen::Integration
            || !automation.policy.feedback.transition
            || snapshot.state.close_proof.is_some()
            || self.blocked_by_unresolved(&snapshot.item.issue)?
            || cancel.is_cancelled()
        {
            return Ok(());
        }
        let Some(approval) = snapshot.authorized_close() else {
            return Ok(());
        };
        if approval.request.id() != expected_decision_id {
            return Ok(());
        }
        let Some(integration) = snapshot
            .execution
            .as_ref()
            .and_then(|execution| execution.progress.integration.as_ref())
        else {
            return Ok(());
        };
        let issue = &snapshot.item.issue;
        if !matches!(&snapshot.item.current, Some(IssueSourceRead::Observed(observation)) if observation.remote_state == IssueRemoteState::Open)
        {
            return Ok(());
        }
        let observed = self.provider.read_closure(issue)?;
        if observed.issue != *issue || !observed.complete || observed.closed {
            return Ok(());
        }
        let actor_id = self.provider.actor_id(issue)?;
        validate_feedback_id(&actor_id).map_err(anyhow::Error::msg)?;
        let intent = IssueFeedbackIntent {
            intent_id: format!("{:032x}", rand::random::<u128>()),
            issue: issue.clone(),
            actor_id,
            automation_policy_version: automation.version,
            disclosure_policy_version: issue_policy_version(&disclosure),
            desired_revision: snapshot.state.desired_revision,
            operation: IssueFeedbackOperation::CloseIssue {
                integration_record_id: integration.record_id.clone(),
                integration_sha: integration.integration_sha.clone(),
                decision_id: approval.request.id().as_str().to_owned(),
                previous_event_id: observed.latest_event.map(|event| event.event_id),
            },
            recorded_at: format!("unix-ms:{now_unix_ms}"),
        };
        self.record_and_send(snapshot, intent, cancel, lease)?;
        Ok(())
    }

    pub(super) fn close_policy_current(
        &self,
        intent: &IssueFeedbackIntent,
        snapshot: &IssueFeedbackSnapshot,
        policy: &IssueAutomationPolicy,
    ) -> bool {
        let IssueFeedbackOperation::CloseIssue {
            integration_record_id,
            integration_sha,
            decision_id,
            ..
        } = &intent.operation
        else {
            return false;
        };
        policy.close.when == IssueAutomationCloseWhen::Integration
            && policy.feedback.transition
            && snapshot.state.close_proof.is_none()
            && matches!(&snapshot.item.current, Some(IssueSourceRead::Observed(observation)) if observation.remote_state == IssueRemoteState::Open)
            && snapshot
                .authorized_close()
                .is_some_and(|approval| approval.request.id().as_str() == decision_id)
            && snapshot
                .execution
                .as_ref()
                .and_then(|execution| execution.progress.integration.as_ref())
                .is_some_and(|integration| {
                    integration.record_id == *integration_record_id
                        && integration.integration_sha == *integration_sha
                })
    }
}
