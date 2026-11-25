//! 상태 댓글·진행 라벨·승인된 닫기를 쓰는 GitHub 경계이다 (#2552, #2553).
//! 쓰기 재시도는 없다. ACK와 별개로 읽기 확인이 성공해야 영수증을 발급한다.
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use serde::de::DeserializeOwned;

use super::issue_snapshot::{GhApiRunner, ensure_node_id, graphql_arguments, run_gh_api};
use super::resource_version::GITHUB_HOSTNAME;
use crate::application::port::outbound::issue_feedback_port::{
    IssueFeedbackClosureSnapshot, IssueFeedbackLabelSnapshot, IssueFeedbackPort,
    IssueFeedbackReadResult, IssueFeedbackWriteResult,
};
use crate::domain::issue_automation::feedback::{
    IssueFeedbackEvidence, IssueFeedbackIntent, IssueFeedbackOperation, IssueFeedbackReceipt,
    validate_feedback_issue,
};
use crate::domain::issue_work::IssueReference;

mod closure;
mod commands;
mod comments;
mod graphql;
mod labels;
mod pagination;

pub struct GithubIssueFeedbackAdapter {
    run_gh_api: GhApiRunner,
}

impl GithubIssueFeedbackAdapter {
    pub fn new(working_directory: PathBuf) -> Self {
        Self::with_runner(Box::new(move |arguments| {
            run_gh_api(&working_directory, arguments)
        }))
    }

    fn with_runner(run_gh_api: GhApiRunner) -> Self {
        Self { run_gh_api }
    }

    fn query<T: DeserializeOwned>(&self, query: &str, fields: &[(&str, &str)]) -> Result<T> {
        let arguments = graphql_arguments(query, fields, None);
        let output = (self.run_gh_api)(&arguments)
            .map_err(|_| anyhow!("GitHub did not confirm the feedback request"))?;
        let envelope: graphql::Envelope<T> = serde_json::from_slice(&output)
            .map_err(|_| anyhow!("GitHub returned an invalid feedback response"))?;
        // Partial GraphQL data is not a complete identity or uniqueness proof.
        if envelope.errors.is_some_and(|errors| !errors.is_empty()) {
            bail!("GitHub did not confirm the feedback request")
        }
        envelope
            .data
            .ok_or_else(|| anyhow!("GitHub did not return feedback data"))
    }

    fn validate_issue(issue: &IssueReference) -> Result<()> {
        validate_feedback_issue(issue).map_err(|_| anyhow!("invalid feedback issue"))?;
        if issue.server_id != GITHUB_HOSTNAME {
            bail!("the feedback issue is not on github.com")
        }
        ensure_node_id(&issue.repository_id)?;
        ensure_node_id(&issue.issue_id)
    }

    fn validate_intent(intent: &IssueFeedbackIntent) -> Result<()> {
        intent
            .validate()
            .map_err(|_| anyhow!("invalid feedback intent"))?;
        Self::validate_issue(&intent.issue)?;
        ensure_node_id(&intent.actor_id)?;
        match &intent.operation {
            IssueFeedbackOperation::UpdateComment { owner, .. } => {
                ensure_node_id(&owner.comment_id)?;
                ensure_node_id(&owner.actor_id)?;
            }
            IssueFeedbackOperation::AddLabel {
                label,
                previous_event_id,
            } => {
                ensure_node_id(&label.id)?;
                if let Some(id) = previous_event_id {
                    ensure_node_id(id)?;
                }
            }
            IssueFeedbackOperation::RemoveLabel {
                label,
                owned_event_id,
            } => {
                ensure_node_id(&label.id)?;
                ensure_node_id(owned_event_id)?;
            }
            IssueFeedbackOperation::CloseIssue {
                previous_event_id, ..
            } => {
                if let Some(id) = previous_event_id {
                    ensure_node_id(id)?;
                }
            }
            IssueFeedbackOperation::CreateComment { .. } => {}
            // 보드 필드는 승인을 검사하는 보드 adapter 만 쓴다 (#2565).
            IssueFeedbackOperation::SetBoardField { .. } => {
                bail!("board field feedback uses the board field port")
            }
        }
        Ok(())
    }

