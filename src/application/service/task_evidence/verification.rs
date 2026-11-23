use crate::application::port::inbound::task_evidence_query_port::{
    TaskEvidenceSourceKind, TaskEvidenceSourceProjection, TaskVerificationProjection,
};
use crate::application::port::outbound::task_verification_repository_port::StoredTaskVerification;
use crate::domain::planning::TaskAcceptanceCriterion;
use crate::domain::planning::verification::{
    TaskVerificationCurrentState, TaskVerificationPhase, TaskVerificationResult,
};

use super::*;

mod environment;
pub(super) use environment::VerificationEnvironment;

impl TaskEvidenceService {
    pub(super) fn project_verification(
        &self,
        task: &TaskEvidenceTaskSnapshot,
        criterion: &TaskAcceptanceCriterion,
        source: Option<&(TaskEvidenceSourceProjection, String)>,
        environments: &mut VerificationEnvironment,
    ) -> Result<Option<TaskVerificationProjection>> {
        let Some(stored) = task
            .host_verifications
            .iter()
            .find(|value| value.attempt.criterion_id == criterion.id)
        else {
            return Ok(criterion
                .verification_recipe
                .map(|_| TaskVerificationProjection {
                    attempt: None,
                    phase: TaskVerificationPhase::Pending,
                    receipt: None,
                    state: TaskVerificationCurrentState::Pending,
                    unavailable: None,
                }));
        };
        validate_stored(stored)?;
        let mut projection = TaskVerificationProjection {
            attempt: Some((&stored.attempt).into()),
            phase: stored.phase,
            receipt: stored.receipt.as_ref().map(Into::into),
            state: TaskVerificationCurrentState::Unavailable,
            unavailable: None,
        };
        match self.verification_state(task, criterion, stored, source, environments) {
            Ok(state) => projection.state = state,
            Err(error) => {
                let reason = error
                    .downcast_ref::<TaskEvidenceError>()
                    .copied()
                    .unwrap_or(TaskEvidenceError::Unavailable);
                if matches!(
                    reason,
                    TaskEvidenceError::Stale
                        | TaskEvidenceError::ScopeChanged
                        | TaskEvidenceError::LeaseChanged
                ) {
                    projection.state = TaskVerificationCurrentState::Stale;
                } else {
                    projection.unavailable = Some(reason);
                }
            }
        }
        Ok(Some(projection))
    }

    fn verification_state(
        &self,
        task: &TaskEvidenceTaskSnapshot,
        criterion: &TaskAcceptanceCriterion,
        stored: &StoredTaskVerification,
        source: Option<&(TaskEvidenceSourceProjection, String)>,
        environments: &mut VerificationEnvironment,
    ) -> Result<TaskVerificationCurrentState> {
        if stored.attempt.scope != task.scope
            || criterion.verification_recipe != Some(stored.attempt.recipe)
        {
            return Err(TaskEvidenceError::Stale.into());
        }
        validate_current_context(task, stored)?;
        if let Some(manifest) = &stored.manifest {
            let (source, session) = source.ok_or(TaskEvidenceError::Unavailable)?;
            if source.tree.tree_sha != manifest.source.tree_sha
                || session != &stored.attempt.session_key
            {
                return Err(TaskEvidenceError::Stale.into());
            }
            if source.kind == TaskEvidenceSourceKind::QueuedSource
                && task.queue_records[0].source_base_commit_sha != manifest.base_commit_sha
            {
                return Err(TaskEvidenceError::Stale.into());
            }
            let runner = self
                .verification_runner
                .as_deref()
                .ok_or(TaskEvidenceError::Unavailable)?;
            if environments.observe(runner, manifest, &stored.attempt.attempt_id)?
                != manifest.environment_sha256
            {
                return Err(TaskEvidenceError::Stale.into());
            }
        }
        match stored.phase {
            TaskVerificationPhase::Pending => Ok(TaskVerificationCurrentState::Pending),
            TaskVerificationPhase::Running => Ok(TaskVerificationCurrentState::Running),
            TaskVerificationPhase::Finished => {
                let receipt = stored
                    .receipt
                    .as_ref()
                    .ok_or(TaskEvidenceError::Unavailable)?;
                match &receipt.result {
                    TaskVerificationResult::NotRun { .. }
                    | TaskVerificationResult::Interrupted {} => {
                        Ok(TaskVerificationCurrentState::NotRun)
                    }
                    TaskVerificationResult::Executed { .. } => {
                        let artifact = receipt
                            .artifact
                            .as_ref()
                            .ok_or(TaskEvidenceError::Unavailable)?;
                        self.artifacts.verify_host_capture(
                            &self.workspace,
                            &stored.attempt.attempt_id,
                            artifact,
                        )?;
                        Ok(if receipt.passed() {
                            TaskVerificationCurrentState::Passed
                        } else {
                            TaskVerificationCurrentState::Failed
                        })
                    }
                }
            }
        }
    }
}

fn validate_stored(stored: &StoredTaskVerification) -> Result<()> {
    stored
        .attempt
        .validate()
        .map_err(|_| TaskEvidenceError::Unavailable)?;
    let attempt = &stored.attempt;
    let lease = &stored.lease;
    if lease.task_id != attempt.scope.task_id
        || lease.slot_id != attempt.slot_id
        || lease.lease_generation.as_deref() != Some(attempt.lease_generation.as_str())
        || lease.session_key() != attempt.session_key
    {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    if let Some(manifest) = &stored.manifest {
        manifest
            .validate()
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        if manifest.recipe != attempt.recipe
            || lease
                .delivery_target
                .as_ref()
                .is_none_or(|target| target.integration_base_commit_sha != manifest.base_commit_sha)
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
    }
    if let Some(receipt) = &stored.receipt {
        receipt
            .validate()
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        if receipt.attempt != *attempt || receipt.manifest != stored.manifest {
            return Err(TaskEvidenceError::Unavailable.into());
        }
    }
    if (stored.phase == TaskVerificationPhase::Finished) != stored.receipt.is_some()
        || (stored.phase == TaskVerificationPhase::Pending && stored.manifest.is_some())
        || (stored.phase == TaskVerificationPhase::Running && stored.manifest.is_none())
    {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(())
}

fn validate_current_context(
    task: &TaskEvidenceTaskSnapshot,
    stored: &StoredTaskVerification,
) -> Result<()> {
    if task
        .leases
        .iter()
        .any(|lease| lease.state == ParallelModeSlotLeaseState::Running)
    {
        if task.leases.len() != 1 || task.leases[0] != stored.lease {
            return Err(TaskEvidenceError::Stale.into());
        }
    } else {
        if task.queue_records.len() != 1
            || task.leases.iter().any(|lease| {
                lease.state == ParallelModeSlotLeaseState::Leased
                    || lease.session_key() != stored.attempt.session_key
            })
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        let queue = &task.queue_records[0];
        let original_target = stored
            .lease
            .delivery_target
            .as_ref()
            .ok_or(TaskEvidenceError::Unavailable)?;
        if queue.task_id != task.task.id
            || queue.session_key != stored.attempt.session_key
            || queue.delivery_target.is_none()
            || queue.source_base_commit_sha != original_target.integration_base_commit_sha
        {
            return Err(TaskEvidenceError::Stale.into());
        }
    }
    Ok(())
}
