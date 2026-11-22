//! 원격 피드백은 durable Sent CAS 성공자만 한 번 쓴다. 재시작 복구는 읽기만 한다.
use super::issue_automation_policy::{IssueAutomationPolicyService, ResolvedIssueAutomationPolicy};
use super::issue_collection::IssueCollectionCancellation;
use super::issue_masking::{IssueTextField, mask_issue_text};
use super::issue_policy::{IssuePolicyService, issue_policy_version};
use crate::application::port::outbound::issue_feedback_port::{
    IssueFeedbackLabelSnapshot, IssueFeedbackPort, IssueFeedbackReadResult,
    IssueFeedbackWriteResult,
};
use crate::application::port::outbound::issue_feedback_store_port::{
    IssueFeedbackClaimResult, IssueFeedbackCompletion, IssueFeedbackFence,
    IssueFeedbackGrantResult, IssueFeedbackRecordResult, IssueFeedbackSaveResult,
    IssueFeedbackSnapshot, IssueFeedbackStorePort,
};
use crate::application::port::outbound::issue_publish_intent_journal_port::{
    IssuePublicationLease, IssuePublishIntentJournalPort,
};
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::execution::IssueExecutionState;
use crate::domain::issue_automation::feedback::projection::safe_progress_label;
use crate::domain::issue_automation::feedback::*;
use crate::domain::issue_automation::policy::IssueAutomationPolicy;
use crate::domain::issue_automation::source::IssueSourceRead;
use crate::domain::issue_automation::{IssueAutomationAutonomy, IssueAutomationState};
use crate::domain::issue_policy::IssuePolicy;
use crate::domain::issue_secret_spans::secret_spans;
use crate::domain::issue_work::IssueReference;
use anyhow::{Result, anyhow, bail};
use std::sync::{Arc, Mutex, MutexGuard};

mod effects;
mod labels;
use labels::terminal_for_feedback;
#[cfg(test)]
pub(crate) mod tests;

pub const ISSUE_FEEDBACK_EVENT_PREFIX: &str = "issue-feedback:";

pub struct IssueFeedbackService {
    workspace: String,
    store: Arc<dyn IssueFeedbackStorePort>,
    provider: Arc<dyn IssueFeedbackPort>,
    publication: Arc<dyn IssuePublishIntentJournalPort>,
    automation: IssueAutomationPolicyService,
    disclosure: IssuePolicyService,
    gate: Mutex<()>,
}
impl IssueFeedbackService {
    pub fn new(
        workspace: String,
        store: Arc<dyn IssueFeedbackStorePort>,
        provider: Arc<dyn IssueFeedbackPort>,
        publication: Arc<dyn IssuePublishIntentJournalPort>,
        automation: IssueAutomationPolicyService,
        disclosure: IssuePolicyService,
    ) -> Self {
        Self {
            workspace,
            store,
            provider,
            publication,
            automation,
            disclosure,
            gate: Mutex::new(()),
        }
    }

    /// Off·정책 읽기 실패·보류와 무관하게 이미 보낸 의도를 읽기로만 확인한다.
    pub fn reconcile_pending(&self, cancel: &IssueCollectionCancellation) -> Result<()> {
        let _gate = self.lock()?;
        let lease = self
            .publication
            .acquire_publication_lease(&self.workspace)?;
        lease.verify_identity()?;
        for record in self.store.pending(&self.workspace)? {
            if cancel.is_cancelled() {
                break;
            }
            self.reconcile_record(record, lease.as_ref())?;
        }
        Ok(())
    }

