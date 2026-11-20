use anyhow::{Result, bail};

use crate::domain::issue_creation::{
    IssueCreatedLocation, IssueCreationApproval, IssueRepositoryReference,
};
use crate::domain::issue_work::{IssueFieldHash, IssueProjectScope, IssueSharedCopy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRepositoryResolveRequest {
    pub owner: String,
    pub repository: String,
    pub project_scope: IssueProjectScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCreateRequest {
    pub repository: IssueRepositoryReference,
    pub intent_id: String,
    pub approval: IssueCreationApproval,
    pub shared_copy: IssueSharedCopy,
}

impl IssueCreateRequest {
    pub fn ensure_authorized(&self) -> Result<()> {
        if self.repository != self.approval.repository
            || self.intent_id != self.approval.intent_id
            || self.approval.shared_copy_hash
                != IssueFieldHash::of_fields(&self.shared_copy.title, &self.shared_copy.body)
        {
            bail!("the approval does not authorize this issue creation")
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IssueCreateResponse {
    pub created: IssueCreatedLocation,
    pub published_copy: IssueSharedCopy,
}

/// Creation errors may follow a successful remote write. Implementations never retry.
pub trait IssueCreatePort: Send + Sync {
    fn resolve_repository(
        &self,
        request: IssueRepositoryResolveRequest,
    ) -> Result<IssueRepositoryReference>;

    fn create_issue(&self, request: IssueCreateRequest) -> Result<IssueCreateResponse>;
}

impl std::fmt::Debug for IssueCreateResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssueCreateResponse")
            .field("created", &self.created)
            .field("published_copy", &"<unverified remote copy>")
            .finish()
    }
}
