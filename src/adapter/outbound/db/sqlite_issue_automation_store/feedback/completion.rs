use anyhow::{Result, anyhow, bail};
use rusqlite::Connection;

use super::super::{
    SqliteIssueAutomationStoreAdapter, next_revision, transaction, validate_expected_revision,
};
use super::{intents, outbox, state, validate_issue};
use crate::application::port::outbound::issue_feedback_store_port::IssueFeedbackCompletion;
use crate::domain::issue_automation::feedback::{
    IssueFeedbackCommentOwner, IssueFeedbackEvidence, IssueFeedbackIntentRecord,
    IssueFeedbackIntentState, IssueFeedbackOperation, IssueFeedbackReceipt, IssueFeedbackState,
    validate_feedback_id,
};
use crate::domain::issue_work::IssueReference;

fn next_record(
    previous: &IssueFeedbackIntentRecord,
    completion: &IssueFeedbackCompletion,
) -> Result<IssueFeedbackIntentRecord> {
    let mut next = previous.clone();
    match completion {
        IssueFeedbackCompletion::CloseAcknowledged => {
            if !matches!(
                &previous.intent.operation,
                IssueFeedbackOperation::CloseIssue { .. }
            ) || !matches!(
                previous.state,
                IssueFeedbackIntentState::Sent | IssueFeedbackIntentState::NeedsConfirmation
            ) || previous.contradictory_response
            {
                bail!("only an uncontradicted sent close can acknowledge its response");
            }
            next.state = IssueFeedbackIntentState::NeedsConfirmation;
            next.closure_acknowledged = true;
        }
        IssueFeedbackCompletion::Confirmed(receipt) => {
            let receipt = receipt.as_ref();
            receipt
                .validate_for(&previous.intent)
                .map_err(|reason| anyhow!(reason))?;
            if previous.state == IssueFeedbackIntentState::Confirmed {
                if previous.receipt.as_ref() != Some(receipt) {
                    bail!("a confirmed feedback receipt cannot change");
                }
                return Ok(next);
            }
            if !matches!(
                previous.state,
                IssueFeedbackIntentState::Sent | IssueFeedbackIntentState::NeedsConfirmation
            ) || previous.contradictory_response
            {
                bail!("this feedback intent cannot be confirmed by readback");
            }
            if let Some(acknowledged) = &previous.acknowledged_comment_id
                && !matches!(&receipt.evidence, IssueFeedbackEvidence::Comment { comment_id, .. }
                    if comment_id == acknowledged)
            {
                bail!("the feedback receipt differs from its acknowledged comment");
            }
            next.state = IssueFeedbackIntentState::Confirmed;
            next.receipt = Some(receipt.clone());
        }
        IssueFeedbackCompletion::NeedsConfirmation {
            contradictory,
            acknowledged_comment_id,
        } => {
            if !matches!(
                previous.state,
                IssueFeedbackIntentState::Sent | IssueFeedbackIntentState::NeedsConfirmation
            ) {
                bail!("only a sent feedback intent may need confirmation");
            }
            if let Some(id) = acknowledged_comment_id {
                validate_feedback_id(id).map_err(|reason| anyhow!(reason))?;
                if !previous.intent.operation.is_comment()
                    || previous
                        .acknowledged_comment_id
                        .as_ref()
                        .is_some_and(|known| known != id)
                    || matches!(&previous.intent.operation, IssueFeedbackOperation::UpdateComment { owner, .. }
                        if owner.comment_id != *id)
                {
                    bail!("the acknowledged feedback comment identity cannot change");
                }
                next.acknowledged_comment_id = Some(id.clone());
            }
            next.state = IssueFeedbackIntentState::NeedsConfirmation;
            next.contradictory_response |= *contradictory;
        }
        IssueFeedbackCompletion::Abandoned => {
            if previous.state == IssueFeedbackIntentState::Abandoned {
                return Ok(next);
            }
            if previous.state != IssueFeedbackIntentState::Recorded {
                bail!("a sent feedback intent cannot be abandoned");
            }
            next.state = IssueFeedbackIntentState::Abandoned;
        }
    }
    next.validate().map_err(|reason| anyhow!(reason))?;
    Ok(next)
}

