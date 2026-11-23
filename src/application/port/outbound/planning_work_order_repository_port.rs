mod control_port;
pub use control_port::{
    PlanningWorkControl, PlanningWorkControlAction, PlanningWorkControlRepositoryPort,
    PlanningWorkInstruction, PlanningWorkInstructionAdmission, PlanningWorkOrderLaunchError,
};
use std::collections::BTreeSet;
use std::fmt;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::domain::parallel_mode::PrValidationRecord;
use crate::domain::planning::{PriorityQueueProjection, TaskAuthorityDocument};

use super::planning_task_repository_port::{
    PlanningTaskAuthorityMutationAudit, PlanningTaskRepositoryPort,
};

mod permits_port;
pub use permits_port::{
    PlanningWorkDeliveryReview, PlanningWorkPermit, PlanningWorkPermitBasis,
    PlanningWorkPermitBlockReason, PlanningWorkPermitDecision, PlanningWorkPermitRepositoryPort,
    PlanningWorkPermitStage, evaluate_work_order_permit,
};

pub const MAX_WORK_ORDER_ID_BYTES: usize = 512;
pub const MAX_WORK_ORDER_ORIGIN_BYTES: usize = 128;
pub const MAX_WORK_ORDER_TASK_ID_BYTES: usize = 512;

/// Immutable, publication-safe work-order reference carried into the delivery queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningWorkLinkReference {
    pub url: String,
    #[serde(default)]
    pub close_on_merge: bool,
}

impl PlanningWorkLinkReference {
    pub fn validate(&self) -> Result<()> {
        if self.url.len() > 4096
            || self
                .url
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control())
            || self.url.contains(['<', '>', '(', ')', '[', ']'])
        {
            bail!("invalid publication link reference");
        }
        let Some((host, path)) = self
            .url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/'))
        else {
            bail!("invalid publication link reference");
        };
        if !host.contains('.')
            || host.split('.').any(|part| {
                part.is_empty()
                    || part.starts_with('-')
                    || part.ends_with('-')
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
            || path.is_empty()
            || !path
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_./~".contains(&byte))
            || path.split('/').any(|part| matches!(part, "." | ".."))
        {
            bail!("invalid publication link reference");
        }
        if self.close_on_merge {
            let Some(path) = self.url.strip_prefix("https://github.com/") else {
                bail!("provider_link requires a canonical GitHub issue URL");
            };
            let parts: Vec<_> = path.split('/').collect();
            let segment = |value: &str| {
                !value.is_empty()
                    && !matches!(value, "." | "..")
                    && value
                        .bytes()
                        .all(|ch| ch.is_ascii_alphanumeric() || b"-_.".contains(&ch))
            };
            if parts.len() != 4
                || !segment(parts[0])
                || !segment(parts[1])
                || parts[2] != "issues"
                || parts[3]
                    .parse::<u64>()
                    .ok()
                    .is_none_or(|number| number == 0 || number.to_string() != parts[3])
            {
                bail!("provider_link requires a canonical GitHub issue URL");
            }
        }
        Ok(())
    }
}

/// Durable planning lineage. This identity does not depend on mutable task provenance or receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningWorkOrderRecord {
    pub work_order_id: String,
    pub origin: String,
    /// Once present, publication metadata is immutable across retries, resets, and supplements.
    /// A legacy missing reference may be recovered once without adding close authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_reference: Option<PlanningWorkLinkReference>,
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
        if let Some(reference) = &self.link_reference {
            reference.validate()?;
        }
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
    pub link_reference: Option<&'a PlanningWorkLinkReference>,
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

/// One planning snapshot fences lineage, grants, evidence and intervention ownership together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkOrderAdmissionSnapshot {
    pub work_order: PlanningWorkOrderRecord,
    pub control: PlanningWorkControl,
    pub permit_basis: Option<PlanningWorkPermitBasis>,
}

#[derive(Debug, Clone, Copy)]
pub struct PlanningWorkOrderSupplementalAdmission<'a> {
    pub admission_key: &'a str,
    pub expected: &'a PlanningWorkOrderAdmissionSnapshot,
    pub observed_planning_revision: i64,
    pub task_id: &'a str,
    pub task_authority: &'a TaskAuthorityDocument,
    pub queue_projection: &'a PriorityQueueProjection,
    pub audit: PlanningTaskAuthorityMutationAudit<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningWorkOrderSupplementalAdmissionResult {
    Admitted {
        snapshot: PlanningWorkOrderAdmissionSnapshot,
        task_id: String,
        planning_revision: i64,
        created: bool,
    },
    Existing {
        snapshot: PlanningWorkOrderAdmissionSnapshot,
        task_id: String,
    },
    RevisionConflict {
        observed_planning_revision: i64,
        current_planning_revision: i64,
    },
    WorkOrderConflict {
        current: PlanningWorkOrderAdmissionSnapshot,
    },
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

/// Immutable correlation survives distributor queue pruning and task document replacement.
/// None ownership is verified legacy; no source record at all is unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningValidationSource {
    pub validation_record_key: String,
    pub source_task_id: String,
    pub work_order_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningWorkOrderValidationLink {
    pub validation_record_key: String,
    pub work_order_id: String,
    pub source_task_id: String,
}

/// 기존 검증 종료를 변경하지 않고 후속 기록과 영수증을 함께 등록하는 종류다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningValidationRecoveryKind {
    ObserveAgain,
    QueueRecovery,
}

