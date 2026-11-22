use super::*;

impl IssueFeedbackService {
    /// 첫 실행 허가 전에만 시작 라벨을 쓴다. true는 permit 부여가 아니라 이 선행 조건의 완료다.
    pub fn ensure_first_grant(
        &self,
        issue: &IssueReference,
        work_order_id: &str,
        cancel: &IssueCollectionCancellation,
    ) -> Result<bool> {
        self.check_scope(issue)?;
        if cancel.is_cancelled() {
            return Ok(false);
        }
        let _gate = self.lock()?;
        let lease = self
            .publication
            .acquire_publication_lease(&self.workspace)?;
        lease.verify_identity()?;
        let Some(snapshot) = self.current(issue)? else {
            return Ok(false);
        };
        if snapshot
            .work
            .as_ref()
            .is_none_or(|work| work.order.work_order_id != work_order_id)
        {
            return Ok(false);
        }
        if snapshot.first_grant_entered {
            return Ok(true);
        }
        let Some((automation, disclosure)) = self.write_context(&snapshot)? else {
            return Ok(false);
        };
        if self.blocked_by_unresolved(issue)? {
            return Ok(false);
        }
        if !automation.policy.feedback.labels {
            return self.enter_first_grant(
                issue,
                work_order_id,
                &snapshot,
                &automation.version,
                cancel,
                lease.as_ref(),
            );
        }
        if !safe_progress_label(&automation.policy) {
            return Ok(false);
        }
        let observed = self
            .provider
            .read_label(issue, ISSUE_FEEDBACK_PROGRESS_LABEL)?;
        if !valid_label_read(issue, &observed) {
            return Ok(false);
        }
        if observed.present {
            if !owned_label_matches(&snapshot, &observed) {
                return Ok(false);
            }
            return self.enter_first_grant(
                issue,
                work_order_id,
                &snapshot,
                &automation.version,
                cancel,
                lease.as_ref(),
            );
        }
        let Some(label) = observed.label else {
            return Ok(false);
        };
        let actor_id = self.provider.actor_id(issue)?;
        let intent = IssueFeedbackIntent {
            intent_id: format!("{:032x}", rand::random::<u128>()),
            issue: issue.clone(),
            actor_id,
            automation_policy_version: automation.version.clone(),
            disclosure_policy_version: issue_policy_version(&disclosure),
            desired_revision: snapshot.state.desired_revision,
            operation: IssueFeedbackOperation::AddLabel {
                label,
                previous_event_id: observed.latest_event.map(|event| event.event_id),
            },
            recorded_at: chrono::Utc::now().to_rfc3339(),
        };
        let Some(record) = self.record_and_send(&snapshot, intent, cancel, lease.as_ref())? else {
            return Ok(false);
        };
        if record.state != IssueFeedbackIntentState::Confirmed {
            return Ok(false);
        }
        let Some(current) = self.current(issue)? else {
            return Ok(false);
        };
        if !authority_matches(snapshot.fence(), current.fence()) {
            return Ok(false);
        }
        let observed = self
            .provider
            .read_label(issue, ISSUE_FEEDBACK_PROGRESS_LABEL)?;
        if !valid_label_read(issue, &observed) || !owned_label_matches(&current, &observed) {
            return Ok(false);
        }
        self.enter_first_grant(
            issue,
            work_order_id,
            &current,
            &automation.version,
            cancel,
            lease.as_ref(),
        )
    }

