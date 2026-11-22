use super::*;
use crate::application::port::outbound::issue_feedback_port::{
    IssueFeedbackClosureEvent, IssueFeedbackClosureSnapshot, IssueFeedbackLabelEvent,
};

#[derive(Clone, Copy)]
pub(crate) enum WriteBehavior {
    Ack,
    Lost,
    Contradictory,
}
#[derive(Clone, Copy)]
pub(crate) enum ReadBehavior {
    Confirm,
    Unavailable,
    DifferentComment,
}

pub(crate) struct FakeProvider {
    pub writes: Mutex<Vec<IssueFeedbackIntent>>,
    pub reconciles: AtomicUsize,
    pub actor_reads: AtomicUsize,
    pub label: Mutex<IssueFeedbackLabelSnapshot>,
    pub closure: Mutex<IssueFeedbackClosureSnapshot>,
    pub write_behavior: Mutex<WriteBehavior>,
    pub read_behavior: Mutex<ReadBehavior>,
    pub after_actor: Hook,
    pub actor: Mutex<String>,
    store: Arc<FakeStore>,
    journal: Arc<FakePublication>,
}
impl FakeProvider {
    pub fn new(store: Arc<FakeStore>, journal: Arc<FakePublication>) -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            reconciles: AtomicUsize::new(0),
            actor_reads: AtomicUsize::new(0),
            label: Mutex::new(IssueFeedbackLabelSnapshot {
                issue: issue(),
                label: Some(IssueSourceLabel {
                    id: "L_running".into(),
                    name: ISSUE_FEEDBACK_PROGRESS_LABEL.into(),
                }),
                present: false,
                latest_event: None,
                complete: true,
            }),
            closure: Mutex::new(IssueFeedbackClosureSnapshot {
                issue: issue(),
                closed: false,
                latest_event: None,
                complete: true,
            }),
            write_behavior: Mutex::new(WriteBehavior::Ack),
            read_behavior: Mutex::new(ReadBehavior::Confirm),
            after_actor: Mutex::new(None),
            actor: Mutex::new(ACTOR.into()),
            store,
            journal,
        }
    }
    fn assert_lease(&self) {
        assert!(self.journal.held.load(Ordering::SeqCst));
    }
}
impl IssueFeedbackPort for FakeProvider {
    fn actor_id(&self, _: &IssueReference) -> Result<String> {
        self.assert_lease();
        self.actor_reads.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.after_actor.lock().unwrap().take() {
            hook();
        }
        Ok(self.actor.lock().unwrap().clone())
    }
    fn read_label(&self, issue: &IssueReference, name: &str) -> Result<IssueFeedbackLabelSnapshot> {
        self.assert_lease();
        assert_eq!(*issue, self.label.lock().unwrap().issue);
        assert_eq!(name, ISSUE_FEEDBACK_PROGRESS_LABEL);
        Ok(self.label.lock().unwrap().clone())
    }
    fn read_closure(&self, issue: &IssueReference) -> Result<IssueFeedbackClosureSnapshot> {
        self.assert_lease();
        let state = self.closure.lock().unwrap().clone();
        assert_eq!(state.issue, *issue);
        Ok(state)
    }
    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        self.assert_lease();
        let stored = self.store.record();
        assert_eq!(
            stored.intent, *intent,
            "the exact intent must be durable before mutation"
        );
        assert_eq!(stored.state, IssueFeedbackIntentState::Sent);
        let mut writes = self.writes.lock().unwrap();
        writes.push(intent.clone());
        let event_id = format!("LE_{}", writes.len());
        if let Some(label) = intent.operation.label() {
            let applied = matches!(intent.operation, IssueFeedbackOperation::AddLabel { .. });
            *self.label.lock().unwrap() = IssueFeedbackLabelSnapshot {
                issue: intent.issue.clone(),
                label: Some(label.clone()),
                present: applied,
                complete: true,
                latest_event: Some(IssueFeedbackLabelEvent {
                    event_id,
                    actor_id: Some(intent.actor_id.clone()),
                    applied,
                }),
            };
        }
        if matches!(&intent.operation, IssueFeedbackOperation::CloseIssue { .. }) {
            *self.closure.lock().unwrap() = IssueFeedbackClosureSnapshot {
                issue: intent.issue.clone(),
                closed: true,
                latest_event: Some(IssueFeedbackClosureEvent {
                    event_id: format!("CE_{}", writes.len()),
                    actor_id: Some(intent.actor_id.clone()),
                    closed: true,
                }),
                complete: true,
            };
        }
        Ok(match *self.write_behavior.lock().unwrap() {
            WriteBehavior::Ack => IssueFeedbackWriteResult::Acknowledged {
                comment_id: intent
                    .operation
                    .is_comment()
                    .then(|| match &intent.operation {
                        IssueFeedbackOperation::UpdateComment { owner, .. } => {
                            owner.comment_id.clone()
                        }
                        _ => COMMENT.into(),
                    }),
            },
            WriteBehavior::Lost => IssueFeedbackWriteResult::Unconfirmed,
            WriteBehavior::Contradictory => IssueFeedbackWriteResult::Contradictory,
        })
    }
    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        self.assert_lease();
        self.reconciles.fetch_add(1, Ordering::SeqCst);
        let behavior = *self.read_behavior.lock().unwrap();
        if matches!(behavior, ReadBehavior::Unavailable) {
            bail!("read unavailable");
        }
        let evidence = match &intent.operation {
            IssueFeedbackOperation::CreateComment { body_sha256, .. }
            | IssueFeedbackOperation::UpdateComment { body_sha256, .. } => {
                IssueFeedbackEvidence::Comment {
                    comment_id: if matches!(behavior, ReadBehavior::DifferentComment) {
                        "IC_wrong".into()
                    } else {
                        COMMENT.into()
                    },
                    body_sha256: body_sha256.clone(),
                }
            }
            IssueFeedbackOperation::CloseIssue { .. } => IssueFeedbackEvidence::Closed {
                event_id: self
                    .closure
                    .lock()
                    .unwrap()
                    .latest_event
                    .as_ref()
                    .unwrap()
                    .event_id
                    .clone(),
            },
            IssueFeedbackOperation::AddLabel { label, .. }
            | IssueFeedbackOperation::RemoveLabel { label, .. } => {
                let state = self.label.lock().unwrap();
                IssueFeedbackEvidence::Label {
                    label: label.clone(),
                    event_id: state.latest_event.as_ref().unwrap().event_id.clone(),
                    applied: state.present,
                }
            }
        };
        Ok(IssueFeedbackReadResult::Confirmed(Box::new(
            IssueFeedbackReceipt {
                issue: intent.issue.clone(),
                intent_id: intent.intent_id.clone(),
                actor_id: intent.actor_id.clone(),
                evidence,
            },
        )))
    }
}
