use super::*;

pub(crate) struct FakeStore {
    pub data: Mutex<Data>,
    pub after_snapshot: Hook,
    pub after_record: Hook,
    pub after_record_read: Hook,
    pub fail_ack_save: AtomicBool,
}
pub(crate) struct Data {
    pub snapshot: IssueFeedbackSnapshot,
    pub records: Vec<IssueFeedbackIntentRecord>,
    pub events: Vec<IssueFeedbackEvent>,
}
impl FakeStore {
    pub fn new(issue: IssueReference) -> Self {
        Self {
            data: Mutex::new(Data {
                snapshot: snapshot(issue),
                records: Vec::new(),
                events: Vec::new(),
            }),
            after_snapshot: Mutex::new(None),
            after_record: Mutex::new(None),
            after_record_read: Mutex::new(None),
            fail_ack_save: AtomicBool::new(false),
        }
    }
    pub fn record(&self) -> IssueFeedbackIntentRecord {
        self.data.lock().unwrap().records.last().unwrap().clone()
    }
    pub fn snapshot_value(&self) -> IssueFeedbackSnapshot {
        self.data.lock().unwrap().snapshot.clone()
    }
}
impl IssueFeedbackStorePort for FakeStore {
    fn snapshot(&self, issue: &IssueReference) -> Result<Option<IssueFeedbackSnapshot>> {
        let data = self.data.lock().unwrap();
        let snapshot = (data.snapshot.item.issue == *issue).then(|| data.snapshot.clone());
        let recorded = data
            .records
            .last()
            .is_some_and(|record| record.state == IssueFeedbackIntentState::Recorded);
        drop(data);
        if let Some(hook) = self.after_snapshot.lock().unwrap().take() {
            hook();
        }
        if recorded && let Some(hook) = self.after_record_read.lock().unwrap().take() {
            hook();
        }
        Ok(snapshot)
    }
    fn save_desired(
        &self,
        issue: &IssueReference,
        expected: IssueFeedbackFence,
        desired: &IssueFeedbackDesired,
    ) -> Result<IssueFeedbackSaveResult> {
        let mut data = self.data.lock().unwrap();
        if data.snapshot.item.issue != *issue || data.snapshot.fence() != expected {
            return Ok(IssueFeedbackSaveResult::Conflict);
        }
        if data.snapshot.state.desired.as_ref() != Some(desired) {
            data.snapshot.state.desired = Some(desired.clone());
            data.snapshot.state.desired_revision += 1;
            data.snapshot.state.revision += 1;
        }
        Ok(IssueFeedbackSaveResult::Saved(Box::new(
            data.snapshot.state.clone(),
        )))
    }
    fn record(
        &self,
        expected: IssueFeedbackFence,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackRecordResult> {
        intent.validate().map_err(anyhow::Error::msg)?;
        let mut data = self.data.lock().unwrap();
        if let Some(record) = data
            .records
            .iter()
            .find(|r| r.intent.intent_id == intent.intent_id)
        {
            anyhow::ensure!(record.intent == *intent, "immutable feedback intent");
            return Ok(IssueFeedbackRecordResult::Existing(record.clone()));
        }
        if data
            .records
            .iter()
            .any(|r| r.intent.issue.same_remote_issue(&intent.issue) && r.state.is_unresolved())
        {
            return Ok(IssueFeedbackRecordResult::Blocked);
        }
        if data.snapshot.fence() != expected {
            return Ok(IssueFeedbackRecordResult::Conflict);
        }
        let record = IssueFeedbackIntentRecord {
            intent: intent.clone(),
            state: IssueFeedbackIntentState::Recorded,
            revision: 1,
            receipt: None,
            acknowledged_comment_id: None,
            contradictory_response: false,
            closure_acknowledged: false,
        };
        data.records.push(record.clone());
        data.snapshot.state.revision += 1;
        drop(data);
        if let Some(hook) = self.after_record.lock().unwrap().take() {
            hook();
        }
        Ok(IssueFeedbackRecordResult::Created(record))
    }
    fn claim_sent(
        &self,
        issue: &IssueReference,
        id: &str,
        revision: u64,
        expected: IssueFeedbackFence,
    ) -> Result<IssueFeedbackClaimResult> {
        let mut data = self.data.lock().unwrap();
        if data.snapshot.fence() != expected {
            return Ok(IssueFeedbackClaimResult::Conflict);
        }
        let Some(index) = data
            .records
            .iter()
            .position(|r| r.intent.intent_id == id && r.intent.issue == *issue)
        else {
            return Ok(IssueFeedbackClaimResult::Conflict);
        };
        let record = &mut data.records[index];
        if record.state != IssueFeedbackIntentState::Recorded {
            return Ok(IssueFeedbackClaimResult::Existing(record.clone()));
        }
        if record.revision != revision {
            return Ok(IssueFeedbackClaimResult::Conflict);
        }
        record.state = IssueFeedbackIntentState::Sent;
        record.revision += 1;
        let result = record.clone();
        data.snapshot.state.revision += 1;
        data.snapshot.publication_attempts += 1;
        Ok(IssueFeedbackClaimResult::Claimed(result))
    }
    fn complete(
        &self,
        issue: &IssueReference,
        id: &str,
        revision: u64,
        completion: &IssueFeedbackCompletion,
    ) -> Result<Option<IssueFeedbackIntentRecord>> {
        if matches!(
            completion,
            IssueFeedbackCompletion::NeedsConfirmation {
                acknowledged_comment_id: Some(_),
                ..
            }
        ) && self.fail_ack_save.swap(false, Ordering::SeqCst)
        {
            bail!("simulated durable ACK failure");
        }
        let mut data = self.data.lock().unwrap();
        let Some(index) = data
            .records
            .iter()
            .position(|r| r.intent.intent_id == id && r.intent.issue == *issue)
        else {
            return Ok(None);
        };
        let mut next = data.records[index].clone();
        if next.revision != revision {
            return Ok(None);
        }
        match completion {
            IssueFeedbackCompletion::CloseAcknowledged => {
                anyhow::ensure!(
                    matches!(
                        &next.intent.operation,
                        IssueFeedbackOperation::CloseIssue { .. }
                    ),
                    "not a close intent"
                );
                next.closure_acknowledged = true;
                next.state = IssueFeedbackIntentState::NeedsConfirmation;
            }
            IssueFeedbackCompletion::Abandoned => {
                anyhow::ensure!(
                    next.state == IssueFeedbackIntentState::Recorded,
                    "cannot abandon a sent intent"
                );
                next.state = IssueFeedbackIntentState::Abandoned;
            }
            IssueFeedbackCompletion::NeedsConfirmation {
                contradictory,
                acknowledged_comment_id,
            } => {
                next.state = IssueFeedbackIntentState::NeedsConfirmation;
                next.contradictory_response |= *contradictory;
                if let Some(id) = acknowledged_comment_id {
                    anyhow::ensure!(
                        next.acknowledged_comment_id
                            .as_ref()
                            .is_none_or(|old| old == id),
                        "changed ACK"
                    );
                    next.acknowledged_comment_id = Some(id.clone());
                }
            }
            IssueFeedbackCompletion::Confirmed(receipt) => {
                anyhow::ensure!(!next.contradictory_response, "sticky contradiction");
                receipt
                    .validate_for(&next.intent)
                    .map_err(anyhow::Error::msg)?;
                if let Some(ack) = &next.acknowledged_comment_id {
                    anyhow::ensure!(
                        matches!(&receipt.evidence, IssueFeedbackEvidence::Comment { comment_id, .. } if ack == comment_id),
                        "changed ACK"
                    );
                }
                next.state = IssueFeedbackIntentState::Confirmed;
                next.receipt = Some(receipt.as_ref().clone());
            }
        }
        if next == data.records[index] {
            return Ok(Some(next));
        }
        next.revision += 1;
        next.validate().map_err(anyhow::Error::msg)?;
        data.snapshot.state.revision += 1;
        if let Some(receipt) = &next.receipt {
            update_owner(&mut data.snapshot.state, &next.intent, receipt);
            let event_revision = data.snapshot.state.revision;
            data.events.push(IssueFeedbackEvent {
                event_id: format!("issue-feedback:{}", next.intent.intent_id),
                issue: issue.clone(),
                intent_id: id.into(),
                action: next.intent.operation.action(),
                receipt: receipt.clone(),
                revision: event_revision,
            });
        }
        data.records[index] = next.clone();
        Ok(Some(next))
    }
    fn unresolved(&self, issue: &IssueReference) -> Result<Vec<IssueFeedbackIntentRecord>> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .records
            .iter()
            .filter(|r| r.intent.issue.same_remote_issue(issue) && r.state.is_unresolved())
            .cloned()
            .collect())
    }
    fn pending(&self, _: &str) -> Result<Vec<IssueFeedbackIntentRecord>> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .records
            .iter()
            .filter(|r| r.state.is_unresolved())
            .cloned()
            .collect())
    }
    fn mark_first_grant(
        &self,
        issue: &IssueReference,
        order: &str,
        expected: IssueFeedbackFence,
    ) -> Result<IssueFeedbackGrantResult> {
        let mut data = self.data.lock().unwrap();
        if data.snapshot.item.issue != *issue
            || data
                .snapshot
                .work
                .as_ref()
                .is_none_or(|w| w.order.work_order_id != order)
        {
            return Ok(IssueFeedbackGrantResult::Conflict);
        }
        if data.snapshot.first_grant_entered {
            return Ok(IssueFeedbackGrantResult::AlreadyEntered);
        }
        if data.snapshot.fence() != expected || data.records.iter().any(|r| r.state.is_unresolved())
        {
            return Ok(IssueFeedbackGrantResult::Conflict);
        }
        data.snapshot.first_grant_entered = true;
        data.snapshot.state.revision += 1;
        Ok(IssueFeedbackGrantResult::Marked)
    }
    fn pending_events(&self, _: &str) -> Result<Vec<IssueFeedbackEvent>> {
        Ok(self.data.lock().unwrap().events.clone())
    }
    fn acknowledge_event(&self, _: &str, id: &str) -> Result<()> {
        self.data
            .lock()
            .unwrap()
            .events
            .retain(|event| event.event_id != id);
        Ok(())
    }
}
fn update_owner(
    state: &mut IssueFeedbackState,
    intent: &IssueFeedbackIntent,
    receipt: &IssueFeedbackReceipt,
) {
    match (&intent.operation, &receipt.evidence) {
        (
            IssueFeedbackOperation::CreateComment { content_sha256, .. }
            | IssueFeedbackOperation::UpdateComment { content_sha256, .. },
            IssueFeedbackEvidence::Comment {
                comment_id,
                body_sha256,
            },
        ) => {
            state.comment_owner = Some(IssueFeedbackCommentOwner {
                issue: intent.issue.clone(),
                actor_id: intent.actor_id.clone(),
                comment_id: comment_id.clone(),
                latest_intent_id: intent.intent_id.clone(),
                body_sha256: body_sha256.clone(),
                content_sha256: content_sha256.clone(),
            });
        }
        (IssueFeedbackOperation::AddLabel { .. }, _) => {
            state.label_proof = Some(receipt.clone());
            state.label_removal_confirmed = false;
        }
        (IssueFeedbackOperation::RemoveLabel { .. }, _) => state.label_removal_confirmed = true,
        (IssueFeedbackOperation::CloseIssue { .. }, IssueFeedbackEvidence::Closed { .. }) => {
            state.close_proof = Some(receipt.clone())
        }
        (
            IssueFeedbackOperation::SetBoardField { .. },
            IssueFeedbackEvidence::BoardField { .. },
        ) => state.board_proof = Some(receipt.clone()),
        _ => panic!("receipt was validated before this point"),
    }
}
