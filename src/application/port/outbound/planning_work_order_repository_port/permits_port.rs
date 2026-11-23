use std::fmt;

pub use crate::domain::planning::PlanningWorkDeliveryAuthority;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::{
    PlanningWorkOrderRecord, PlanningWorkOrderRepositoryPort, validate_opaque_key,
    validate_work_order_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningWorkPermitStage {
    Dispatch,
    Push,
    PullRequest,
    Integrate,
}

impl PlanningWorkPermitStage {
    pub const ALL: [Self; 4] = [
        Self::Dispatch,
        Self::Push,
        Self::PullRequest,
        Self::Integrate,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Push => "push",
            Self::PullRequest => "pull_request",
            Self::Integrate => "integrate",
        }
    }
}

/// Host-reviewed frozen diff identity. This is absent in permits saved before diff review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningWorkDeliveryReview {
    pub base_commit_sha: String,
    pub source_commit_sha: String,
    pub gate_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<PlanningWorkDeliveryAuthority>,
}

impl PlanningWorkDeliveryReview {
    pub fn validate(&self) -> Result<()> {
        if !full_git_object_id(&self.base_commit_sha)
            || !full_git_object_id(&self.source_commit_sha)
        {
            bail!("delivery review commits must be full Git object ids");
        }
        if let Some(authority) = &self.authority {
            authority.validate().map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
}

/// Independently persisted current evidence. A grant cannot update its own comparison basis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningWorkPermitBasis {
    #[serde(default)]
    pub authorization_epoch: u64,
    pub policy_version: String,
    pub field_hash: String,
    pub gate_revision: u64,
    pub source_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_review: Option<PlanningWorkDeliveryReview>,
}

impl PlanningWorkPermitBasis {
    pub fn validate(&self) -> Result<()> {
        validate_opaque_key(&self.policy_version, "permit policy version", 512)?;
        validate_opaque_key(&self.field_hash, "permit field hash", 512)?;
        if !full_git_object_id(&self.source_sha) {
            bail!("permit source_sha must be a full Git object id");
        }
        if let Some(review) = &self.delivery_review {
            review.validate()?;
            if review.source_commit_sha != self.source_sha
                || review.gate_revision != self.gate_revision
            {
                bail!("delivery review must match the current permit source and gate revision");
            }
        }
        Ok(())
    }
}

fn full_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningWorkPermit {
    pub work_order_id: String,
    pub stage: PlanningWorkPermitStage,
    #[serde(flatten)]
    pub basis: PlanningWorkPermitBasis,
    pub requires_human_review: bool,
}

impl PlanningWorkPermit {
    pub fn validate(&self) -> Result<()> {
        validate_work_order_id(&self.work_order_id)?;
        self.basis.validate()?;
        if self.requires_auto_delivery()
            && (self.stage == PlanningWorkPermitStage::Dispatch || self.requires_human_review)
        {
            bail!("automatic delivery cannot be a dispatch or a Human-review grant");
        }
        Ok(())
    }

    pub fn requires_auto_delivery(&self) -> bool {
        matches!(
            self.basis
                .delivery_review
                .as_ref()
                .and_then(|review| review.authority.as_ref()),
            Some(PlanningWorkDeliveryAuthority::Auto { .. })
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkPermitBlockReason {
    Missing,
    Stale,
    ExecutionAttemptLimit,
}

impl PlanningWorkPermitBlockReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "permit_missing",
            Self::Stale => "permit_stale",
            Self::ExecutionAttemptLimit => "execution_attempt_limit",
        }
    }
}

impl fmt::Display for PlanningWorkPermitBlockReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkPermitDecision {
    /// Ordinary planning tasks have no work-order authority and keep their existing behavior.
    Legacy,
    Granted {
        requires_human_review: bool,
    },
    /// Explicit Auto authority still requires the current enforced evidence gate.
    GrantedAuto,
    Denied(PlanningWorkPermitBlockReason),
}

impl PlanningWorkPermitDecision {
    pub fn blocked_reason(self) -> Option<&'static str> {
        match self {
            Self::Denied(reason) => Some(reason.label()),
            Self::Legacy | Self::Granted { .. } | Self::GrantedAuto => None,
        }
    }

    pub fn requires_auto_delivery(self) -> bool {
        matches!(self, Self::GrantedAuto)
    }

    pub fn requires_human_review(self) -> bool {
        matches!(
            self,
            Self::Granted {
                requires_human_review: true
            }
        )
    }
}

pub fn evaluate_work_order_permit(
    work_order: &PlanningWorkOrderRecord,
    current_basis: Option<&PlanningWorkPermitBasis>,
    stage: PlanningWorkPermitStage,
    source_sha: Option<&str>,
) -> PlanningWorkPermitDecision {
    if work_order.validate().is_err()
        || current_basis.is_some_and(|basis| basis.validate().is_err())
    {
        return PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale);
    }
    let Some(permit) = work_order
        .permits
        .iter()
        .find(|permit| permit.stage == stage)
    else {
        return PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing);
    };
    if current_basis != Some(&permit.basis)
        || source_sha.is_some_and(|sha| sha != permit.basis.source_sha)
        || (stage != PlanningWorkPermitStage::Dispatch && source_sha.is_none())
    {
        return PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale);
    }
    if work_order.origin == "issue_automation"
        && stage != PlanningWorkPermitStage::Dispatch
        && permit.basis.delivery_review.is_none()
    {
        return PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing);
    }
    let auto = stage != PlanningWorkPermitStage::Dispatch && permit.requires_auto_delivery();
    if work_order.origin == "issue_automation"
        && stage == PlanningWorkPermitStage::Integrate
        && !permit.requires_human_review
        && !auto
    {
        return PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing);
    }
    if auto {
        PlanningWorkPermitDecision::GrantedAuto
    } else {
        PlanningWorkPermitDecision::Granted {
            requires_human_review: permit.requires_human_review,
        }
    }
}

/// Planning owns grant/revoke persistence; dispatch and delivery never import issue automation.
pub trait PlanningWorkPermitRepositoryPort:
    PlanningWorkOrderRepositoryPort + super::PlanningWorkControlRepositoryPort
{
    fn load_work_order_permit_basis(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
    ) -> Result<Option<PlanningWorkPermitBasis>>;

    /// Replaying the same evidence is idempotent. Changed evidence needs a newer gate revision.
    fn set_work_order_permit_basis(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        basis: &PlanningWorkPermitBasis,
    ) -> Result<()>;

    /// A grant must match the separately stored basis and postdate that stage's last revocation.
    fn grant_work_order_permit(
        &self,
        workspace_dir: &str,
        permit: &PlanningWorkPermit,
    ) -> Result<()>;

    /// Removes the specified grants and fences delayed replays at the current gate revision.
    /// An empty stage list is a no-op. Use Stage::ALL to revoke every stage.
    fn revoke_work_order_permits(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        stages: &[PlanningWorkPermitStage],
        reason: &str,
    ) -> Result<usize>;

    /// Delivery requires its freshly observed SHA; dispatch may preflight the durable basis.
    /// The actual slot claim repeats this check atomically with its frozen integration SHA.
    fn check_work_order_permit_for_task(
        &self,
        workspace_dir: &str,
        task_id: &str,
        stage: PlanningWorkPermitStage,
        source_sha: Option<&str>,
    ) -> Result<PlanningWorkPermitDecision>;
}

#[cfg(test)]
mod tests;
