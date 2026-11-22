use anyhow::{Result, anyhow, bail};

use super::GithubIssueFeedbackAdapter;
use super::graphql;
use super::pagination::Pages;
use crate::adapter::outbound::github::issue_snapshot::ensure_node_id;
use crate::application::port::outbound::issue_feedback_port::{
    IssueFeedbackLabelEvent, IssueFeedbackLabelSnapshot, IssueFeedbackReadResult,
    IssueFeedbackWriteResult,
};
use crate::domain::issue_automation::feedback::{
    ISSUE_FEEDBACK_PROGRESS_LABEL, IssueFeedbackEvidence, IssueFeedbackIntent,
    IssueFeedbackOperation,
};
use crate::domain::issue_automation::source::IssueSourceLabel;
use crate::domain::issue_work::IssueReference;

pub(super) struct LabelObservation {
    pub snapshot: IssueFeedbackLabelSnapshot,
    events: Vec<IssueFeedbackLabelEvent>,
}

impl GithubIssueFeedbackAdapter {
    pub(super) fn label_observation(
        &self,
        issue: &IssueReference,
        name: &str,
    ) -> Result<LabelObservation> {
        if name != ISSUE_FEEDBACK_PROGRESS_LABEL {
            bail!("feedback may only observe its progress label")
        }
        let response: graphql::NodeData<graphql::IssueNode> = self.query(
            graphql::RESOLVE_LABEL,
            &[("id", &issue.issue_id), ("name", name)],
        )?;
        let node = response
            .node
            .ok_or_else(|| anyhow!("GitHub did not return the feedback label"))?;
        Self::require_issue(&node, issue)?;
        let remote = node.repository.and_then(|repository| repository.label);
        let Some(remote) = remote else {
            return Ok(LabelObservation {
                snapshot: IssueFeedbackLabelSnapshot {
                    issue: issue.clone(),
                    label: None,
                    present: false,
                    latest_event: None,
                    complete: true,
                },
                events: Vec::new(),
            });
        };
        Self::validate_remote_label(&remote, issue)?;
        if remote.name != name {
            bail!("GitHub returned a different feedback label")
        }
        let label = IssueSourceLabel {
            id: remote.id,
            name: remote.name,
        };
        let labels = self.read_label_pages(
            graphql::LABELS,
            issue,
            |node| node.labels,
            |label| label.id.as_str(),
        )?;
        let mut present = false;
        for observed in labels {
            Self::validate_remote_label(&observed, issue)?;
            if observed.id == label.id {
                if observed.name != label.name {
                    bail!("GitHub changed the feedback label identity")
                }
                present = true;
            } else if observed.name == label.name {
                bail!("GitHub returned ambiguous feedback labels")
            }
        }
        let remote_events = self.read_label_pages(
            graphql::LABEL_EVENTS,
            issue,
            |node| node.timeline_items,
            |event| event.id.as_str(),
        )?;
        let mut events = Vec::new();
        for event in remote_events {
            ensure_node_id(&event.id)?;
            Self::validate_remote_label(&event.label, issue)?;
            let applied = match event.typename.as_str() {
                "LabeledEvent" => true,
                "UnlabeledEvent" => false,
                _ => bail!("GitHub returned an unknown feedback label event"),
            };
            if event.label.id != label.id {
                continue;
            }
            if event.label.name != label.name {
                bail!("GitHub changed the feedback label identity")
            }
            events.push(IssueFeedbackLabelEvent {
                event_id: event.id,
                actor_id: Self::actor(event.actor.as_ref()).map(str::to_string),
                applied,
            });
        }
        let latest_event = events.last().cloned();
        if latest_event
            .as_ref()
            .is_some_and(|event| event.applied != present)
        {
            bail!("GitHub changed the feedback label while it was being read")
        }
        Ok(LabelObservation {
            snapshot: IssueFeedbackLabelSnapshot {
                issue: issue.clone(),
                label: Some(label),
                present,
                latest_event,
                complete: true,
            },
            events,
        })
    }

    fn validate_remote_label(label: &graphql::Label, issue: &IssueReference) -> Result<()> {
        ensure_node_id(&label.id)?;
        if label.repository.id != issue.repository_id {
            bail!("GitHub returned a label from a different feedback repository")
        }
        Ok(())
    }

