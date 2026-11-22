//! GitHub 상태 댓글과 진행 라벨의 영속 의도이다. I/O와 실행 허가는 만들지 않는다.
pub mod projection;

use super::source::IssueSourceLabel;
use crate::domain::issue_work::{IssueProvider, IssueReference};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ISSUE_FEEDBACK_PROGRESS_LABEL: &str = "akra:running";
pub const MAX_ISSUE_FEEDBACK_BODY_BYTES: usize = 48 * 1024;

pub fn feedback_body_hash(body: &str) -> String {
    format!("{:x}", Sha256::digest(body.as_bytes()))
}
pub fn feedback_marker(intent_id: &str) -> String {
    format!("<!-- akra:issue-feedback:{intent_id} -->")
}
pub fn validate_feedback_id(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.len() > 1024
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err("invalid feedback identity");
    }
    Ok(())
}
fn validate_hash(value: &str) -> Result<(), &'static str> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid feedback hash");
    }
    Ok(())
}
pub fn validate_feedback_issue(issue: &IssueReference) -> Result<(), &'static str> {
    if issue.provider != IssueProvider::GitHub {
        return Err("unsupported feedback provider");
    }
    for value in [
        &issue.server_id,
        &issue.repository_id,
        &issue.issue_id,
        &issue.project_scope.workspace_dir,
    ] {
        validate_feedback_id(value)?;
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFeedbackDesired {
    /// 공유 정책과 실제 비밀값 탐지기를 지난, 표식 없는 결정적 상태 본문이다.
    pub comment_body: String,
    pub comment_body_sha256: String,
}
impl std::fmt::Debug for IssueFeedbackDesired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssueFeedbackDesired")
            .field("comment_body_sha256", &self.comment_body_sha256)
            .finish()
    }
}
impl IssueFeedbackDesired {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.comment_body.is_empty()
            || self.comment_body.len() > MAX_ISSUE_FEEDBACK_BODY_BYTES
            || self.comment_body.contains('\0')
            || feedback_body_hash(&self.comment_body) != self.comment_body_sha256
        {
            return Err("invalid shared feedback body");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFeedbackCommentOwner {
    pub issue: IssueReference,
    pub comment_id: String,
    pub actor_id: String,
    pub latest_intent_id: String,
    pub body_sha256: String,
    pub content_sha256: String,
}
impl IssueFeedbackCommentOwner {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_feedback_issue(&self.issue)?;
        for value in [&self.comment_id, &self.actor_id, &self.latest_intent_id] {
            validate_feedback_id(value)?;
        }
        validate_hash(&self.body_sha256)?;
        validate_hash(&self.content_sha256)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueFeedbackOperation {
    CreateComment {
        body: String,
        body_sha256: String,
        content_sha256: String,
    },
    UpdateComment {
        owner: Box<IssueFeedbackCommentOwner>,
        body: String,
        body_sha256: String,
        content_sha256: String,
    },
    AddLabel {
        label: IssueSourceLabel,
        previous_event_id: Option<String>,
    },
    RemoveLabel {
        label: IssueSourceLabel,
        owned_event_id: String,
    },
    CloseIssue {
        integration_record_id: String,
        integration_sha: String,
        decision_id: String,
        previous_event_id: Option<String>,
    },
}
impl std::fmt::Debug for IssueFeedbackOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateComment { body_sha256, .. } => {
                f.debug_tuple("CreateComment").field(body_sha256).finish()
            }
            Self::UpdateComment {
                owner, body_sha256, ..
            } => f
                .debug_tuple("UpdateComment")
                .field(&owner.comment_id)
                .field(body_sha256)
                .finish(),
            Self::AddLabel {
                label,
                previous_event_id,
            } => f
                .debug_tuple("AddLabel")
                .field(label)
                .field(previous_event_id)
                .finish(),
            Self::RemoveLabel {
                label,
                owned_event_id,
            } => f
                .debug_tuple("RemoveLabel")
                .field(label)
                .field(owned_event_id)
                .finish(),
            Self::CloseIssue {
                integration_record_id,
                integration_sha,
                decision_id,
                previous_event_id,
            } => f
                .debug_struct("CloseIssue")
                .field("integration_record_id", integration_record_id)
                .field("integration_sha", integration_sha)
                .field("decision_id", decision_id)
                .field("previous_event_id", previous_event_id)
                .finish(),
        }
    }
}
impl IssueFeedbackOperation {
    pub fn is_comment(&self) -> bool {
        matches!(
            self,
            Self::CreateComment { .. } | Self::UpdateComment { .. }
        )
    }
    pub fn label(&self) -> Option<&IssueSourceLabel> {
        match self {
            Self::AddLabel { label, .. } | Self::RemoveLabel { label, .. } => Some(label),
            _ => None,
        }
    }
    pub fn comment_content_hash(&self) -> Option<&str> {
        match self {
            Self::CreateComment { content_sha256, .. }
            | Self::UpdateComment { content_sha256, .. } => Some(content_sha256),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFeedbackIntent {
    pub intent_id: String,
    pub issue: IssueReference,
    pub actor_id: String,
    pub automation_policy_version: String,
    pub disclosure_policy_version: String,
    pub desired_revision: u64,
    pub operation: IssueFeedbackOperation,
    pub recorded_at: String,
}
impl IssueFeedbackIntent {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_feedback_issue(&self.issue)?;
        validate_feedback_id(&self.intent_id)?;
        if !self
            .intent_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err("invalid feedback intent marker");
        }
        validate_feedback_id(&self.actor_id)?;
        validate_hash(&self.automation_policy_version)?;
        validate_hash(&self.disclosure_policy_version)?;
        if self.desired_revision > i64::MAX as u64 || self.recorded_at.trim().is_empty() {
            return Err("invalid feedback intent revision or time");
        }
        match &self.operation {
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
            } => {
                validate_hash(content_sha256)?;
                let suffix = format!("\n\n{}", feedback_marker(&self.intent_id));
                let Some(content) = body.strip_suffix(suffix.as_str()) else {
                    return Err("invalid feedback comment marker");
                };
                if body.len() > MAX_ISSUE_FEEDBACK_BODY_BYTES
                    || body.contains('\0')
                    || content.is_empty()
                    || content.contains("<!-- akra:issue-feedback:")
                    || feedback_body_hash(body) != *body_sha256
                    || feedback_body_hash(content) != *content_sha256
                {
                    return Err("invalid feedback comment");
                }
                if let IssueFeedbackOperation::UpdateComment { owner, .. } = &self.operation {
                    owner.validate()?;
                    if owner.issue != self.issue || owner.actor_id != self.actor_id {
                        return Err("feedback comment ownership changed");
                    }
                }
            }
            IssueFeedbackOperation::AddLabel {
                label,
                previous_event_id,
            } => {
                validate_label(label)?;
                if let Some(id) = previous_event_id {
                    validate_feedback_id(id)?;
                }
            }
            IssueFeedbackOperation::RemoveLabel {
                label,
                owned_event_id,
            } => {
                validate_label(label)?;
                validate_feedback_id(owned_event_id)?;
            }
            IssueFeedbackOperation::CloseIssue {
                integration_record_id,
                integration_sha,
                decision_id,
                previous_event_id,
            } => {
                validate_feedback_id(integration_record_id)?;
                validate_feedback_id(decision_id)?;
                if !matches!(integration_sha.len(), 40 | 64)
                    || !integration_sha.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err("invalid integration proof for issue close");
                }
                if let Some(id) = previous_event_id {
                    validate_feedback_id(id)?;
                }
            }
        }
        Ok(())
    }
}
fn validate_label(label: &IssueSourceLabel) -> Result<(), &'static str> {
    validate_feedback_id(&label.id)?;
    if label.name != ISSUE_FEEDBACK_PROGRESS_LABEL {
        return Err("feedback may only change its progress label");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueFeedbackIntentState {
    Recorded,
    Sent,
    Confirmed,
    NeedsConfirmation,
    Abandoned,
}
impl IssueFeedbackIntentState {
    pub fn is_unresolved(self) -> bool {
        matches!(self, Self::Recorded | Self::Sent | Self::NeedsConfirmation)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFeedbackReceipt {
    pub issue: IssueReference,
    pub intent_id: String,
    pub actor_id: String,
    pub evidence: IssueFeedbackEvidence,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueFeedbackEvidence {
    Comment {
        comment_id: String,
        body_sha256: String,
    },
    Label {
        label: IssueSourceLabel,
        event_id: String,
        applied: bool,
    },
    Closed {
        event_id: String,
    },
}
impl IssueFeedbackReceipt {
    pub fn validate_for(&self, intent: &IssueFeedbackIntent) -> Result<(), &'static str> {
        intent.validate()?;
        if self.issue != intent.issue
            || self.intent_id != intent.intent_id
            || self.actor_id != intent.actor_id
        {
            return Err("feedback receipt identity changed");
        }
        match (&intent.operation, &self.evidence) {
            (
                IssueFeedbackOperation::CreateComment { body_sha256, .. },
                IssueFeedbackEvidence::Comment {
                    comment_id,
                    body_sha256: observed,
                },
            ) if body_sha256 == observed => validate_feedback_id(comment_id),
            (
                IssueFeedbackOperation::UpdateComment {
                    owner, body_sha256, ..
                },
                IssueFeedbackEvidence::Comment {
                    comment_id,
                    body_sha256: observed,
                },
            ) if owner.comment_id == *comment_id && body_sha256 == observed => Ok(()),
            (
                IssueFeedbackOperation::AddLabel {
                    label,
                    previous_event_id,
                },
                IssueFeedbackEvidence::Label {
                    label: observed,
                    event_id,
                    applied: true,
                },
            ) if label == observed && previous_event_id.as_ref() != Some(event_id) => {
                validate_feedback_id(event_id)
            }
            (
                IssueFeedbackOperation::RemoveLabel {
                    label,
                    owned_event_id,
                },
                IssueFeedbackEvidence::Label {
                    label: observed,
                    event_id,
                    applied: false,
                },
            ) if label == observed && owned_event_id != event_id => validate_feedback_id(event_id),
            (
                IssueFeedbackOperation::CloseIssue {
                    previous_event_id, ..
                },
                IssueFeedbackEvidence::Closed { event_id },
            ) if previous_event_id.as_ref() != Some(event_id) => validate_feedback_id(event_id),
            _ => Err("feedback receipt does not prove its intent"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFeedbackIntentRecord {
    pub intent: IssueFeedbackIntent,
    pub state: IssueFeedbackIntentState,
    pub revision: u64,
    pub receipt: Option<IssueFeedbackReceipt>,
    /// 확인된 create ACK ID는 이후 읽기로 다른 댓글에 바인딩하지 않는다.
    #[serde(default)]
    pub acknowledged_comment_id: Option<String>,
    /// 명백히 다른 ACK/소유자를 본 증거는 재조회로 지우지 않는다.
    pub contradictory_response: bool,
    /// Close readback cannot distinguish an unsent intent from a human using the same account.
    #[serde(default)]
    pub closure_acknowledged: bool,
}
impl IssueFeedbackIntentRecord {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.intent.validate()?;
        if self.revision == 0 || self.revision > i64::MAX as u64 {
            return Err("invalid feedback record revision");
        }
        if let Some(id) = &self.acknowledged_comment_id {
            validate_feedback_id(id)?;
            if !self.intent.operation.is_comment() {
                return Err("a label intent cannot own a comment ACK");
            }
            if let Some(IssueFeedbackReceipt {
                evidence: IssueFeedbackEvidence::Comment { comment_id, .. },
                ..
            }) = &self.receipt
                && comment_id != id
            {
                return Err("feedback receipt differs from its acknowledged comment");
            }
        }
        if self.closure_acknowledged
            && (!matches!(
                &self.intent.operation,
                IssueFeedbackOperation::CloseIssue { .. }
            ) || matches!(
                self.state,
                IssueFeedbackIntentState::Recorded | IssueFeedbackIntentState::Abandoned
            ))
        {
            return Err("invalid close acknowledgement");
        }
        if let Some(receipt) = &self.receipt {
            receipt.validate_for(&self.intent)?;
            if matches!(&receipt.evidence, IssueFeedbackEvidence::Closed { .. })
                && !self.closure_acknowledged
            {
                return Err("close confirmation requires a durable acknowledgement");
            }
        }
        if (self.state == IssueFeedbackIntentState::Confirmed) != self.receipt.is_some()
            || (self.contradictory_response
                && self.state != IssueFeedbackIntentState::NeedsConfirmation)
        {
            return Err("invalid feedback confirmation state");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFeedbackState {
    pub issue: IssueReference,
    pub revision: u64,
    pub desired_revision: u64,
    pub desired: Option<IssueFeedbackDesired>,
    pub comment_owner: Option<IssueFeedbackCommentOwner>,
    pub label_proof: Option<IssueFeedbackReceipt>,
    /// Remove 확인 뒤에도 이전 raw 관측의 exact Add proof를 보존한다.
    #[serde(default)]
    pub label_removal_confirmed: bool,
    /// A confirmed transition remains a historical fact after a human reopens the issue.
    #[serde(default)]
    pub close_proof: Option<IssueFeedbackReceipt>,
}
impl IssueFeedbackState {
    pub fn new(issue: IssueReference) -> Self {
        Self {
            issue,
            revision: 0,
            desired_revision: 0,
            desired: None,
            comment_owner: None,
            label_proof: None,
            label_removal_confirmed: false,
            close_proof: None,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_feedback_issue(&self.issue)?;
        if self.revision > i64::MAX as u64 || self.desired_revision > i64::MAX as u64 {
            return Err("invalid feedback state revision");
        }
        if let Some(desired) = &self.desired {
            desired.validate()?;
        }
        if let Some(owner) = &self.comment_owner {
            owner.validate()?;
            if owner.issue != self.issue {
                return Err("feedback owner scope changed");
            }
        }
        if self.label_removal_confirmed && self.label_proof.is_none() {
            return Err("a removed progress label requires its original owned proof");
        }
        if let Some(proof) = &self.label_proof
            && (proof.issue != self.issue
                || !matches!(
                    &proof.evidence,
                    IssueFeedbackEvidence::Label { applied: true, .. }
                ))
        {
            return Err("invalid owned feedback label proof");
        }
        if let Some(proof) = &self.close_proof {
            if proof.issue != self.issue {
                return Err("invalid owned close proof");
            }
            validate_feedback_id(&proof.intent_id)?;
            validate_feedback_id(&proof.actor_id)?;
            match &proof.evidence {
                IssueFeedbackEvidence::Closed { event_id } => validate_feedback_id(event_id)?,
                _ => return Err("invalid owned close proof"),
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueFeedbackAction {
    CommentCreated,
    CommentUpdated,
    LabelAdded,
    LabelRemoved,
    IssueClosed,
}
impl IssueFeedbackOperation {
    pub fn action(&self) -> IssueFeedbackAction {
        match self {
            Self::CreateComment { .. } => IssueFeedbackAction::CommentCreated,
            Self::UpdateComment { .. } => IssueFeedbackAction::CommentUpdated,
            Self::AddLabel { .. } => IssueFeedbackAction::LabelAdded,
            Self::RemoveLabel { .. } => IssueFeedbackAction::LabelRemoved,
            Self::CloseIssue { .. } => IssueFeedbackAction::IssueClosed,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFeedbackEvent {
    pub event_id: String,
    pub issue: IssueReference,
    pub intent_id: String,
    pub action: IssueFeedbackAction,
    pub receipt: IssueFeedbackReceipt,
    /// 확인과 outbox 삽입을 원자적으로 확정한 feedback state revision이다.
    pub revision: u64,
}