/// observed_*는 새 등록의 CAS 입력이며 완료 영수증의 불변 payload에는 포함하지 않는다.
#[derive(Debug, Clone, Copy)]
pub struct PlanningValidationRecoveryRequest<'a> {
    pub command_id: &'a str,
    pub work_order_id: &'a str,
    pub terminal_record_id: &'a str,
    pub kind: PlanningValidationRecoveryKind,
    pub authorization_epoch: u64,
    pub gate_revision: u64,
    pub policy_version: &'a str,
    pub original: &'a PrValidationRecord,
    pub observed_work_order: &'a PlanningWorkOrderRecord,
    pub observed_planning_revision: i64,
    pub is_current_terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningValidationRecoveryReceipt {
    pub command_id: String,
    pub terminal_record_id: String,
    pub outcome: PlanningValidationRecoveryOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanningValidationRecoveryOutcome {
    Registered { validation_record_key: String },
    Blocked { reason: String },
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
    /// 원본 CAS, 현재 제어/정책 세대, 후속 기록/계보/성공 영수증을 같은 transaction에서 검사한다.
    fn recover_work_order_validation(
        &self,
        _workspace: &str,
        _request: PlanningValidationRecoveryRequest<'_>,
    ) -> Result<PlanningValidationRecoveryReceipt> {
        bail!("work-order validation recovery is unavailable")
    }

    /// 등록된 후속 기록만 최초 원본의 보완 admission identity를 상속한다.
    fn load_work_order_validation_recovery_origin(
        &self,
        _workspace: &str,
        _validation_record_key: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    fn load_work_order_admission_snapshot(
        &self,
        _workspace: &str,
        _work_order_id: &str,
    ) -> Result<Option<PlanningWorkOrderAdmissionSnapshot>> {
        bail!("atomic work-order supplemental admission is unavailable")
    }

    fn load_work_order_supplemental_task(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _admission_key: &str,
    ) -> Result<Option<String>> {
        bail!("atomic work-order supplemental admission is unavailable")
    }

    fn admit_supplemental_work_order_task(
        &self,
        _workspace: &str,
        _request: PlanningWorkOrderSupplementalAdmission<'_>,
    ) -> Result<PlanningWorkOrderSupplementalAdmissionResult> {
        bail!("atomic work-order supplemental admission is unavailable")
    }

    /// Return an existing identity before validating replacement task input or authority.
    /// Under one transaction, validate its origin/lineage and preserve every permit and budget.
    /// A missing legacy reference may become Some only with close_on_merge=false; any existing
    /// reference must match exactly. Replaying never grants permits or bumps planning revision.
    /// Stale permits do not prevent replay: the caller may be retrying to refresh them.
    fn replay_work_order(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        origin: &str,
        link_reference: Option<&PlanningWorkLinkReference>,
    ) -> Result<Option<PlanningWorkOrderRecord>>;

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

    /// A legacy source task has no work-order correlation and returns None.
    fn link_work_order_validation_record(
        &self,
        _workspace_dir: &str,
        _validation_record_key: &str,
        _source_task_id: &str,
    ) -> Result<Option<PlanningWorkOrderValidationLink>> {
        bail!("work-order validation correlation is unavailable")
    }

    fn load_work_order_validation_source(
        &self,
        _workspace: &str,
        _validation_record_key: &str,
    ) -> Result<Option<PlanningValidationSource>> {
        bail!("work-order validation source is unavailable")
    }

    fn load_work_order_validation_link(
        &self,
        _workspace_dir: &str,
        _validation_record_key: &str,
    ) -> Result<Option<PlanningWorkOrderValidationLink>> {
        bail!("work-order validation correlation is unavailable")
    }

    fn work_order_validation_record_keys(
        &self,
        _workspace_dir: &str,
        _work_order_id: &str,
    ) -> Result<Vec<String>> {
        bail!("work-order validation correlation is unavailable")
    }

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

pub(crate) fn validate_admission_key(value: &str) -> Result<()> {
    validate_opaque_key(value, "work order task admission key", 1024)
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
