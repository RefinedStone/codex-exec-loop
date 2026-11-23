use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitDecision, PlanningWorkPermitStage,
};
use crate::application::port::outbound::task_evidence_repository_port::TaskEvidenceSnapshotQuery;
use crate::domain::parallel_mode::ParallelModeSlotLeaseState;
use crate::domain::planning::TaskStatus;
use crate::domain::planning::evidence::criteria_sha256;

use super::*;

pub(super) fn require_current(
    dependencies: &TaskVerificationDependencies,
    stored: &StoredTaskVerification,
) -> Result<()> {
    stored.attempt.validate()?;
    let attempt = &stored.attempt;
    let lease = &stored.lease;
    if lease.task_id != attempt.scope.task_id
        || lease.slot_id != attempt.slot_id
        || lease.lease_generation.as_deref() != Some(attempt.lease_generation.as_str())
        || lease.session_key() != attempt.session_key
        || lease.state != ParallelModeSlotLeaseState::Running
    {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    let snapshot = dependencies.repository.load_current_snapshot(
        &dependencies.workspace,
        &TaskEvidenceSnapshotQuery {
            task_ids: vec![attempt.scope.task_id.clone()],
            after_task_id: None,
            limit: 1,
        },
    )?;
    if snapshot.tasks.len() != 1 {
        return Err(TaskEvidenceError::ScopeChanged.into());
    }
    let current = &snapshot.tasks[0];
    if current.task.id != attempt.scope.task_id
        || current.scope != attempt.scope
        || current.task.acceptance_revision != attempt.scope.acceptance_revision
        || criteria_sha256(&current.task.acceptance)? != attempt.scope.criteria_sha256
        || !current.task.acceptance.iter().any(|criterion| {
            criterion.id == attempt.criterion_id
                && criterion.verification_recipe == Some(attempt.recipe)
        })
    {
        return Err(TaskEvidenceError::ScopeChanged.into());
    }
    if !matches!(
        current.task.status,
        TaskStatus::Ready | TaskStatus::InProgress
    ) || current.leases.len() != 1
        || current.leases[0] != *lease
        || dependencies
            .authority
            .worker_task_requires_preservation(&dependencies.workspace, &attempt.scope.task_id)?
    {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    let base = lease
        .delivery_target
        .as_ref()
        .ok_or(TaskEvidenceError::LeaseChanged)?;
    if !matches!(
        dependencies.authority.check_work_order_permit_for_task(
            &dependencies.workspace,
            &attempt.scope.task_id,
            PlanningWorkPermitStage::Dispatch,
            Some(&base.integration_base_commit_sha),
        )?,
        PlanningWorkPermitDecision::Legacy | PlanningWorkPermitDecision::Granted { .. }
    ) {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    Ok(())
}

pub(super) fn not_run_reason(error: &anyhow::Error) -> TaskVerificationNotRunReason {
    match error.downcast_ref::<TaskEvidenceError>() {
        Some(TaskEvidenceError::Stale) => TaskVerificationNotRunReason::Stale,
        Some(TaskEvidenceError::ScopeChanged) => TaskVerificationNotRunReason::ScopeChanged,
        Some(TaskEvidenceError::LeaseChanged) => TaskVerificationNotRunReason::LeaseChanged,
        _ => TaskVerificationNotRunReason::Unavailable,
    }
}