    /// raw 에이전트 문장은 받지 않는다. 저장된 enum·집계만 상태 공유본으로 렌더한다.
    pub fn sync_issue(
        &self,
        issue: &IssueReference,
        now_unix_ms: u64,
        cancel: &IssueCollectionCancellation,
    ) -> Result<()> {
        self.check_scope(issue)?;
        if cancel.is_cancelled() {
            return Ok(());
        }
        let _gate = self.lock()?;
        let lease = self
            .publication
            .acquire_publication_lease(&self.workspace)?;
        lease.verify_identity()?;
        let Some(initial) = self.current(issue)? else {
            return Ok(());
        };
        let Some((initial_policy, initial_disclosure)) = self.write_context(&initial)? else {
            return Ok(());
        };
        if initial_policy.policy.feedback.labels {
            self.remove_terminal_label(
                &initial,
                &initial_policy,
                &initial_disclosure,
                now_unix_ms,
                cancel,
                lease.as_ref(),
            )?;
        }
        let Some(snapshot) = self.current(issue)? else {
            return Ok(());
        };
        let Some((automation, disclosure)) = self.write_context(&snapshot)? else {
            return Ok(());
        };
        if !automation.policy.feedback.comment || snapshot.work.is_none() {
            return Ok(());
        }
        let desired = masked_status(&render_status(&snapshot, &automation.policy), &disclosure)?;
        let Some(saved) = cancel
            .commit_if_active(|| self.store.save_desired(issue, snapshot.fence(), &desired))?
        else {
            return Ok(());
        };
        if matches!(saved, IssueFeedbackSaveResult::Conflict) {
            return Ok(());
        }
        let Some(current) = self.current(issue)? else {
            return Ok(());
        };
        if !authority_matches(snapshot.fence(), current.fence())
            || self.blocked_by_unresolved(issue)?
        {
            return Ok(());
        }
        if current
            .state
            .comment_owner
            .as_ref()
            .is_some_and(|owner| owner.content_sha256 == desired.comment_body_sha256)
        {
            return Ok(());
        }
        let actor_id = self.provider.actor_id(issue)?;
        validate_feedback_id(&actor_id).map_err(anyhow::Error::msg)?;
        let intent_id = format!("{:032x}", rand::random::<u128>());
        let body = format!(
            "{}\n\n{}",
            desired.comment_body,
            feedback_marker(&intent_id)
        );
        let body_sha256 = feedback_body_hash(&body);
        let content_sha256 = desired.comment_body_sha256;
        let operation = match &current.state.comment_owner {
            Some(owner) if owner.issue == *issue && owner.actor_id == actor_id => {
                IssueFeedbackOperation::UpdateComment {
                    owner: Box::new(owner.clone()),
                    body,
                    body_sha256,
                    content_sha256,
                }
            }
            Some(_) => bail!("feedback_unconfirmed: the status comment belongs to another owner"),
            None => IssueFeedbackOperation::CreateComment {
                body,
                body_sha256,
                content_sha256,
            },
        };
        let intent = IssueFeedbackIntent {
            intent_id,
            issue: issue.clone(),
            actor_id,
            automation_policy_version: automation.version,
            disclosure_policy_version: issue_policy_version(&disclosure),
            desired_revision: current.state.desired_revision,
            operation,
            recorded_at: format!("unix-ms:{now_unix_ms}"),
        };
        self.record_and_send(&current, intent, cancel, lease.as_ref())?;
        Ok(())
    }

