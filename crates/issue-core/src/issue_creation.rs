//! A new issue has a repository and a local intent before it has an issue ID (#2436).
//! Only the masked, approved shared copy is journaled. Raw draft text never belongs here.
use serde::{Deserialize, Serialize};

use crate::issue_publish_intent::IssuePublishVaultField;
use crate::issue_work::{
    IssueFieldHash, IssueLocator, IssueProjectScope, IssueProvider, IssueReference, IssueSharedCopy,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRepositoryReference {
    pub provider: IssueProvider,
    pub server_id: String,
    pub repository_id: String,
    pub project_scope: IssueProjectScope,
}

impl IssueRepositoryReference {
    pub fn contains(&self, issue: &IssueReference) -> bool {
        self.provider == issue.provider
            && self.server_id == issue.server_id
            && self.repository_id == issue.repository_id
            && self.project_scope == issue.project_scope
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationApproval {
    pub repository: IssueRepositoryReference,
    pub intent_id: String,
    pub shared_copy_hash: IssueFieldHash,
    pub policy_version: String,
    pub template_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationApprovedCopy {
    pub approval: IssueCreationApproval,
    pub shared_copy: IssueSharedCopy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCreationIntentState {
    Prepared,
    Held,
    Recorded,
    Sent,
    NeedsConfirmation,
    Confirmed,
    Abandoned,
}

impl IssueCreationIntentState {
    pub fn may_have_been_sent(self) -> bool {
        matches!(self, Self::Sent | Self::NeedsConfirmation | Self::Confirmed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreatedLocation {
    pub issue: IssueReference,
    pub locator: IssueLocator,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreationIntentRecord {
    pub intent_id: String,
    pub repository: IssueRepositoryReference,
    pub state: IssueCreationIntentState,
    /// Immutable after the first approved request, including while Held.
    pub approved: Option<IssueCreationApprovedCopy>,
    pub pending_vault_fields: Vec<IssuePublishVaultField>,
    /// Stable candidate returned by creation, or explicitly selected by the user.
    pub created: Option<IssueCreatedLocation>,
    pub observed_hash: Option<IssueFieldHash>,
    pub write_response_mismatch: bool,
    pub restore_link_finalized: bool,
    pub shared_copy_only: bool,
    pub predecessor_intent_id: Option<String>,
}

impl IssueCreationIntentRecord {
    /// Kept separate from the random update intent namespace.
    pub fn adoption_intent_id(&self) -> String {
        format!("create:{}", self.intent_id)
    }
}
