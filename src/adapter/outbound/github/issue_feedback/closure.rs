use anyhow::{Result, anyhow, bail};

use super::GithubIssueFeedbackAdapter;
use super::graphql;
use super::pagination::Pages;
use crate::adapter::outbound::github::issue_snapshot::ensure_node_id;
use crate::application::port::outbound::issue_feedback_port::{
    IssueFeedbackClosureEvent, IssueFeedbackClosureSnapshot, IssueFeedbackReadResult,
    IssueFeedbackWriteResult,
};
use crate::domain::issue_automation::feedback::{
    IssueFeedbackEvidence, IssueFeedbackIntent, IssueFeedbackOperation,
};
use crate::domain::issue_work::IssueReference;

pub(super) struct ClosureObservation {
    pub snapshot: IssueFeedbackClosureSnapshot,
    events: Vec<IssueFeedbackClosureEvent>,
}

impl GithubIssueFeedbackAdapter {
    pub(super) fn closure_observation(&self, issue: &IssueReference) -> Result<ClosureObservation> {
        let mut pages = Pages::default();
        let mut after: Option<String> = None;
        let mut closed = None;
        let mut events = Vec::new();
        loop {
            let mut fields = vec![("id", issue.issue_id.as_str())];
            if let Some(cursor) = &after {
                fields.push(("after", cursor));
            }
            let response: graphql::NodeData<graphql::IssueNode> =
                self.query(graphql::CLOSURE_EVENTS, &fields)?;
            let node = response
                .node
                .ok_or_else(|| anyhow!("GitHub did not return the feedback issue"))?;
            Self::require_issue(&node, issue)?;
            let current_closed = match node.state.as_deref() {
                Some("OPEN") => false,
                Some("CLOSED") => true,
                _ => bail!("GitHub did not return the feedback issue state"),
            };
            if closed.is_some_and(|previous| previous != current_closed) {
                bail!("GitHub changed the feedback issue while it was being read")
            }
            closed = Some(current_closed);
            let connection = node
                .closure_events
                .ok_or_else(|| anyhow!("GitHub did not return feedback closure events"))?;
            let (nodes, next) = pages.accept(connection, |event| event.id.as_str())?;
            for event in nodes {
                ensure_node_id(&event.id)?;
                Self::require_issue(&event.closable, issue)?;
                let event_closed = match event.typename.as_str() {
                    "ClosedEvent" => true,
                    "ReopenedEvent" => false,
                    _ => bail!("GitHub returned an unknown feedback closure event"),
                };
                events.push(IssueFeedbackClosureEvent {
                    event_id: event.id,
                    actor_id: Self::actor(event.actor.as_ref()).map(str::to_string),
                    closed: event_closed,
                });
            }
            let Some(next) = next else {
                let latest_event = events.last().cloned();
                if latest_event
                    .as_ref()
                    .map_or(current_closed, |event| event.closed != current_closed)
                {
                    bail!("GitHub changed the feedback issue closure while it was being read")
                }
                return Ok(ClosureObservation {
                    snapshot: IssueFeedbackClosureSnapshot {
                        issue: issue.clone(),
                        closed: current_closed,
                        latest_event,
                        complete: true,
                    },
                    events,
                });
            };
            after = Some(next);
        }
    }

    pub(super) fn write_closure(
        &self,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackWriteResult> {
        let IssueFeedbackOperation::CloseIssue {
            previous_event_id, ..
        } = &intent.operation
        else {
            bail!("invalid closure feedback operation");
        };
        let observed = match self.closure_observation(&intent.issue) {
            Ok(observed) => observed.snapshot,
            Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
        };
        if !observed.complete
            || observed.closed
            || observed
                .latest_event
                .as_ref()
                .map(|event| event.event_id.as_str())
                != previous_event_id.as_deref()
        {
            // An already closed issue is not evidence that this intent caused the closure.
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        // GitHub has no conditional close CAS. The durable journal permits only this one attempt.
        let response = self.query::<graphql::MutationData>(
            graphql::CLOSE_ISSUE,
            &[
                ("id", &intent.issue.issue_id),
                ("intent", &intent.intent_id),
            ],
        );
        let payload = match response {
            Ok(data) => match data.feedback {
                Some(payload) => payload,
                None => return Ok(IssueFeedbackWriteResult::Unconfirmed),
            },
            Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
        };
        match payload.client_mutation_id.as_deref() {
            Some(id) if id == intent.intent_id => {}
            Some(_) => return Ok(IssueFeedbackWriteResult::Contradictory),
            None => return Ok(IssueFeedbackWriteResult::Unconfirmed),
        }
        let Some(node) = payload.issue else {
            return Ok(IssueFeedbackWriteResult::Unconfirmed);
        };
        if !Self::matches_issue(&node, &intent.issue) {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        match node.state.as_deref() {
            Some("CLOSED") => Ok(IssueFeedbackWriteResult::Acknowledged { comment_id: None }),
            Some("OPEN") => Ok(IssueFeedbackWriteResult::Contradictory),
            _ => Ok(IssueFeedbackWriteResult::Unconfirmed),
        }
    }

    pub(super) fn reconcile_closure(
        &self,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackReadResult> {
        let IssueFeedbackOperation::CloseIssue {
            previous_event_id, ..
        } = &intent.operation
        else {
            bail!("invalid closure feedback operation");
        };
        let observed = self.closure_observation(&intent.issue)?;
        let start = if let Some(previous) = previous_event_id {
            let Some(index) = observed
                .events
                .iter()
                .position(|event| event.event_id == *previous)
            else {
                return Ok(IssueFeedbackReadResult::NeedsConfirmation);
            };
            if observed.events[index].closed {
                return Ok(IssueFeedbackReadResult::Contradictory);
            }
            index + 1
        } else {
            0
        };
        let new_events = &observed.events[start..];
        let [event] = new_events else {
            return Ok(if new_events.is_empty() {
                IssueFeedbackReadResult::NeedsConfirmation
            } else {
                IssueFeedbackReadResult::Contradictory
            });
        };
        let Some(actor) = &event.actor_id else {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        };
        if !observed.snapshot.complete
            || !observed.snapshot.closed
            || !event.closed
            || actor != &intent.actor_id
        {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        Self::confirmed(
            intent,
            IssueFeedbackEvidence::Closed {
                event_id: event.event_id.clone(),
            },
        )
    }
}