fn apply_receipt(
    state: &mut IssueFeedbackState,
    record: &IssueFeedbackIntentRecord,
    receipt: &IssueFeedbackReceipt,
) -> Result<()> {
    match (&record.intent.operation, &receipt.evidence) {
        (
            IssueFeedbackOperation::CreateComment { content_sha256, .. },
            IssueFeedbackEvidence::Comment {
                comment_id,
                body_sha256,
            },
        ) => {
            if state.comment_owner.is_some() {
                bail!("a feedback issue already owns a status comment");
            }
            state.comment_owner = Some(comment_owner(
                record,
                comment_id,
                body_sha256,
                content_sha256,
            ));
        }
        (
            IssueFeedbackOperation::UpdateComment {
                owner,
                content_sha256,
                ..
            },
            IssueFeedbackEvidence::Comment {
                comment_id,
                body_sha256,
            },
        ) => {
            if state.comment_owner.as_ref() != Some(owner.as_ref()) {
                bail!("the feedback comment ownership changed before confirmation");
            }
            state.comment_owner = Some(comment_owner(
                record,
                comment_id,
                body_sha256,
                content_sha256,
            ));
        }
        (
            IssueFeedbackOperation::AddLabel { .. },
            IssueFeedbackEvidence::Label { applied: true, .. },
        ) => {
            if state.label_proof.is_some() {
                bail!("the feedback issue already owns a label effect");
            }
            state.label_proof = Some(receipt.clone());
            state.label_removal_confirmed = false;
        }
        (
            IssueFeedbackOperation::RemoveLabel {
                label,
                owned_event_id,
            },
            IssueFeedbackEvidence::Label { applied: false, .. },
        ) => {
            if state.label_proof.as_ref().is_none_or(|proof| {
                proof.issue != record.intent.issue
                    || proof.actor_id != record.intent.actor_id
                    || !matches!(&proof.evidence, IssueFeedbackEvidence::Label {
                        label: observed, event_id, applied: true
                    } if observed == label && event_id == owned_event_id)
            }) {
                bail!("the feedback label ownership changed before confirmation");
            }
            state.label_removal_confirmed = true;
        }
        (IssueFeedbackOperation::CloseIssue { .. }, IssueFeedbackEvidence::Closed { .. }) => {
            if state.close_proof.is_some() {
                bail!("the issue already has a confirmed close intent");
            }
            state.close_proof = Some(receipt.clone());
        }
        _ => bail!("the feedback completion does not match its operation"),
    }
    Ok(())
}

fn comment_owner(
    record: &IssueFeedbackIntentRecord,
    comment_id: &str,
    body_sha256: &str,
    content_sha256: &str,
) -> IssueFeedbackCommentOwner {
    IssueFeedbackCommentOwner {
        issue: record.intent.issue.clone(),
        comment_id: comment_id.to_owned(),
        actor_id: record.intent.actor_id.clone(),
        latest_intent_id: record.intent.intent_id.clone(),
        body_sha256: body_sha256.to_owned(),
        content_sha256: content_sha256.to_owned(),
    }
}

pub(super) fn complete(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
    intent_id: &str,
    expected_revision: u64,
    completion: &IssueFeedbackCompletion,
) -> Result<Option<IssueFeedbackIntentRecord>> {
    validate_issue(issue)?;
    validate_expected_revision(expected_revision)?;
    adapter.with_connection(&issue.project_scope.workspace_dir, |connection, guard| {
        transaction(connection, guard, |tx| {
            complete_in_transaction(tx, issue, intent_id, expected_revision, completion)
        })
    })
}

fn complete_in_transaction(
    connection: &Connection,
    issue: &IssueReference,
    intent_id: &str,
    expected_revision: u64,
    completion: &IssueFeedbackCompletion,
) -> Result<Option<IssueFeedbackIntentRecord>> {
    let Some(previous) = intents::read(connection, intent_id)? else {
        return Ok(None);
    };
    if previous.intent.issue != *issue {
        bail!("the feedback completion belongs to another owner");
    }
    let mut current = state::read(connection, issue)?;
    let mut next = next_record(&previous, completion)?;
    if next == previous {
        return Ok(Some(previous));
    }
    if previous.revision != expected_revision {
        return Ok(None);
    }
    next.revision = next_revision(previous.revision)?;
    let receipt = match completion {
        IssueFeedbackCompletion::Confirmed(receipt) => {
            let receipt = receipt.as_ref();
            apply_receipt(&mut current, &next, receipt)?;
            Some(receipt)
        }
        _ => None,
    };
    intents::update(connection, &previous, &next)?;
    state::bump(connection, &mut current)?;
    if let Some(receipt) = receipt {
        outbox::append(connection, &next, receipt, current.revision)?;
    }
    Ok(Some(next))
}
