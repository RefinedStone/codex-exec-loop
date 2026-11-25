use super::*;

impl IssueFeedbackService {
    pub(super) fn record_and_send(
        &self,
        snapshot: &IssueFeedbackSnapshot,
        intent: IssueFeedbackIntent,
        cancel: &IssueCollectionCancellation,
        lease: &dyn IssuePublicationLease,
    ) -> Result<Option<IssueFeedbackIntentRecord>> {
        intent.validate().map_err(anyhow::Error::msg)?;
        if cancel.is_cancelled() || self.blocked_by_unresolved(&intent.issue)? {
            return Ok(None);
        }
        // Keep the originally observed binding generation through Sent, remote ACK/Unknown
        // and receipt settlement. Pause completion must not overtake an admitted write.
        let action_guard = self.acquire_feedback_action(snapshot)?;
        lease.verify_identity()?;
        let Some(result) =
            cancel.commit_if_active(|| self.store.record(snapshot.fence(), &intent))?
        else {
            return Ok(None);
        };
        let record = match result {
            IssueFeedbackRecordResult::Created(record) => record,
            IssueFeedbackRecordResult::Existing(record) => return Ok(Some(record)),
            IssueFeedbackRecordResult::Blocked | IssueFeedbackRecordResult::Conflict => {
                return Ok(None);
            }
        };
        let Some(current) = self.current(&intent.issue)? else {
            return self.abandon_recorded(&record);
        };
        if !authority_matches(snapshot.fence(), current.fence())
            || !self.intent_policy_current(&record.intent, &current)?
            || self.publish_pending(&intent.issue)?
        {
            return self.abandon_recorded(&record);
        }
        lease.verify_identity()?;
        let Some(claim) = cancel.commit_if_active(|| {
            self.store.claim_sent(
                &intent.issue,
                &intent.intent_id,
                record.revision,
                current.fence(),
            )
        })?
        else {
            return self.abandon_recorded(&record);
        };
        let sent = match claim {
            IssueFeedbackClaimResult::Claimed(record) => record,
            IssueFeedbackClaimResult::Existing(record) => return Ok(Some(record)),
            IssueFeedbackClaimResult::Conflict => return self.abandon_recorded(&record),
        };
        if cancel.is_cancelled() {
            return self.unconfirmed(&sent, false, None);
        }
        lease.verify_identity()?;
        let Some(last) = self.current(&intent.issue)? else {
            return self.unconfirmed(&sent, false, None);
        };
        if !authority_matches(current.fence(), last.fence())
            || !self.intent_policy_current(&intent, &last)?
        {
            return self.unconfirmed(&sent, false, None);
        }
        // Sent is durable before this sole mutation. Cancellation gates never hold during remote I/O.
        if action_guard.validate().is_err() {
            return self.unconfirmed(&sent, false, None);
        }
        let answer = if matches!(
            &intent.operation,
            IssueFeedbackOperation::SetBoardField { .. }
        ) {
            self.write_board_field(&intent)
        } else {
            self.provider
                .write(&intent)
                .unwrap_or(IssueFeedbackWriteResult::Unconfirmed)
        };
        lease.verify_identity()?;
        let record = match answer {
            IssueFeedbackWriteResult::Contradictory => return self.unconfirmed(&sent, true, None),
            IssueFeedbackWriteResult::Acknowledged { comment_id } => {
                if intent.operation.is_comment() != comment_id.is_some() {
                    return self.unconfirmed(&sent, true, None);
                }
                if let Some(id) = &comment_id {
                    if validate_feedback_id(id).is_err() {
                        return self.unconfirmed(&sent, true, None);
                    }
                    if let IssueFeedbackOperation::UpdateComment { owner, .. } = &intent.operation
                        && owner.comment_id != *id
                    {
                        return self.unconfirmed(&sent, true, None);
                    }
                }
                let acknowledged =
                    if matches!(&intent.operation, IssueFeedbackOperation::CloseIssue { .. }) {
                        self.store.complete(
                            &intent.issue,
                            &intent.intent_id,
                            sent.revision,
                            &IssueFeedbackCompletion::CloseAcknowledged,
                        )?
                    } else {
                        self.unconfirmed(&sent, false, comment_id)?
                    };
                let Some(record) = acknowledged else {
                    return Ok(None);
                };
                record
            }
            IssueFeedbackWriteResult::Unconfirmed => sent,
        };
        self.reconcile_record(record, lease)
    }

