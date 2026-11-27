use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::*;

const PROPERTY_KEY: &str = "akra.issue-feedback.v1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommentProperty {
    version: u32,
    intent_id: String,
    body_sha256: String,
    content_sha256: String,
}
struct ObservedComment {
    id: String,
    actor: String,
    editor: String,
    property: CommentProperty,
    actual_body_sha256: String,
    actual_content_sha256: String,
}

impl ObservedComment {
    fn matches(&self, actor: &str, intent_id: &str, body_hash: &str, content_hash: &str) -> bool {
        self.actor == actor
            && self.editor == actor
            && self.property.version == 1
            && self.property.intent_id == intent_id
            && self.property.body_sha256 == body_hash
            && self.property.content_sha256 == content_hash
            && self.actual_body_sha256 == body_hash
            && self.actual_content_sha256 == content_hash
    }
}

impl JiraIssueFeedbackAdapter {
    pub(super) fn write_comment(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackWriteResult> {
        let (body, body_sha256, content_sha256) = match &intent.operation {
            IssueFeedbackOperation::CreateComment {
                body,
                body_sha256,
                content_sha256,
            }
            | IssueFeedbackOperation::UpdateComment {
                body,
                body_sha256,
                content_sha256,
                ..
            } => (body, body_sha256, content_sha256),
            _ => bail!("invalid Jira comment intent"),
        };
        let suffix = format!("\n\n{}", feedback_marker(&intent.intent_id));
        let content = body
            .strip_suffix(&suffix)
            .ok_or_else(|| anyhow!("invalid Jira comment marker"))?;
        let adf = crate::adapter::outbound::jira::adf::writable_adf(content)
            .ok_or_else(|| anyhow!("feedback text has no lossless Jira representation"))?;
        let property = serde_json::to_value(CommentProperty {
            version: 1,
            intent_id: intent.intent_id.clone(),
            body_sha256: body_sha256.clone(),
            content_sha256: content_sha256.clone(),
        })?;
        let request = match &intent.operation {
            IssueFeedbackOperation::CreateComment { .. } => {
                if !self.find_comments(context, intent)?.is_empty() {
                    return Ok(IssueFeedbackWriteResult::Contradictory);
                }
                JiraIssueFeedbackWriteRequest::CreateComment {
                    issue_id: intent.issue.issue_id.clone(),
                    body: adf,
                    property,
                }
            }
            IssueFeedbackOperation::UpdateComment { owner, .. } => {
                let old = self.read_comment(context, &intent.issue, &owner.comment_id)?;
                if old.as_ref().is_none_or(|old| {
                    !old.matches(
                        &owner.actor_id,
                        &owner.latest_intent_id,
                        &owner.body_sha256,
                        &owner.content_sha256,
                    )
                }) {
                    return Ok(IssueFeedbackWriteResult::Contradictory);
                }
                JiraIssueFeedbackWriteRequest::UpdateComment {
                    issue_id: intent.issue.issue_id.clone(),
                    comment_id: owner.comment_id.clone(),
                    body: adf,
                    property,
                }
            }
            _ => unreachable!(),
        };
        self.issue_status(context, &intent.issue)?;
        let response = match self.send(context, intent, request) {
            Ok(JiraIssueFeedbackWriteResponse::Comment(response)) => response.into_value(),
            Ok(JiraIssueFeedbackWriteResponse::Rejected(reason)) => {
                return Ok(IssueFeedbackWriteResult::JiraRejected(reason));
            }
            Ok(_) => return Ok(IssueFeedbackWriteResult::Contradictory),
            Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
        };
        // The ACK pins a comment ID. Body and property proof are established by readback.
        let id = self.comment_identity(context, &intent.issue, &response)?;
        if response
            .pointer("/author/accountId")
            .and_then(Value::as_str)
            != Some(intent.actor_id.as_str())
            || response
                .pointer("/updateAuthor/accountId")
                .and_then(Value::as_str)
                != Some(intent.actor_id.as_str())
            || matches!(&intent.operation, IssueFeedbackOperation::UpdateComment { owner, .. } if owner.comment_id != id)
        {
            return Ok(IssueFeedbackWriteResult::Contradictory);
        }
        Ok(IssueFeedbackWriteResult::Acknowledged {
            comment_id: Some(id),
        })
    }

    pub(super) fn reconcile_comment(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackReadResult> {
        let (body_sha256, content_sha256, observed) = match &intent.operation {
            IssueFeedbackOperation::CreateComment {
                body_sha256,
                content_sha256,
                ..
            } => (
                body_sha256,
                content_sha256,
                self.find_comments(context, intent)?,
            ),
            IssueFeedbackOperation::UpdateComment {
                owner,
                body_sha256,
                content_sha256,
                ..
            } => (
                body_sha256,
                content_sha256,
                self.read_comment(context, &intent.issue, &owner.comment_id)?
                    .into_iter()
                    .collect(),
            ),
            _ => bail!("invalid Jira comment reconciliation"),
        };
        self.issue_status(context, &intent.issue)?;
        if observed.is_empty() {
            return Ok(IssueFeedbackReadResult::NeedsConfirmation);
        }
        if observed.len() != 1 {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        let observed = &observed[0];
        if !observed.matches(
            &intent.actor_id,
            &intent.intent_id,
            body_sha256,
            content_sha256,
        ) {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        Self::confirmed(
            intent,
            IssueFeedbackEvidence::Comment {
                comment_id: observed.id.clone(),
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
        validate_jira_feedback_id(id).map_err(anyhow::Error::msg)?;
        let value = self.read(
            context,
            JiraIssueFeedbackReadRequest::Comment {
                issue_id: issue.issue_id.clone(),
                comment_id: id.to_owned(),
            },
        )?;
        if string(&value, "id")? != id {
            bail!("Jira returned a different comment");
        }
        self.observe_comment(context, issue, &value)
    }

    fn find_comments(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
    ) -> Result<Vec<ObservedComment>> {
        let mut start = 0u64;
        let mut total = None;
        let mut seen = BTreeSet::new();
        let mut found = Vec::new();
        for _ in 0..MAX_PAGES {
            let page = self.read(
                context,
                JiraIssueFeedbackReadRequest::Comments {
                    issue_id: intent.issue.issue_id.clone(),
                    start_at: start,
                },
            )?;
            let (values, next) = accept_page(&page, "comments", start, &mut total, &mut seen)?;
            for value in values {
                if let Some(observed) = self.observe_comment(context, &intent.issue, value)?
                    && observed.property.intent_id == intent.intent_id
                {
                    found.push(observed);
                    if found.len() > 1 {
                        return Ok(found);
                    }
                }
            }
            match next {
                Some(next) => start = next,
                None => return Ok(found),
            }
        }
        bail!("Jira comment observation exceeded its bound")
    }

    fn comment_identity(
        &self,
        context: &FeedbackContext,
        issue: &IssueReference,
        value: &Value,
    ) -> Result<String> {
        let id = string(value, "id")?;
        validate_jira_feedback_id(id).map_err(anyhow::Error::msg)?;
        // The response must name the same numeric parent, not merely the requested key.
        let expected = format!(
            "{}/rest/api/3/issue/{}/comment/{id}",
            context.binding.site_url, issue.issue_id
        );
        if string(value, "self")? != expected {
            bail!("Jira comment parent changed");
        }
        Ok(id.to_owned())
    }

    fn observe_comment(
        &self,
        context: &FeedbackContext,
        issue: &IssueReference,
        value: &Value,
    ) -> Result<Option<ObservedComment>> {
        let id = self.comment_identity(context, issue, value)?;
        let property = if let Some(properties) = value.get("properties") {
            let properties = properties
                .as_array()
                .ok_or_else(|| anyhow!("invalid Jira comment properties"))?;
            if properties.len() > 64 {
                bail!("Jira comment property bound exceeded");
            }
            let mut matching = properties.iter().filter(|property| {
                property.get("key").and_then(Value::as_str) == Some(PROPERTY_KEY)
            });
            let result = matching
                .next()
                .map(|property| property.get("value").cloned().unwrap_or(Value::Null));
            if matching.next().is_some() {
                bail!("ambiguous Jira comment property");
            }
            result
        } else {
            let request = JiraIssueFeedbackReadRequest::CommentProperty {
                issue_id: issue.issue_id.clone(),
                comment_id: id.clone(),
            };
            self.credentials
                .validate_current(&context.lease.reference)?;
            let response = self.api.read(
                &context.lease,
                &request,
                &AtomicBool::new(false),
                context.deadline,
            );
            self.credentials
                .validate_current(&context.lease.reference)?;
            match response {
                Ok(document) => {
                    let property = document.into_value();
                    if property.get("key").and_then(Value::as_str) != Some(PROPERTY_KEY) {
                        bail!("Jira comment property identity changed");
                    }
                    Some(property.get("value").cloned().unwrap_or(Value::Null))
                }
                Err(crate::domain::provider_credentials::ProviderCredentialError::NotFound) => None,
                Err(error) => return Err(error.into()),
            }
        };
        let Some(property) = property else {
            return Ok(None);
        };
        let property: CommentProperty = serde_json::from_value(property)
            .map_err(|_| anyhow!("invalid Jira feedback property"))?;
        if property.version != 1 {
            bail!("unsupported Jira feedback property version");
        }
        validate_feedback_id(&property.intent_id).map_err(anyhow::Error::msg)?;
        for hash in [&property.body_sha256, &property.content_sha256] {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("invalid Jira feedback property hash");
            }
        }
        let body = value
            .get("body")
            .ok_or_else(|| anyhow!("Jira comment body is missing"))?;
        if serde_json::to_vec(body)?.len() > 256 * 1024 {
            bail!("Jira comment body bound exceeded");
        }
        let content = crate::adapter::outbound::jira::adf::adf_to_markdown(body);
        if !content.unsupported.is_empty() || content.markdown.len() > MAX_ISSUE_FEEDBACK_BODY_BYTES
        {
            bail!("Jira comment has unsupported content");
        }
        let logical = format!(
            "{}\n\n{}",
            content.markdown,
            feedback_marker(&property.intent_id)
        );
        let actor = value
            .pointer("/author/accountId")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Jira comment author is missing"))?;
        let editor = value
            .pointer("/updateAuthor/accountId")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Jira comment editor is missing"))?;
        validate_feedback_id(actor).map_err(anyhow::Error::msg)?;
        validate_feedback_id(editor).map_err(anyhow::Error::msg)?;
        Ok(Some(ObservedComment {
            id,
            actor: actor.to_owned(),
            editor: editor.to_owned(),
            property,
            actual_body_sha256: feedback_body_hash(&logical),
            actual_content_sha256: feedback_body_hash(&content.markdown),
        }))
    }
}

pub(super) fn accept_page<'a>(
    page: &'a Value,
    field: &str,
    start: u64,
    expected_total: &mut Option<u64>,
    seen: &mut BTreeSet<String>,
) -> Result<(&'a [Value], Option<u64>)> {
    let actual_start = page
        .get("startAt")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Jira page start is missing"))?;
    let total = page
        .get("total")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Jira page total is missing"))?;
    let maximum = page
        .get("maxResults")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Jira page limit is missing"))?;
    let values = page
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Jira page values are missing"))?;
    if actual_start != start
        || total > MAX_ITEMS as u64
        || expected_total.is_some_and(|old| old != total)
        || maximum == 0
        || maximum > 100
        || values.len() > maximum as usize
    {
        bail!("Jira feedback pagination changed");
    }
    *expected_total = Some(total);
    let end = start
        .checked_add(values.len() as u64)
        .ok_or_else(|| anyhow!("Jira page overflow"))?;
    if end > total || (values.is_empty() && end < total) {
        bail!("Jira feedback page is incomplete");
    }
    for value in values {
        let id = string(value, "id")?;
        validate_jira_feedback_id(id).map_err(anyhow::Error::msg)?;
        if !seen.insert(id.to_owned()) {
            bail!("duplicate Jira feedback identity");
        }
    }
    if page
        .get("isLast")
        .and_then(Value::as_bool)
        .is_some_and(|last| last != (end == total))
    {
        bail!("Jira feedback pagination is contradictory");
    }
    Ok((values.as_slice(), (end < total).then_some(end)))
}
