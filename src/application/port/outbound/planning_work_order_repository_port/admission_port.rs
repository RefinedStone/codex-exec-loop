use anyhow::Result;

use super::{
    PlanningTaskAuthorityMutationAudit, PlanningWorkControl, PlanningWorkOrderRecord,
    PlanningWorkPermitBasis, PriorityQueueProjection, TaskAuthorityDocument,
};

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

pub(crate) fn validate_admission_key(value: &str) -> Result<()> {
    super::validate_opaque_key(value, "work order task admission key", 1024)
}
