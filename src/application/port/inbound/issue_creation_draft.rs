//! Sensitive creation requests are deliberately not Debug or Serialize.
use serde::{Deserialize, Serialize};

use crate::application::port::inbound::issue_draft_port::{
    IssueDraftPreview, IssueDraftSpanActions, IssueDraftTemplate, IssueDraftText,
};
use crate::domain::issue_creation::{
    IssueCreatedLocation, IssueCreationApproval, IssueCreationIntentState, IssueRepositoryReference,
};
use crate::domain::issue_work::IssueSharedCopy;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationTarget {
    pub cwd: String,
    pub owner: String,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationIntent {
    pub intent_id: String,
    pub state: IssueCreationIntentState,
    pub created: Option<IssueCreatedLocation>,
    pub shared_copy: Option<IssueSharedCopy>,
    pub restore_link_required: bool,
    pub restore_link_finalized: bool,
    pub predecessor_intent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationOpened {
    pub repository: IssueRepositoryReference,
    pub intent: IssueCreationIntent,
    pub title: String,
    pub body: String,
    pub template: IssueDraftTemplate,
    pub required_sections: Vec<String>,
    pub span_actions: IssueDraftSpanActions,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationPreviewRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
    pub text: IssueDraftText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationPreview {
    pub preview: IssueDraftPreview,
    pub approval: Option<IssueCreationApproval>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationPublishRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
    pub text: IssueDraftText,
    pub approval: IssueCreationApproval,
    #[serde(default)]
    pub shared_copy_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IssueCreationPublished {
    Confirmed { intent: IssueCreationIntent },
    NeedsConfirmation { intent: IssueCreationIntent },
    Held { intent: IssueCreationIntent },
    Refused { reason: &'static str },
    Blocked { preview: IssueCreationPreview },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationIntentRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationConfirmRequest {
    pub target: IssueCreationTarget,
    pub intent_id: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationRestartRequest {
    pub target: IssueCreationTarget,
    pub previous_intent_id: String,
    /// An unresolved predecessor may already exist on GitHub. Never infer this consent.
    #[serde(default)]
    pub acknowledge_uncertain: bool,
}