    fn read_label_pages<T>(
        &self,
        query: &str,
        issue: &IssueReference,
        connection: impl Fn(graphql::IssueNode) -> Option<graphql::Connection<T>>,
        node_id: impl Fn(&T) -> &str,
    ) -> Result<Vec<T>> {
        let mut pages = Pages::default();
        let mut after: Option<String> = None;
        let mut found = Vec::new();
        loop {
            let mut fields = vec![("id", issue.issue_id.as_str())];
            if let Some(cursor) = &after {
                fields.push(("after", cursor));
            }
            let response: graphql::NodeData<graphql::IssueNode> = self.query(query, &fields)?;
            let node = response
                .node
                .ok_or_else(|| anyhow!("GitHub did not return feedback label data"))?;
            Self::require_issue(&node, issue)?;
            let page = connection(node)
                .ok_or_else(|| anyhow!("GitHub did not return feedback label data"))?;
            let (nodes, next) = pages.accept(page, &node_id)?;
            found.extend(nodes);
            let Some(next) = next else {
                return Ok(found);
            };
            after = Some(next);
        }
    }

    pub(super) fn write_label(
        &self,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackWriteResult> {
        let label = intent
            .operation
            .label()
            .ok_or_else(|| anyhow!("invalid label feedback operation"))?;
        let observed = match self.label_observation(&intent.issue, &label.name) {
            Ok(observed) => observed.snapshot,
            Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
        };
        if observed.label.as_ref() != Some(label) || !observed.complete {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        let (query, current_basis) = match &intent.operation {
            IssueFeedbackOperation::AddLabel {
                previous_event_id, ..
            } => (
                graphql::ADD_LABEL,
                !observed.present
                    && observed
                        .latest_event
                        .as_ref()
                        .map(|event| event.event_id.as_str())
                        == previous_event_id.as_deref(),
            ),
            IssueFeedbackOperation::RemoveLabel { owned_event_id, .. } => (
                graphql::REMOVE_LABEL,
                observed.present
                    && observed.latest_event.as_ref().is_some_and(|event| {
                        event.applied
                            && event.event_id == *owned_event_id
                            && event.actor_id.as_deref() == Some(intent.actor_id.as_str())
                    }),
            ),
            _ => bail!("invalid label feedback operation"),
        };
        if !current_basis {
            // A satisfied state is not evidence that this intent caused it.
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        let response = self.query::<graphql::MutationData>(
            query,
            &[
                ("id", &intent.issue.issue_id),
                ("label", &label.id),
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
        let Some(node) = payload.labelable else {
            return Ok(IssueFeedbackWriteResult::Unconfirmed);
        };
        if !Self::matches_issue(&node, &intent.issue) {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        Ok(IssueFeedbackWriteResult::Acknowledged { comment_id: None })
    }

    pub(super) fn reconcile_label(
        &self,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackReadResult> {
        let label = intent
            .operation
            .label()
            .ok_or_else(|| anyhow!("invalid label feedback operation"))?;
        let observed = self.label_observation(&intent.issue, &label.name)?;
        if observed.snapshot.label.as_ref() != Some(label) || !observed.snapshot.complete {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        }
        let (previous, applied) = match &intent.operation {
            IssueFeedbackOperation::AddLabel {
                previous_event_id, ..
            } => (previous_event_id.as_deref(), true),
            IssueFeedbackOperation::RemoveLabel { owned_event_id, .. } => {
                (Some(owned_event_id.as_str()), false)
            }
            _ => bail!("invalid label feedback operation"),
        };
        let start = if let Some(previous) = previous {
            let Some(index) = observed
                .events
                .iter()
                .position(|event| event.event_id == previous)
            else {
                return Ok(IssueFeedbackReadResult::NeedsConfirmation);
            };
            let prior = &observed.events[index];
            if prior.applied == applied
                || (!applied && prior.actor_id.as_deref() != Some(intent.actor_id.as_str()))
            {
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
        if event.applied != applied
            || observed.snapshot.present != applied
            || actor != &intent.actor_id
        {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        Self::confirmed(
            intent,
            IssueFeedbackEvidence::Label {
                label: label.clone(),
                event_id: event.event_id.clone(),
                applied,
            },
        )
    }
}