    /// 상세 조회가 사용할 저장 사실이다. 원격 확인이나 쓰기를 수행하지 않는다.
    pub fn snapshot(&self, issue: &IssueReference) -> Result<Option<IssueFeedbackSnapshot>> {
        self.check_scope(issue)?;
        self.current(issue)
    }
    pub fn unresolved(&self, issue: &IssueReference) -> Result<Vec<IssueFeedbackIntentRecord>> {
        self.check_scope(issue)?;
        self.store.unresolved(issue)
    }
    pub fn pending_events(&self) -> Result<Vec<IssueFeedbackEvent>> {
        self.store.pending_events(&self.workspace)
    }
    pub fn acknowledge_event(&self, event_id: &str) -> Result<()> {
        if !event_id.starts_with(ISSUE_FEEDBACK_EVENT_PREFIX) {
            bail!("invalid feedback event identity");
        }
        self.store.acknowledge_event(&self.workspace, event_id)
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>> {
        self.gate
            .lock()
            .map_err(|_| anyhow!("the feedback operation gate is unavailable"))
    }
    fn check_scope(&self, issue: &IssueReference) -> Result<()> {
        validate_feedback_issue(issue).map_err(anyhow::Error::msg)?;
        if issue.project_scope.workspace_dir != self.workspace {
            bail!("feedback owner workspace changed");
        }
        Ok(())
    }
    fn current(&self, issue: &IssueReference) -> Result<Option<IssueFeedbackSnapshot>> {
        let snapshot = self.store.snapshot(issue)?;
        if let Some(value) = &snapshot {
            value.state.validate().map_err(anyhow::Error::msg)?;
            if value.item.issue != *issue || value.state.issue != *issue {
                bail!("feedback snapshot ownership changed");
            }
        }
        Ok(snapshot)
    }
    fn write_context(
        &self,
        snapshot: &IssueFeedbackSnapshot,
    ) -> Result<Option<(ResolvedIssueAutomationPolicy, IssuePolicy)>> {
        if !snapshot_allows_write(snapshot) {
            return Ok(None);
        }
        let file = self.automation.resolve(&self.workspace)?;
        let automation = super::issue_autonomy::effective_policy(file, &snapshot.autonomy);
        if automation.policy.binding.github.mode == IssueAutomationAutonomy::Off {
            return Ok(None);
        }
        let disclosure = self.disclosure.resolve(&self.workspace)?;
        Ok(Some((automation, disclosure)))
    }
    fn blocked_by_unresolved(&self, issue: &IssueReference) -> Result<bool> {
        if !self.store.unresolved(issue)?.is_empty() {
            return Ok(true);
        }
        self.publish_pending(issue)
    }
    fn publish_pending(&self, issue: &IssueReference) -> Result<bool> {
        Ok(self
            .publication
            .load(&self.workspace)?
            .iter()
            .any(|record| record.issue.same_remote_issue(issue) && record.state.is_unresolved()))
    }
}

fn snapshot_allows_write(snapshot: &IssueFeedbackSnapshot) -> bool {
    let Some(tracked) = &snapshot.item.tracked else {
        return false;
    };
    snapshot.collection.config.mode == IssueCollectionMode::Observe
        && snapshot.autonomy.pending.is_none()
        && !snapshot.pending_intervention
        && !tracked.flags.held
        && !tracked.flags.stale
        && !tracked.flags.unknown
        && matches!(snapshot.item.current, Some(IssueSourceRead::Observed(_)))
}
fn authority_matches(before: IssueFeedbackFence, after: IssueFeedbackFence) -> bool {
    IssueFeedbackFence {
        feedback_revision: 0,
        ..before
    } == IssueFeedbackFence {
        feedback_revision: 0,
        ..after
    }
}
fn masked_status(body: &str, policy: &IssuePolicy) -> Result<IssueFeedbackDesired> {
    let spans = secret_spans(IssueTextField::Body, body);
    let masked = mask_issue_text("", body, &spans, &policy.publish)
        .map_err(|_| anyhow!("feedback disclosure is blocked"))?;
    let desired = IssueFeedbackDesired {
        comment_body_sha256: feedback_body_hash(&masked.shared_copy.body),
        comment_body: masked.shared_copy.body,
    };
    desired.validate().map_err(anyhow::Error::msg)?;
    Ok(desired)
}
fn render_status(snapshot: &IssueFeedbackSnapshot, policy: &IssueAutomationPolicy) -> String {
    let tracked = snapshot.item.tracked.as_ref().map(|value| value.state);
    let phase = if tracked == Some(IssueAutomationState::Cancelled) {
        "취소됨"
    } else if let Some(execution) = &snapshot.execution {
        match execution.progress.state {
            IssueExecutionState::Queued => "실행 대기",
            IssueExecutionState::Running => "실행 중",
            IssueExecutionState::InReview => "리뷰 중",
            IssueExecutionState::Done => "통합 확인",
            IssueExecutionState::Failed => "사람의 복구 대기",
        }
    } else {
        match tracked {
            Some(IssueAutomationState::Done) => "통합 확인",
            Some(IssueAutomationState::Failed) => "사람의 복구 대기",
            Some(IssueAutomationState::Running) => "실행 중",
            Some(IssueAutomationState::InReview) => "리뷰 중",
            _ => "실행 대기",
        }
    };
    let mut body = format!("### Akra 작업 상태\n\n상태: {phase}");
    if let Some(execution) = &snapshot.execution {
        let attempts = execution.progress.attempts;
        body.push_str(&format!(
            "\n\n실행: {}/{}\n검증: {}",
            attempts.execution, policy.budget.max_attempts, attempts.verification
        ));
        if let Some(terminal) = &execution.progress.terminal {
            body.push_str("\n\n다음 행동: ");
            body.push_str(terminal.classification.recovery_summary());
        }
        if execution.progress.integration.is_some() {
            body.push_str("\n\n통합을 확인했습니다. 외부 이슈의 닫힘은 별도 상태입니다.");
        }
    }
    body
}
