use anyhow::{Result, anyhow, bail};

use super::GithubIssueFeedbackAdapter;
use super::graphql;
use super::pagination::Pages;
use crate::adapter::outbound::github::issue_snapshot::ensure_node_id;
use crate::application::port::outbound::issue_feedback_port::{
    IssueFeedbackReadResult, IssueFeedbackWriteResult,
};
use crate::domain::issue_automation::feedback::{
    IssueFeedbackEvidence, IssueFeedbackIntent, IssueFeedbackOperation, feedback_body_hash,
    feedback_marker,
};
use crate::domain::issue_work::IssueReference;

impl GithubIssueFeedbackAdapter {
    pub(super) fn write_comment(
        &self,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackWriteResult> {
        let (query, target, body, hash, owner) = match &intent.operation {
            IssueFeedbackOperation::CreateComment {
                body, body_sha256, ..
            } => (
                graphql::CREATE_COMMENT,
                intent.issue.issue_id.as_str(),
                body,
                body_sha256,
                None,
            ),
            IssueFeedbackOperation::UpdateComment {
                owner,
                body,
                body_sha256,
                ..
            } => (
                graphql::UPDATE_COMMENT,
                owner.comment_id.as_str(),
                body,
                body_sha256,
                Some(owner),
            ),
            _ => bail!("invalid comment feedback operation"),
        };
        if let Some(owner) = owner {
            let current = match self.read_comment(&owner.comment_id) {
                Ok(Some(comment)) => comment,
                Ok(None) => return Ok(IssueFeedbackWriteResult::Contradictory),
                Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
            };
            match Self::comment_matches(
                &current,
                &intent.issue,
                &owner.actor_id,
                &owner.latest_intent_id,
                &owner.body_sha256,
                Some(&owner.comment_id),
                false,
            ) {
                Ok(true) => {}
                Ok(false) => return Ok(IssueFeedbackWriteResult::Contradictory),
                Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
            }
        }
        // Preflight is not an atomic provider CAS. A concurrent remote edit can still race here.
        let response = self.query::<graphql::MutationData>(
            &graphql::with_comment_fragment(query),
            &[
                ("id", target),
                ("body", body),
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
        let comment = if owner.is_some() {
            payload.issue_comment
        } else {
            payload.comment_edge.and_then(|edge| edge.node)
        };
        let Some(comment) = comment else {
            return Ok(IssueFeedbackWriteResult::Unconfirmed);
        };
        match Self::comment_matches(
            &comment,
            &intent.issue,
            &intent.actor_id,
            &intent.intent_id,
            hash,
            owner.map(|owner| owner.comment_id.as_str()),
            owner.is_some(),
        ) {
            Ok(true) => Ok(IssueFeedbackWriteResult::Acknowledged {
                comment_id: Some(comment.id),
            }),
            Ok(false) => Ok(IssueFeedbackWriteResult::Contradictory),
            Err(_) => Ok(IssueFeedbackWriteResult::Unconfirmed),
        }
    }

    pub(super) fn reconcile_comment(
        &self,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackReadResult> {
        let (comment, hash, required_id, edited) = match &intent.operation {
            IssueFeedbackOperation::CreateComment { body_sha256, .. } => {
                // A marker alone is not ownership. Complete pagination comes before candidate selection.
                let comments = self.read_comments(&intent.issue)?;
                let marker = feedback_marker(&intent.intent_id);
                let mut candidates = Vec::new();
                for comment in comments {
                    Self::require_issue(&comment.issue, &intent.issue)?;
                    if comment.repository.id != intent.issue.repository_id
                        || comment.pull_request.is_some()
                    {
                        bail!("GitHub returned a comment from a different feedback issue")
                    }
                    if !comment.body.ends_with(&marker) {
                        continue;
                    }
                    let actor = Self::actor(comment.author.as_ref()).ok_or_else(|| {
                        anyhow!("GitHub did not identify the feedback comment author")
                    })?;
                    if actor == intent.actor_id {
                        candidates.push(comment);
                    }
                }
                if candidates.len() > 1 {
                    return Ok(IssueFeedbackReadResult::Contradictory);
                }
                let Some(comment) = candidates.pop() else {
                    return Ok(IssueFeedbackReadResult::NeedsConfirmation);
                };
                (comment, body_sha256, None, false)
            }
            IssueFeedbackOperation::UpdateComment {
                owner, body_sha256, ..
            } => {
                let Some(comment) = self.read_comment(&owner.comment_id)? else {
                    return Ok(IssueFeedbackReadResult::NeedsConfirmation);
                };
                (comment, body_sha256, Some(owner.comment_id.as_str()), true)
            }
            _ => bail!("invalid comment feedback operation"),
        };
        if !Self::comment_matches(
            &comment,
            &intent.issue,
            &intent.actor_id,
            &intent.intent_id,
            hash,
            required_id,
            edited,
        )? {
            return Ok(IssueFeedbackReadResult::Contradictory);
        }
        Self::confirmed(
            intent,
            IssueFeedbackEvidence::Comment {
                comment_id: comment.id,
                body_sha256: hash.clone(),
            },
        )
    }

    fn read_comment(&self, comment_id: &str) -> Result<Option<graphql::Comment>> {
        ensure_node_id(comment_id)?;
        let response: graphql::NodeData<graphql::Comment> = self.query(
            &graphql::with_comment_fragment(graphql::COMMENT),
            &[("id", comment_id)],
        )?;
        Ok(response.node)
    }

    fn read_comments(&self, issue: &IssueReference) -> Result<Vec<graphql::Comment>> {
        let mut pages = Pages::default();
        let mut after: Option<String> = None;
        let mut comments = Vec::new();
        let query = graphql::with_comment_fragment(graphql::COMMENTS);
        loop {
            let mut fields = vec![("id", issue.issue_id.as_str())];
            if let Some(cursor) = &after {
                fields.push(("after", cursor));
            }
            let response: graphql::NodeData<graphql::IssueNode> = self.query(&query, &fields)?;
            let node = response
                .node
                .ok_or_else(|| anyhow!("GitHub did not return feedback comments"))?;
            Self::require_issue(&node, issue)?;
            let connection = node
                .comments
                .ok_or_else(|| anyhow!("GitHub did not return feedback comments"))?;
            let (nodes, next) = pages.accept(connection, |comment| comment.id.as_str())?;
            comments.extend(nodes);
            let Some(next) = next else {
                return Ok(comments);
            };
            after = Some(next);
        }
    }

    fn comment_matches(
        comment: &graphql::Comment,
        issue: &IssueReference,
        actor_id: &str,
        intent_id: &str,
        body_sha256: &str,
        comment_id: Option<&str>,
        require_editor: bool,
    ) -> Result<bool> {
        ensure_node_id(&comment.id)?;
        if comment.typename != "IssueComment"
            || !Self::matches_issue(&comment.issue, issue)
            || comment.repository.id != issue.repository_id
            || comment.pull_request.is_some()
            || comment_id.is_some_and(|id| id != comment.id)
        {
            return Ok(false);
        }
        let author = Self::actor(comment.author.as_ref())
            .ok_or_else(|| anyhow!("GitHub did not identify the feedback comment author"))?;
        if author != actor_id {
            return Ok(false);
        }
        match comment.editor.as_ref() {
            Some(editor) => {
                let editor = Self::actor(Some(editor)).ok_or_else(|| {
                    anyhow!("GitHub did not identify the feedback comment editor")
                })?;
                if editor != actor_id {
                    return Ok(false);
                }
            }
            None if require_editor || comment.last_edited_at.is_some() => {
                bail!("GitHub did not identify the feedback comment editor")
            }
            None => {}
        }
        if require_editor && comment.last_edited_at.is_none() {
            bail!("GitHub did not confirm the feedback comment edit")
        }
        Ok(feedback_body_hash(&comment.body) == body_sha256
            && comment.body.ends_with(&feedback_marker(intent_id)))
    }
}
