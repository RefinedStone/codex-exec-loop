use std::collections::BTreeSet;

use super::*;

struct StateEvent {
    id: String,
    actor: Option<String>,
    from: Option<String>,
    to: String,
}

impl LinearIssueFeedbackAdapter {
    pub(super) fn state_snapshot(
        &self,
        context: &FeedbackContext,
        issue: &IssueReference,
        mapping: &IssueAutomationLinearCloseState,
    ) -> Result<IssueFeedbackLinearStateSnapshot> {
        mapping
            .validate()
            .map_err(|_| anyhow!("invalid Linear close state mapping"))?;
        let before = self.issue_state(context, issue)?;
        if before.closed || !mapping.from_state_ids.contains(&before.id) {
            bail!("Linear close source state is not explicitly allowed");
        }
        // The destination must be a completed state of the same team, not a guessed name.
        let target = self.read(
            context,
            LinearIssueReadRequest::WorkflowState {
                state_id: mapping.to_state_id.clone(),
            },
        )?;
        if target.pointer("/workflowState/id").and_then(Value::as_str)
            != Some(mapping.to_state_id.as_str())
            || target
                .pointer("/workflowState/type")
                .and_then(Value::as_str)
                != Some("completed")
            || target
                .pointer("/workflowState/team/id")
                .and_then(Value::as_str)
                != Some(issue.repository_id.as_str())
        {
            bail!("Linear close target state changed");
        }
        let events = self.state_events(context, issue)?;
        if events.first().is_some_and(|event| event.to != before.id)
            || self.issue_state(context, issue)? != before
        {
            bail!("Linear state changed during close preparation");
        }
        Ok(IssueFeedbackLinearStateSnapshot {
            issue: issue.clone(),
            target: IssueFeedbackLinearState {
                from_state_id: before.id,
                to_state_id: mapping.to_state_id.clone(),
            },
            previous_event_id: events.first().map(|event| event.id.clone()),
        })
    }

    pub(super) fn write_state(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackWriteResult> {
        let IssueFeedbackOperation::CloseIssue {
            linear_state: Some(target),
            previous_event_id,
            ..
        } = &intent.operation
        else {
            bail!("Linear state intent is missing");
        };
        let Some(mapping) = context.binding.close_state.as_ref() else {
            bail!("Linear state changes are disabled");
        };
        let snapshot = self.state_snapshot(context, &intent.issue, mapping)?;
        if snapshot.target != *target || snapshot.previous_event_id != *previous_event_id {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        // Linear offers no conditional state CAS. Only the durable Sent winner may make this
        // one attempt, and a lost response remains unknown without an acknowledgement.
        let response = match self.send(
            context,
            intent,
            LinearIssueWriteRequest::UpdateState {
                issue_id: intent.issue.issue_id.clone(),
                state_id: target.to_state_id.clone(),
            },
        ) {
            Ok(response) => response,
            Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
        };
        let payload = &response["issueUpdate"];
        if payload.get("success").and_then(Value::as_bool) != Some(true)
            || payload.pointer("/issue/id").and_then(Value::as_str)
                != Some(intent.issue.issue_id.as_str())
            || payload.pointer("/issue/state/id").and_then(Value::as_str)
                != Some(target.to_state_id.as_str())
        {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        Ok(IssueFeedbackWriteResult::Acknowledged { comment_id: None })
    }

    pub(super) fn reconcile_state(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackReadResult> {
        let IssueFeedbackOperation::CloseIssue {
            linear_state: Some(target),
            previous_event_id,
            ..
        } = &intent.operation
        else {
            bail!("Linear state intent is missing");
        };
        let before = self.issue_state(context, &intent.issue)?;
        let events = self.state_events(context, &intent.issue)?;
        if self.issue_state(context, &intent.issue)? != before {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        }
        // History is newest first: the events after the snapshot precede its previous event.
        let new_events = match previous_event_id {
            Some(previous) => {
                let Some(index) = events.iter().position(|event| &event.id == previous) else {
                    return Ok(IssueFeedbackReadResult::NeedsConfirmation);
                };
                &events[..index]
            }
            None => events.as_slice(),
        };
        let Some(newest) = new_events.first() else {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        };
        if new_events.len() != 1
            || newest.from.as_deref() != Some(target.from_state_id.as_str())
            || newest.to != target.to_state_id
            || newest.actor.as_deref() != Some(intent.actor_id.as_str())
            || !before.closed
            || before.id != target.to_state_id
        {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        Self::confirmed(
            intent,
            IssueFeedbackEvidence::Closed {
                event_id: newest.id.clone(),
            },
        )
    }

    /// Workflow state changes only, newest first, as Linear's forward pagination returns them.
    fn state_events(
        &self,
        context: &FeedbackContext,
        issue: &IssueReference,
    ) -> Result<Vec<StateEvent>> {
        let mut after = None;
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        let mut last_at = None;
        for _ in 0..MAX_PAGES {
            let value = self.read(
                context,
                LinearIssueReadRequest::History {
                    issue_id: issue.issue_id.clone(),
                    after: after.clone(),
                },
            )?;
            if value.pointer("/issue/id").and_then(Value::as_str) != Some(issue.issue_id.as_str()) {
                bail!("Linear returned history of a different issue");
            }
            let (nodes, next) = page(&value["issue"]["history"], after.as_deref())?;
            for node in nodes {
                let id = string(node, "id")?;
                validate_linear_feedback_id(id).map_err(anyhow::Error::msg)?;
                if !seen.insert(id.to_owned()) {
                    bail!("duplicate Linear history identity");
                }
                let at = chrono::DateTime::parse_from_rfc3339(string(node, "createdAt")?)
                    .map_err(|_| anyhow!("invalid Linear history timestamp"))?;
                if last_at.is_some_and(|newer| newer < at) {
                    bail!("Linear history order changed");
                }
                last_at = Some(at);
                let Some(to) = optional_id(node, "toStateId")? else {
                    continue;
                };
                result.push(StateEvent {
                    id: id.to_owned(),
                    actor: optional_id(node, "actorId")?,
                    from: optional_id(node, "fromStateId")?,
                    to,
                });
            }
            match next {
                Some(next) => after = Some(next),
                None => return Ok(result),
            }
        }
        bail!("Linear state observation exceeded its bound")
    }
}

fn optional_id(value: &Value, field: &str) -> Result<Option<String>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => {
            validate_linear_feedback_id(id).map_err(anyhow::Error::msg)?;
            Ok(Some(id.clone()))
        }
        Some(_) => bail!("invalid Linear history identity"),
    }
}
