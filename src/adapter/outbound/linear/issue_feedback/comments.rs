use std::collections::BTreeSet;

use super::*;

/// Linear has no comment property store. Ownership is the author, the issue and the exact
/// Markdown body including the intent marker. A body that Linear stored differently is never
/// adopted; it stays for a person to confirm.
struct ObservedComment {
    id: String,
    actor: String,
    body_sha256: String,
}

impl ObservedComment {
    fn matches(&self, actor: &str, body_hash: &str) -> bool {
        self.actor == actor && self.body_sha256 == body_hash
    }
}

impl LinearIssueFeedbackAdapter {
    pub(super) fn write_comment(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackWriteResult> {
        let body = match &intent.operation {
            IssueFeedbackOperation::CreateComment { body, .. }
            | IssueFeedbackOperation::UpdateComment { body, .. } => body,
            _ => bail!("invalid Linear comment intent"),
        };
        let request = match &intent.operation {
            IssueFeedbackOperation::CreateComment { .. } => {
                if !self.find_comments(context, intent)?.is_empty() {
                    return Ok(IssueFeedbackWriteResult::Contradictory);
                }
                LinearIssueWriteRequest::CreateComment {
                    issue_id: intent.issue.issue_id.clone(),
                    body: body.clone(),
                }
            }
            IssueFeedbackOperation::UpdateComment { owner, .. } => {
                let old = self.read_comment(context, &intent.issue, &owner.comment_id)?;
                if old
                    .as_ref()
                    .is_none_or(|old| !old.matches(&owner.actor_id, &owner.body_sha256))
                {
                    return Ok(IssueFeedbackWriteResult::Contradictory);
                }
                LinearIssueWriteRequest::UpdateComment {
                    comment_id: owner.comment_id.clone(),
                    body: body.clone(),
                }
            }
            _ => unreachable!(),
        };
        self.issue_state(context, &intent.issue)?;
        let field = match request {
            LinearIssueWriteRequest::CreateComment { .. } => "commentCreate",
            _ => "commentUpdate",
        };
        let response = match self.send(context, intent, request) {
            Ok(response) => response,
            Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
        };
        let payload = &response[field];
        if payload.get("success").and_then(Value::as_bool) != Some(true) {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        // The ACK pins a comment ID. Body proof is established by readback.
        let comment = &payload["comment"];
        let Ok(id) = string(comment, "id") else {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        };
        if validate_linear_feedback_id(id).is_err()
            || comment.pointer("/user/id").and_then(Value::as_str) != Some(intent.actor_id.as_str())
            || comment.pointer("/issue/id").and_then(Value::as_str)
                != Some(intent.issue.issue_id.as_str())
            || matches!(&intent.operation, IssueFeedbackOperation::UpdateComment { owner, .. } if owner.comment_id != id)
        {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        Ok(IssueFeedbackWriteResult::Acknowledged {
            comment_id: Some(id.to_owned()),
        })
    }

    pub(super) fn reconcile_comment(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackReadResult> {
        let (body_sha256, observed) = match &intent.operation {
            IssueFeedbackOperation::CreateComment { body_sha256, .. } => {
                (body_sha256, self.find_comments(context, intent)?)
            }
            IssueFeedbackOperation::UpdateComment {
                owner, body_sha256, ..
            } => (
                body_sha256,
                self.read_comment(context, &intent.issue, &owner.comment_id)?
                    .into_iter()
                    .collect(),
            ),
            _ => bail!("invalid Linear comment reconciliation"),
        };
        self.issue_state(context, &intent.issue)?;
        if observed.is_empty() {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        }
        if observed.len() != 1 || !observed[0].matches(&intent.actor_id, body_sha256) {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        Self::confirmed(
            intent,
            IssueFeedbackEvidence::Comment {
                comment_id: observed[0].id.clone(),
                body_sha256: body_sha256.clone(),
            },
        )
    }

    fn read_comment(
        &self,
        context: &FeedbackContext,
        issue: &IssueReference,
        id: &str,
    ) -> Result<Option<ObservedComment>> {
        validate_linear_feedback_id(id).map_err(anyhow::Error::msg)?;
        let value = self.read(
            context,
            LinearIssueReadRequest::Comment {
                comment_id: id.to_owned(),
            },
        )?;
        let comment = &value["comment"];
        if string(comment, "id")? != id {
            bail!("Linear returned a different comment");
        }
        observe_comment(issue, comment)
    }

    /// Every comment page is read; the intent marker selects candidates, never ownership.
    fn find_comments(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<Vec<ObservedComment>> {
        let marker = feedback_marker(&intent.intent_id);
        let mut after = None;
        let mut seen = BTreeSet::new();
        let mut found = Vec::new();
        for _ in 0..MAX_PAGES {
            let value = self.read(
                context,
                LinearIssueReadRequest::Comments {
                    issue_id: intent.issue.issue_id.clone(),
                    after: after.clone(),
                },
            )?;
            if value.pointer("/issue/id").and_then(Value::as_str)
                != Some(intent.issue.issue_id.as_str())
            {
                bail!("Linear returned comments of a different issue");
            }
            let (nodes, next) = page(&value["issue"]["comments"], after.as_deref())?;
            for node in nodes {
                if !seen.insert(string(node, "id")?.to_owned()) {
                    bail!("duplicate Linear comment identity");
                }
                let body = string_body(node)?;
                if body.contains(&marker)
                    && let Some(observed) = observe_comment(&intent.issue, node)?
                {
                    found.push(observed);
                    if found.len() > 1 {
                        return Ok(found);
                    }
                }
            }
            match next {
                Some(next) => after = Some(next),
                None => return Ok(found),
            }
        }
        bail!("Linear comment observation exceeded its bound")
    }
}

fn string_body(comment: &Value) -> Result<&str> {
    comment
        .get("body")
        .and_then(Value::as_str)
        .filter(|body| body.len() <= 256 * 1024)
        .ok_or_else(|| anyhow!("Linear comment body is missing"))
}

fn observe_comment(issue: &IssueReference, comment: &Value) -> Result<Option<ObservedComment>> {
    let id = string(comment, "id")?;
    validate_linear_feedback_id(id).map_err(anyhow::Error::msg)?;
    if comment.pointer("/issue/id").and_then(Value::as_str) != Some(issue.issue_id.as_str()) {
        bail!("Linear comment parent changed");
    }
    // A deleted user or an integration comment has no owning user; it is never ours.
    let Some(actor) = comment.pointer("/user/id").and_then(Value::as_str) else {
        return Ok(None);
    };
    validate_feedback_id(actor).map_err(anyhow::Error::msg)?;
    Ok(Some(ObservedComment {
        id: id.to_owned(),
        actor: actor.to_owned(),
        body_sha256: feedback_body_hash(string_body(comment)?),
    }))
}
