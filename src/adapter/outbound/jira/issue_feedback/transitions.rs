use std::collections::BTreeSet;

use super::*;

struct StatusEvent {
    id: String,
    actor: String,
    from: String,
    to: String,
}

impl JiraIssueFeedbackAdapter {
    pub(super) fn transition_snapshot(
        &self,
        context: &FeedbackContext,
        issue: &IssueReference,
        mapping: &IssueAutomationJiraCloseTransition,
    ) -> Result<IssueFeedbackJiraTransitionSnapshot> {
        mapping
            .validate()
            .map_err(|_| anyhow!("invalid Jira close transition mapping"))?;
        let before = self.issue_status(context, issue)?;
        if before.done || !mapping.from_status_ids.contains(&before.id) {
            bail!("Jira close source status is not explicitly allowed");
        }
        let available = self.read(
            context,
            JiraIssueFeedbackReadRequest::Transitions {
                issue_id: issue.issue_id.clone(),
            },
        )?;
        let transitions = available
            .get("transitions")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Jira transition observation is incomplete"))?;
        if transitions.len() > 256 {
            bail!("Jira transition bound exceeded");
        }
        let mut matching = transitions.iter().filter(|transition| {
            transition.get("id").and_then(Value::as_str) == Some(mapping.transition_id.as_str())
        });
        let transition = matching
            .next()
            .ok_or_else(|| anyhow!("the approved Jira transition is unavailable"))?;
        if matching.next().is_some()
            || transition.pointer("/to/id").and_then(Value::as_str)
                != Some(mapping.to_status_id.as_str())
            || transition
                .pointer("/to/statusCategory/key")
                .and_then(Value::as_str)
                != Some("done")
            || transition.get("isAvailable").and_then(Value::as_bool) == Some(false)
        {
            bail!("Jira transition target changed");
        }
        let fields = transition
            .get("fields")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("Jira transition requirements are incomplete"))?;
        if fields.len() > 256
            || fields
                .values()
                .any(|field| field.get("required").and_then(Value::as_bool) != Some(false))
        {
            // No guessed required fields, hidden defaults or repository-supplied field edits.
            bail!("Jira transition requires unsupported field input");
        }
        let events = self.status_events(context, issue)?;
        if events.last().is_some_and(|event| event.to != before.id)
            || self.issue_status(context, issue)? != before
        {
            bail!("Jira status changed during close preparation");
        }
        Ok(IssueFeedbackJiraTransitionSnapshot {
            issue: issue.clone(),
            target: IssueFeedbackJiraTransition {
                transition_id: mapping.transition_id.clone(),
                from_status_id: before.id,
                to_status_id: mapping.to_status_id.clone(),
            },
            previous_event_id: events.last().map(|event| event.id.clone()),
        })
    }

    pub(super) fn write_transition(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackWriteResult> {
        let IssueFeedbackOperation::CloseIssue {
            jira_transition: Some(target),
            previous_event_id,
            ..
        } = &intent.operation
        else {
            bail!("Jira transition intent is missing");
        };
        let Some(mapping) = context.binding.close_transition.as_ref() else {
            bail!("Jira transitions are disabled");
        };
        let snapshot = self.transition_snapshot(context, &intent.issue, mapping)?;
        if snapshot.target != *target || snapshot.previous_event_id != *previous_event_id {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        // Jira offers no conditional transition CAS. Only the durable Sent winner may make
        // this one attempt, and a lost 204 remains unknown without a durable acknowledgement.
        match self.send(
            context,
            intent,
            JiraIssueFeedbackWriteRequest::Transition {
                issue_id: intent.issue.issue_id.clone(),
                transition_id: target.transition_id.clone(),
            },
        ) {
            Ok(JiraIssueFeedbackWriteResponse::Transitioned) => {
                Ok(IssueFeedbackWriteResult::Acknowledged { comment_id: None })
            }
            Ok(JiraIssueFeedbackWriteResponse::Rejected(reason)) => {
                Ok(IssueFeedbackWriteResult::JiraRejected(reason))
            }
            Ok(_) => Ok(IssueFeedbackWriteResult::Contradictory),
            Err(_) => Ok(IssueFeedbackWriteResult::Unconfirmed),
        }
    }

    pub(super) fn reconcile_transition(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackReadResult> {
        let IssueFeedbackOperation::CloseIssue {
            jira_transition: Some(target),
            previous_event_id,
            ..
        } = &intent.operation
        else {
            bail!("Jira transition intent is missing");
        };
        let before = self.issue_status(context, &intent.issue)?;
        let events = self.status_events(context, &intent.issue)?;
        if self.issue_status(context, &intent.issue)? != before {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        }
        let Some(last) = events.last() else {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        };
        let new_events = match previous_event_id {
            Some(previous) => {
                let Some(index) = events.iter().position(|event| &event.id == previous) else {
                    return Ok(IssueFeedbackReadResult::NeedsConfirmation);
                };
                &events[index + 1..]
            }
            None => events.as_slice(),
        };
        if new_events.is_empty() {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        }
        if new_events.len() != 1
            || last.from != target.from_status_id
            || last.to != target.to_status_id
            || last.actor != intent.actor_id
            || !before.done
            || before.id != target.to_status_id
        {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        Self::confirmed(
            intent,
            IssueFeedbackEvidence::Closed {
                event_id: last.id.clone(),
            },
        )
    }

    fn status_events(
        &self,
        context: &FeedbackContext,
        issue: &IssueReference,
    ) -> Result<Vec<StatusEvent>> {
        let mut start = 0;
        let mut total = None;
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        let mut last_at = None;
        for _ in 0..MAX_PAGES {
            let page = self.read(
                context,
                JiraIssueFeedbackReadRequest::Changelog {
                    issue_id: issue.issue_id.clone(),
                    start_at: start,
                },
            )?;
            let (values, next) =
                super::comments::accept_page(&page, "values", start, &mut total, &mut seen)?;
            for value in values {
                let at = chrono::DateTime::parse_from_rfc3339(string(value, "created")?)
                    .or_else(|_| {
                        chrono::DateTime::parse_from_str(
                            string(value, "created").unwrap_or(""),
                            "%Y-%m-%dT%H:%M:%S%.f%z",
                        )
                    })
                    .map_err(|_| anyhow!("invalid Jira changelog timestamp"))?;
                if last_at.is_some_and(|old| old > at) {
                    bail!("Jira changelog order changed");
                }
                last_at = Some(at);
                let items = value
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow!("Jira changelog items are missing"))?;
                if items.len() > 256 {
                    bail!("Jira changelog bound exceeded");
                }
                let mut status_items = items
                    .iter()
                    .filter(|item| item.get("fieldId").and_then(Value::as_str) == Some("status"));
                let Some(status) = status_items.next() else {
                    continue;
                };
                if status_items.next().is_some() {
                    bail!("ambiguous Jira status history");
                }
                let from = string(status, "from")?;
                let to = string(status, "to")?;
                validate_jira_feedback_id(from).map_err(anyhow::Error::msg)?;
                validate_jira_feedback_id(to).map_err(anyhow::Error::msg)?;
                let actor = value
                    .pointer("/author/accountId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("Jira status author is missing"))?;
                validate_feedback_id(actor).map_err(anyhow::Error::msg)?;
                result.push(StatusEvent {
                    id: string(value, "id")?.to_owned(),
                    actor: actor.to_owned(),
                    from: from.to_owned(),
                    to: to.to_owned(),
                });
            }
            match next {
                Some(next) => start = next,
                None => return Ok(result),
            }
        }
        bail!("Jira status observation exceeded its bound")
    }
}
