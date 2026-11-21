use std::collections::BTreeSet;
use std::fmt;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::domain::planning::{PriorityQueueProjection, TaskAuthorityDocument};

use super::planning_task_repository_port::{
    PlanningTaskAuthorityMutationAudit, PlanningTaskRepositoryPort,
};

mod permits;
pub use permits::{
    PlanningWorkPermit, PlanningWorkPermitBasis, PlanningWorkPermitBlockReason,
    PlanningWorkPermitDecision, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
    evaluate_work_order_permit,
};

pub const MAX_WORK_ORDER_ID_BYTES: usize = 512;
pub const MAX_WORK_ORDER_ORIGIN_BYTES: usize = 128;
pub const MAX_WORK_ORDER_TASK_ID_BYTES: usize = 512;

/// Durable planning lineage. This identity does not depend on mutable task provenance or receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningWorkOrderRecord {
    pub work_order_id: String,
    pub origin: String,
    /// The first entry is the original task; supplemental tasks retain their attachment order.
    pub task_ids: Vec<String>,
    /// Grants are interpreted only against the separately persisted current permit basis.
    pub permits: Vec<PlanningWorkPermit>,
}

impl PlanningWorkOrderRecord {
    pub fn initial_task_id(&self) -> Result<&str> {
        self.task_ids
            .first()
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("work order has no initial task"))
    }

    pub fn ensure_origin(&self, origin: &str) -> Result<()> {
        if self.origin != origin {
            return Err(PlanningWorkOrderError::OriginConflict {
                work_order_id: self.work_order_id.clone(),
            }
            .into());
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        validate_work_order_identity(&self.work_order_id, &self.origin)?;
        self.initial_task_id()?;
        let mut seen = BTreeSet::new();
        for task_id in &self.task_ids {
            validate_work_order_task_id(task_id)?;
            if !seen.insert(task_id) {
                bail!("work order contains a duplicate task identity");
            }
        }
        let mut stages = BTreeSet::new();
        for permit in &self.permits {
            permit.validate()?;
            if permit.work_order_id != self.work_order_id {
                bail!("permit belongs to another work order");
            }
            if !stages.insert(permit.stage) {
                bail!("work order contains duplicate stage permits");
            }
        }
        Ok(())
    }
}

/// A candidate already built and validated by the ordinary planning task mutation service.
/// The concrete adapter checks the observed revision and persists the task, projection, lineage,
/// revision, and mutation audit in one transaction. There is deliberately no permits input.
#[derive(Debug, Clone, Copy)]
pub struct PlanningWorkOrderRegistration<'a> {
    pub work_order_id: &'a str,
    pub origin: &'a str,
    pub initial_task_id: &'a str,
    pub observed_planning_revision: i64,
    pub task_authority: &'a TaskAuthorityDocument,
    pub queue_projection: &'a PriorityQueueProjection,
    pub audit: PlanningTaskAuthorityMutationAudit<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningWorkOrderRegistrationResult {
    Registered {
        work_order: PlanningWorkOrderRecord,
        planning_revision: i64,
    },
    Existing {
        work_order: PlanningWorkOrderRecord,
    },
    RevisionConflict {
        observed_planning_revision: i64,
        current_planning_revision: i64,
    },
    /// A removed task identity can still belong to an immutable lineage. The service must allocate
    /// another ordinary task ID instead of replacing that membership or returning another order.
    TaskIdConflict {
        task_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkOrderLinkResult {
    pub work_order: PlanningWorkOrderRecord,
    pub linked: bool,
    pub planning_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningWorkOrderError {
    OriginConflict {
        work_order_id: String,
    },
    WorkOrderNotFound {
        work_order_id: String,
    },
    TaskNotFound {
        task_id: String,
    },
    TaskAlreadyLinked {
        task_id: String,
        work_order_id: String,
    },
}

impl fmt::Display for PlanningWorkOrderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OriginConflict { .. } => {
                formatter.write_str("work order already belongs to a different origin")
            }
            Self::WorkOrderNotFound { .. } => formatter.write_str("work order does not exist"),
            Self::TaskNotFound { .. } => formatter.write_str("planning task does not exist"),
            Self::TaskAlreadyLinked { .. } => {
                formatter.write_str("planning task already belongs to another work order")
            }
        }
    }
}

impl std::error::Error for PlanningWorkOrderError {}

/// A neutral outbound boundary owned by planning. One implementation also owns its task authority,
/// so callers cannot accidentally persist the task and its durable lineage in different stores.
pub trait PlanningWorkOrderRepositoryPort: PlanningTaskRepositoryPort {
    fn load_work_order(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>>;

    fn load_work_order_for_task(
        &self,
        workspace_dir: &str,
        task_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>>;

    /// Existing identity wins before revision, authority guard, or candidate validation.
    /// An existing ID with a different origin is a typed OriginConflict error.
    /// A successful new registration creates exactly one task and an empty permit list.
    fn register_work_order(
        &self,
        workspace_dir: &str,
        registration: PlanningWorkOrderRegistration<'_>,
    ) -> Result<PlanningWorkOrderRegistrationResult>;

    /// Existing membership is an idempotent read even when that task has since been removed.
    /// New membership requires both an existing work order and a current planning task.
    /// A task can belong to only one work order for the lifetime of the authority database.
    fn link_supplemental_task(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        task_id: &str,
    ) -> Result<PlanningWorkOrderLinkResult>;
}

pub(crate) fn validate_work_order_identity(work_order_id: &str, origin: &str) -> Result<()> {
    validate_work_order_id(work_order_id)?;
    validate_opaque_key(origin, "work order origin", MAX_WORK_ORDER_ORIGIN_BYTES)
}

pub(crate) fn validate_work_order_id(work_order_id: &str) -> Result<()> {
    validate_opaque_key(work_order_id, "work order id", MAX_WORK_ORDER_ID_BYTES)
}

pub(crate) fn validate_work_order_task_id(task_id: &str) -> Result<()> {
    validate_opaque_key(task_id, "work order task id", MAX_WORK_ORDER_TASK_ID_BYTES)?;
    if task_id != task_id.trim() {
        bail!("work order task id must not have surrounding whitespace");
    }
    Ok(())
}

fn validate_opaque_key(value: &str, label: &str, maximum_bytes: usize) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{label} must not be blank");
    }
    if value.len() > maximum_bytes {
        bail!("{label} exceeds its byte limit");
    }
    if value.chars().any(char::is_control) {
        bail!("{label} must not contain control characters");
    }
    // These are opaque keys. Preserve the caller's exact bytes rather than trim or reinterpret them.
    Ok(())
}

#[cfg(test)]
mod tests;