    fn intent_policy_current(
        &self,
        intent: &IssueFeedbackIntent,
        snapshot: &IssueFeedbackSnapshot,
    ) -> Result<bool> {
        let Some((automation, disclosure)) = self.write_context(snapshot)? else {
            return Ok(false);
        };
        Ok(actor_matches_snapshot(
            &IssueFeedbackActorContext {
                actor_id: intent.actor_id.clone(),
                connection: intent.connection.clone(),
            },
            snapshot,
        ) && automation.version == intent.automation_policy_version
            && issue_policy_version(&disclosure) == intent.disclosure_policy_version
            && if matches!(&intent.operation, IssueFeedbackOperation::CloseIssue { .. }) {
                self.close_policy_current(intent, snapshot, &automation.policy)
            } else if intent.operation.is_comment() {
                automation.policy.feedback.comment
            } else if let IssueFeedbackOperation::SetBoardField {
                project_id,
                field_id,
                option_id,
                ..
            } = &intent.operation
            {
                // 보내기 직전에도 보드 허가와 지금 단계의 목표를 다시 본다.
                super::board::board_target(snapshot, &automation.policy).is_some_and(
                    |(board, target)| {
                        board.project_id == *project_id
                            && board.field_id == *field_id
                            && target == option_id
                    },
                )
            } else {
                automation.policy.feedback.labels
                    && safe_progress_label(&automation.policy)
                    && match &intent.operation {
                        IssueFeedbackOperation::AddLabel { .. } => !snapshot.first_grant_entered,
                        IssueFeedbackOperation::RemoveLabel { .. } => {
                            snapshot.first_grant_entered
                                && terminal_for_feedback(snapshot)
                                && !snapshot.state.label_removal_confirmed
                        }
                        _ => false,
                    }
            })
    }
    fn abandon_recorded(
        &self,
        record: &IssueFeedbackIntentRecord,
    ) -> Result<Option<IssueFeedbackIntentRecord>> {
        self.store.complete(
            &record.intent.issue,
            &record.intent.intent_id,
            record.revision,
            &IssueFeedbackCompletion::Abandoned,
        )
    }
    fn unconfirmed(
        &self,
        record: &IssueFeedbackIntentRecord,
        contradictory: bool,
        acknowledged_comment_id: Option<String>,
    ) -> Result<Option<IssueFeedbackIntentRecord>> {
        self.store.complete(
            &record.intent.issue,
            &record.intent.intent_id,
            record.revision,
            &IssueFeedbackCompletion::NeedsConfirmation {
                contradictory,
                acknowledged_comment_id,
            },
        )
    }
    pub(super) fn reconcile_record(
        &self,
        record: IssueFeedbackIntentRecord,
        lease: &dyn IssuePublicationLease,
    ) -> Result<Option<IssueFeedbackIntentRecord>> {
        record.validate().map_err(anyhow::Error::msg)?;
        if record.state == IssueFeedbackIntentState::Recorded {
            // Recorded has proof that no mutation was dispatched. Recovery never sends it.
            return self.abandon_recorded(&record);
        }
        if !record.state.is_unresolved() || record.contradictory_response {
            return Ok(Some(record));
        }
        lease.verify_identity()?;
        let answer = if matches!(
            &record.intent.operation,
            IssueFeedbackOperation::SetBoardField { .. }
        ) {
            self.reconcile_board_field(&record.intent)
        } else {
            self.provider
                .reconcile(&record.intent)
                .unwrap_or(IssueFeedbackReadResult::NeedsConfirmation)
        };
        lease.verify_identity()?;
        match answer {
            IssueFeedbackReadResult::Confirmed(receipt) => {
                if matches!(
                    &record.intent.operation,
                    IssueFeedbackOperation::CloseIssue { .. }
                ) && !record.closure_acknowledged
                {
                    return self.unconfirmed(&record, false, None);
                }
                let ack_differs = record.acknowledged_comment_id.as_ref().is_some_and(|id| {
                    !matches!(&receipt.evidence, IssueFeedbackEvidence::Comment { comment_id, .. } if comment_id == id)
                });
                if ack_differs || receipt.validate_for(&record.intent).is_err() {
                    return self.unconfirmed(&record, true, None);
                }
                self.store.complete(
                    &record.intent.issue,
                    &record.intent.intent_id,
                    record.revision,
                    &IssueFeedbackCompletion::Confirmed(receipt),
                )
            }
            IssueFeedbackReadResult::NeedsConfirmation => self.unconfirmed(&record, false, None),
            IssueFeedbackReadResult::Contradictory => self.unconfirmed(&record, true, None),
        }
    }
}