    fn matches_issue(node: &graphql::IssueNode, issue: &IssueReference) -> bool {
        node.typename == "Issue"
            && node.id.as_deref() == Some(issue.issue_id.as_str())
            && node
                .repository
                .as_ref()
                .is_some_and(|repository| repository.id == issue.repository_id)
    }

    fn require_issue(node: &graphql::IssueNode, issue: &IssueReference) -> Result<()> {
        if !Self::matches_issue(node, issue) {
            bail!("GitHub returned a different feedback issue")
        }
        Ok(())
    }

    fn actor(actor: Option<&graphql::Actor>) -> Option<&str> {
        let actor = actor?;
        if !matches!(actor.typename.as_str(), "User" | "Bot") {
            return None;
        }
        let id = actor.id.as_deref()?;
        ensure_node_id(id).ok()?;
        Some(id)
    }

    fn confirmed(
        intent: &IssueFeedbackIntent,
        evidence: IssueFeedbackEvidence,
    ) -> Result<IssueFeedbackReadResult> {
        let receipt = IssueFeedbackReceipt {
            issue: intent.issue.clone(),
            intent_id: intent.intent_id.clone(),
            actor_id: intent.actor_id.clone(),
            evidence,
        };
        receipt
            .validate_for(intent)
            .map_err(|_| anyhow!("invalid feedback receipt"))?;
        Ok(IssueFeedbackReadResult::Confirmed(Box::new(receipt)))
    }
}

impl IssueFeedbackPort for GithubIssueFeedbackAdapter {
    fn actor_id(&self, issue: &IssueReference) -> Result<String> {
        Self::validate_issue(issue)?;
        let data: graphql::ViewerData = self.query(graphql::VIEWER, &[("id", &issue.issue_id)])?;
        let node = data
            .node
            .ok_or_else(|| anyhow!("GitHub did not return the feedback issue"))?;
        Self::require_issue(&node, issue)?;
        Self::actor(data.viewer.as_ref())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("GitHub did not identify the feedback account"))
    }

    fn read_label(&self, issue: &IssueReference, name: &str) -> Result<IssueFeedbackLabelSnapshot> {
        Self::validate_issue(issue)?;
        self.label_observation(issue, name)
            .map(|observation| observation.snapshot)
    }

    fn read_closure(&self, issue: &IssueReference) -> Result<IssueFeedbackClosureSnapshot> {
        Self::validate_issue(issue)?;
        self.closure_observation(issue)
            .map(|observation| observation.snapshot)
    }

    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        Self::validate_intent(intent)?;
        // The journal owns the single Sent claim; this fresh read does not resend anything.
        match self.actor_id(&intent.issue) {
            Ok(actor) if actor == intent.actor_id => {}
            Ok(_) => return Ok(IssueFeedbackWriteResult::Contradictory),
            Err(_) => return Ok(IssueFeedbackWriteResult::Unconfirmed),
        }
        match &intent.operation {
            IssueFeedbackOperation::CreateComment { .. }
            | IssueFeedbackOperation::UpdateComment { .. } => self.write_comment(intent),
            IssueFeedbackOperation::AddLabel { .. }
            | IssueFeedbackOperation::RemoveLabel { .. } => self.write_label(intent),
            IssueFeedbackOperation::CloseIssue { .. } => self.write_closure(intent),
            IssueFeedbackOperation::SetBoardField { .. } => {
                bail!("board field feedback uses the board field port")
            }
        }
    }

    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        Self::validate_intent(intent)?;
        let result = match &intent.operation {
            IssueFeedbackOperation::CreateComment { .. }
            | IssueFeedbackOperation::UpdateComment { .. } => self.reconcile_comment(intent),
            IssueFeedbackOperation::AddLabel { .. }
            | IssueFeedbackOperation::RemoveLabel { .. } => self.reconcile_label(intent),
            IssueFeedbackOperation::CloseIssue { .. } => self.reconcile_closure(intent),
            IssueFeedbackOperation::SetBoardField { .. } => {
                Err(anyhow!("board field feedback uses the board field port"))
            }
        };
        // Read failures never become absence proofs or permission to write again.
        Ok(result.unwrap_or(IssueFeedbackReadResult::NeedsConfirmation))
    }
}

#[cfg(test)]
mod tests;