    fn enter_first_grant(
        &self,
        issue: &IssueReference,
        work_order_id: &str,
        snapshot: &IssueFeedbackSnapshot,
        expected_policy_version: &str,
        cancel: &IssueCollectionCancellation,
        lease: &dyn IssuePublicationLease,
    ) -> Result<bool> {
        lease.verify_identity()?;
        let Some((current_policy, _)) = self.write_context(snapshot)? else {
            return Ok(false);
        };
        if current_policy.version != expected_policy_version
            || (current_policy.policy.feedback.labels
                && !safe_progress_label(&current_policy.policy))
        {
            return Ok(false);
        }
        let result = cancel.commit_if_active(|| {
            self.store
                .mark_first_grant(issue, work_order_id, snapshot.fence())
        })?;
        Ok(matches!(
            result,
            Some(IssueFeedbackGrantResult::Marked | IssueFeedbackGrantResult::AlreadyEntered)
        ))
    }
}
fn valid_label_read(issue: &IssueReference, observed: &IssueFeedbackLabelSnapshot) -> bool {
    observed.issue == *issue
        && observed.complete
        && observed
            .label
            .as_ref()
            .is_some_and(|label| label.name == ISSUE_FEEDBACK_PROGRESS_LABEL)
}
fn owned_label_matches(
    snapshot: &IssueFeedbackSnapshot,
    observed: &IssueFeedbackLabelSnapshot,
) -> bool {
    let Some(proof) = &snapshot.state.label_proof else {
        return false;
    };
    let Some(event) = &observed.latest_event else {
        return false;
    };
    let IssueFeedbackEvidence::Label {
        label,
        event_id,
        applied: true,
    } = &proof.evidence
    else {
        return false;
    };
    observed.present
        && event.applied
        && proof.issue == observed.issue
        && observed.label.as_ref() == Some(label)
        && event.event_id == *event_id
        && event.actor_id.as_ref() == Some(&proof.actor_id)
}

pub(super) fn terminal_for_feedback(snapshot: &IssueFeedbackSnapshot) -> bool {
    snapshot.item.tracked.as_ref().is_some_and(|tracked| {
        matches!(
            tracked.state,
            IssueAutomationState::Done
                | IssueAutomationState::Cancelled
                | IssueAutomationState::Failed
        )
    }) || snapshot.execution.as_ref().is_some_and(|execution| {
        matches!(
            execution.progress.state,
            IssueExecutionState::Done | IssueExecutionState::Failed
        )
    })
}
impl IssueFeedbackService {
    pub(super) fn remove_terminal_label(
        &self,
        snapshot: &IssueFeedbackSnapshot,
        automation: &ResolvedIssueAutomationPolicy,
        disclosure: &IssuePolicy,
        now_unix_ms: u64,
        cancel: &IssueCollectionCancellation,
        lease: &dyn IssuePublicationLease,
    ) -> Result<()> {
        if !snapshot.first_grant_entered
            || !terminal_for_feedback(snapshot)
            || snapshot.state.label_removal_confirmed
            || !safe_progress_label(&automation.policy)
            || self.blocked_by_unresolved(&snapshot.item.issue)?
        {
            return Ok(());
        }
        let Some(proof) = &snapshot.state.label_proof else {
            return Ok(());
        };
        let IssueFeedbackEvidence::Label {
            label,
            event_id,
            applied: true,
        } = &proof.evidence
        else {
            return Ok(());
        };
        let issue = &snapshot.item.issue;
        let observed = self
            .provider
            .read_label(issue, ISSUE_FEEDBACK_PROGRESS_LABEL)?;
        if !valid_label_read(issue, &observed) || !owned_label_matches(snapshot, &observed) {
            return Ok(());
        }
        let actor_id = self.provider.actor_id(issue)?;
        if actor_id != proof.actor_id {
            return Ok(());
        }
        let intent = IssueFeedbackIntent {
            intent_id: format!("{:032x}", rand::random::<u128>()),
            issue: issue.clone(),
            actor_id,
            automation_policy_version: automation.version.clone(),
            disclosure_policy_version: issue_policy_version(disclosure),
            desired_revision: snapshot.state.desired_revision,
            operation: IssueFeedbackOperation::RemoveLabel {
                label: label.clone(),
                owned_event_id: event_id.clone(),
            },
            recorded_at: format!("unix-ms:{now_unix_ms}"),
        };
        self.record_and_send(snapshot, intent, cancel, lease)?;
        Ok(())
    }
}
