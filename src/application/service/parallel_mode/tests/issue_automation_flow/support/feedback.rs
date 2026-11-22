use super::*;
use crate::application::port::outbound::issue_feedback_port::{
    IssueFeedbackClosureSnapshot, IssueFeedbackLabelSnapshot, IssueFeedbackPort,
    IssueFeedbackReadResult, IssueFeedbackWriteResult,
};
use crate::application::port::outbound::issue_feedback_store_port::IssueFeedbackStorePort;
use crate::domain::issue_automation::feedback::{
    IssueFeedbackEvidence, IssueFeedbackIntent, IssueFeedbackIntentState, IssueFeedbackOperation,
    IssueFeedbackReceipt, feedback_body_hash,
};
use crate::domain::issue_automation::source::{IssueRemoteState, IssueSourceRead};
use std::sync::atomic::{AtomicUsize, Ordering};

const ACTOR: &str = "U_flow_feedback";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlowRemoteComment {
    pub comment_id: String,
    pub issue: IssueReference,
    pub actor_id: String,
    pub intent_id: String,
    pub body: String,
    pub body_sha256: String,
}

/// Emulates remote effects, never local journal transitions or policy decisions.
/// A second create call really creates a second comment, exposing duplicate sends.
pub(crate) struct FlowFeedbackProvider {
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    source: Arc<FlowSource>,
    writes: Mutex<Vec<IssueFeedbackIntent>>,
    comments: Mutex<BTreeMap<String, FlowRemoteComment>>,
    next_lost_acks: AtomicUsize,
    next_failed_reads: AtomicUsize,
    reconciles: AtomicUsize,
}
impl FlowFeedbackProvider {
    pub fn new(store: Arc<SqliteIssueAutomationStoreAdapter>, source: Arc<FlowSource>) -> Self {
        Self {
            store,
            source,
            writes: Mutex::new(Vec::new()),
            comments: Mutex::new(BTreeMap::new()),
            next_lost_acks: AtomicUsize::new(0),
            next_failed_reads: AtomicUsize::new(0),
            reconciles: AtomicUsize::new(0),
        }
    }
    pub fn lose_next_comment_ack(&self) {
        self.next_lost_acks.fetch_add(1, Ordering::SeqCst);
    }
    pub fn fail_next_comment_readback(&self) {
        self.next_failed_reads.fetch_add(1, Ordering::SeqCst);
    }
    pub fn writes(&self) -> Vec<IssueFeedbackIntent> {
        self.writes.lock().unwrap().clone()
    }
    pub fn comments(&self) -> Vec<FlowRemoteComment> {
        self.comments.lock().unwrap().values().cloned().collect()
    }
    pub fn comment_count(&self) -> usize {
        self.comments.lock().unwrap().len()
    }
    pub fn create_count(&self) -> usize {
        self.writes
            .lock()
            .unwrap()
            .iter()
            .filter(|intent| {
                matches!(
                    intent.operation,
                    IssueFeedbackOperation::CreateComment { .. }
                )
            })
            .count()
    }
    pub fn reconcile_count(&self) -> usize {
        self.reconciles.load(Ordering::SeqCst)
    }
}
fn consume_once(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            value.checked_sub(1)
        })
        .is_ok()
}
impl IssueFeedbackPort for FlowFeedbackProvider {
    fn actor_id(&self, issue: &IssueReference) -> Result<String> {
        ensure!(
            *issue == self.source.issue(),
            "feedback actor scope changed"
        );
        Ok(ACTOR.into())
    }
    fn read_label(&self, issue: &IssueReference, _: &str) -> Result<IssueFeedbackLabelSnapshot> {
        ensure!(
            *issue == self.source.issue(),
            "feedback label scope changed"
        );
        Ok(IssueFeedbackLabelSnapshot {
            issue: issue.clone(),
            label: None,
            present: false,
            latest_event: None,
            complete: true,
        })
    }
    fn read_closure(&self, issue: &IssueReference) -> Result<IssueFeedbackClosureSnapshot> {
        ensure!(
            *issue == self.source.issue(),
            "feedback closure scope changed"
        );
        let read = self.source.read.lock().unwrap();
        let IssueSourceRead::Observed(observed) = &*read else {
            anyhow::bail!("remote issue state unavailable");
        };
        Ok(IssueFeedbackClosureSnapshot {
            issue: issue.clone(),
            closed: observed.remote_state == IssueRemoteState::Closed,
            latest_event: None,
            complete: true,
        })
    }
    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        let records = self.store.unresolved(&intent.issue)?;
        let durable = records
            .iter()
            .find(|record| record.intent.intent_id == intent.intent_id)
            .ok_or_else(|| anyhow!("feedback mutation has no durable intent"))?;
        ensure!(
            durable.intent == *intent && durable.state == IssueFeedbackIntentState::Sent,
            "the exact Sent CAS must precede the remote mutation"
        );
        let mut comments = self.comments.lock().unwrap();
        let (comment_id, body, body_sha256) = match &intent.operation {
            IssueFeedbackOperation::CreateComment {
                body, body_sha256, ..
            } => (format!("IC_flow_{}", comments.len() + 1), body, body_sha256),
            IssueFeedbackOperation::UpdateComment {
                owner,
                body,
                body_sha256,
                ..
            } => {
                let existing = comments
                    .get(&owner.comment_id)
                    .ok_or_else(|| anyhow!("owned remote comment was removed"))?;
                ensure!(
                    existing.issue == owner.issue
                        && existing.actor_id == owner.actor_id
                        && existing.intent_id == owner.latest_intent_id
                        && existing.body_sha256 == owner.body_sha256,
                    "the remote comment owner changed before update"
                );
                (owner.comment_id.clone(), body, body_sha256)
            }
            _ => anyhow::bail!("the comment-only flow unexpectedly attempted another mutation"),
        };
        ensure!(
            feedback_body_hash(body) == *body_sha256,
            "remote body hash differs"
        );
        comments.insert(
            comment_id.clone(),
            FlowRemoteComment {
                comment_id: comment_id.clone(),
                issue: intent.issue.clone(),
                actor_id: intent.actor_id.clone(),
                intent_id: intent.intent_id.clone(),
                body: body.clone(),
                body_sha256: body_sha256.clone(),
            },
        );
        self.writes.lock().unwrap().push(intent.clone());
        if consume_once(&self.next_lost_acks) {
            Ok(IssueFeedbackWriteResult::Unconfirmed)
        } else {
            Ok(IssueFeedbackWriteResult::Acknowledged {
                comment_id: Some(comment_id),
            })
        }
    }
    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        self.reconciles.fetch_add(1, Ordering::SeqCst);
        if consume_once(&self.next_failed_reads) {
            anyhow::bail!("remote comment readback unavailable");
        }
        let comments = self.comments.lock().unwrap();
        let matching: Vec<_> = comments
            .values()
            .filter(|comment| {
                comment.issue == intent.issue
                    && comment.actor_id == intent.actor_id
                    && comment.intent_id == intent.intent_id
            })
            .collect();
        if matching.len() != 1 {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        }
        let comment = matching[0];
        let expected_hash = match &intent.operation {
            IssueFeedbackOperation::CreateComment { body_sha256, .. } => body_sha256,
            IssueFeedbackOperation::UpdateComment {
                owner, body_sha256, ..
            } => {
                if owner.comment_id != comment.comment_id {
                    return Ok(IssueFeedbackReadResult::Contradictory);
                }
                body_sha256
            }
            _ => anyhow::bail!("the comment-only flow unexpectedly reconciled another mutation"),
        };
        if comment.body_sha256 != *expected_hash
            || feedback_body_hash(&comment.body) != *expected_hash
        {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        Ok(IssueFeedbackReadResult::Confirmed(Box::new(
            IssueFeedbackReceipt {
                issue: intent.issue.clone(),
                intent_id: intent.intent_id.clone(),
                actor_id: intent.actor_id.clone(),
                evidence: IssueFeedbackEvidence::Comment {
                    comment_id: comment.comment_id.clone(),
                    body_sha256: comment.body_sha256.clone(),
                },
            },
        )))
    }
}
