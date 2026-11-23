//! Records worker-reported results against host-observed bytes and a current acceptance scope.
//! E2 does not change completion, dispatch or publication policy.
use std::sync::Arc;

use anyhow::Result;

use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationDisplay, IssueConfirmationPreview, IssueConfirmationTarget,
};
use crate::application::port::inbound::stored_command_port::task_evidence::{
    PrepareTaskEvidenceRequest, RecordTaskEvidenceRequest, WaiveTaskCriterionRequest,
};
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::application::port::outbound::task_evidence_repository_port::{
    TaskCriterionWaiverCommit, TaskEvidencePrepareCommit, TaskEvidenceRecordCommit,
    TaskEvidenceRepositoryPort, TaskEvidenceSnapshotQuery, TaskEvidenceTaskSnapshot,
};
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use crate::domain::planning::evidence::{
    TaskCriterionWaiverReceipt, TaskEvidenceError, TaskEvidenceEvent, TaskEvidencePrepared,
    TaskEvidenceReceipt, TaskEvidenceWriteResult, criteria_sha256, normalize_evidence_note,
    normalize_waiver_reason, validate_evidence_request_id,
};

use super::parallel_mode::evidence::{snapshot_worker_tree, with_running_worker};

mod query;
#[cfg(test)]
pub(crate) mod tests;
mod verification;

pub struct TaskEvidenceService {
    workspace: String,
    repository: Arc<dyn TaskEvidenceRepositoryPort>,
    artifacts: Arc<dyn TaskEvidenceArtifactPort>,
    runtime: Arc<dyn ParallelModeRuntimePort>,
    authority: Arc<dyn PlanningAuthorityPort>,
    verification_runner: Option<Arc<dyn crate::application::port::outbound::task_verification_runner_port::TaskVerificationRunnerPort>>,
}

impl TaskEvidenceService {
    pub fn new(
        workspace: String,
        repository: Arc<dyn TaskEvidenceRepositoryPort>,
        artifacts: Arc<dyn TaskEvidenceArtifactPort>,
        runtime: Arc<dyn ParallelModeRuntimePort>,
        authority: Arc<dyn PlanningAuthorityPort>,
    ) -> Self {
        Self {
            workspace,
            repository,
            artifacts,
            runtime,
            authority,
            verification_runner: None,
        }
    }

    pub fn with_verification_runner(
        mut self,
        runner: Arc<dyn crate::application::port::outbound::task_verification_runner_port::TaskVerificationRunnerPort>,
    ) -> Self {
        self.verification_runner = Some(runner);
        self
    }

    pub fn prepare_evidence(
        &self,
        request: PrepareTaskEvidenceRequest,
        context: &StoredCommandContext,
    ) -> Result<TaskEvidencePrepared> {
        request.validate()?;
        let current = self.current_criterion(
            &request.task_id,
            &request.criterion_id,
            &request.scope_id,
            request.expected_acceptance_revision,
        )?;
        let lease = running_lease(&current)?;
        with_running_worker(
            self.authority.as_ref(),
            self.runtime.as_ref(),
            &self.workspace,
            &lease,
            || {
                let tree = snapshot_worker_tree(self.runtime.as_ref(), &lease)?;
                let result = self
                    .repository
                    .prepare_attempt(
                        &self.workspace,
                        &TaskEvidencePrepareCommit {
                            request_id: request.request_id,
                            scope: current.scope.clone(),
                            criterion_id: request.criterion_id,
                            lease: lease.clone(),
                            prepared_tree: tree.clone(),
                            at: context.at.clone(),
                        },
                    )
                    .map_err(closed_error)?;
                let artifact_path = self
                    .artifacts
                    .prepare_artifact(&self.workspace, &result.value)
                    .map_err(closed_error)?;
                if tree != snapshot_worker_tree(self.runtime.as_ref(), &lease)? {
                    return Err(TaskEvidenceError::Stale.into());
                }
                self.require_current_attempt(&result.value, &lease)?;
                let artifact_path = artifact_path
                    .to_str()
                    .ok_or(TaskEvidenceError::Unavailable)?
                    .to_owned();
                Ok(TaskEvidencePrepared {
                    attempt: result.value,
                    artifact_path,
                    changed: result.changed,
                })
            },
        )
    }

    pub fn record_evidence(
        &self,
        request: RecordTaskEvidenceRequest,
        context: &StoredCommandContext,
    ) -> Result<TaskEvidenceWriteResult<TaskEvidenceReceipt>> {
        request.validate()?;
        let note = normalize_evidence_note(request.note.as_deref())?;
        let stored = self
            .repository
            .load_attempt(&self.workspace, &request.attempt_id)
            .map_err(closed_error)?
            .ok_or(TaskEvidenceError::Missing)?;
        // A retry returns an immutable past fact. Only the query projection determines whether
        // that fact is still current; changing caller/time must not mint another receipt.
        if let Some(receipt) = stored.receipt {
            receipt.validate()?;
            return if receipt.evidence.status == request.status && receipt.evidence.note == note {
                Ok(TaskEvidenceWriteResult {
                    value: receipt,
                    changed: false,
                })
            } else {
                Err(TaskEvidenceError::Conflict.into())
            };
        }
        let attempt = stored.attempt;
        let lease = stored.lease;
        self.require_current_attempt(&attempt, &lease)?;
        with_running_worker(
            self.authority.as_ref(),
            self.runtime.as_ref(),
            &self.workspace,
            &lease,
            || {
                let tree = snapshot_worker_tree(self.runtime.as_ref(), &lease)?;
                if tree != attempt.prepared_tree {
                    return Err(TaskEvidenceError::Stale.into());
                }
                let artifact = self
                    .artifacts
                    .observe_and_seal(&self.workspace, &attempt, &tree)
                    .map_err(closed_error)?;
                if tree != snapshot_worker_tree(self.runtime.as_ref(), &lease)? {
                    return Err(TaskEvidenceError::Stale.into());
                }
                self.require_current_attempt(&attempt, &lease)?;
                let by = context.caller.as_deref().unwrap_or("kernel");
                validate_evidence_request_id(by)?;
                self.repository
                    .record_evidence(
                        &self.workspace,
                        &TaskEvidenceRecordCommit {
                            attempt,
                            lease: lease.clone(),
                            status: request.status,
                            artifact,
                            recorded_tree: tree,
                            note,
                            by: by.to_owned(),
                            at: context.at.clone(),
                        },
                    )
                    .map_err(closed_error)
            },
        )
    }

    pub fn prepare_waiver(
        &self,
        request: &WaiveTaskCriterionRequest,
    ) -> Result<IssueConfirmationPreview> {
        request.validate()?;
        let current = self.current_criterion(
            &request.task_id,
            &request.criterion_id,
            &request.scope_id,
            request.expected_acceptance_revision,
        )?;
        Ok(IssueConfirmationPreview {
            target: self.waiver_target(request),
            display: IssueConfirmationDisplay {
                message: "이 완료 기준을 면제하시겠습니까?".into(),
                detail: format!(
                    "작업: {}\n기준: {}\n완료 기준 판: {}\n사유: {}\n\n이 면제는 현재 완료 기준 판에 적용됩니다. 소스 tree가 바뀌어도 유지되며, 완료 기준 판이 바뀌면 무효입니다. 명세·전달·통합 승인은 별도로 필요합니다.",
                    current.task.title,
                    request.criterion_id,
                    request.expected_acceptance_revision,
                    request.reason.trim(),
                ),
                confirm_label: "이 기준 면제".into(),
            },
        })
    }

    pub fn waive_criterion(
        &self,
        request: WaiveTaskCriterionRequest,
        context: &StoredCommandContext,
    ) -> Result<TaskEvidenceWriteResult<TaskCriterionWaiverReceipt>> {
        request.validate()?;
        let current = self.current_criterion(
            &request.task_id,
            &request.criterion_id,
            &request.scope_id,
            request.expected_acceptance_revision,
        )?;
        let confirmation = context
            .human_confirmation
            .as_ref()
            .ok_or(TaskEvidenceError::HumanConfirmationRequired)?;
        confirmation
            .require_target(&self.waiver_target(&request))
            .map_err(|_| TaskEvidenceError::HumanConfirmationRequired)?;
        let (reason, _) = normalize_waiver_reason(&request.reason)?;
        self.repository
            .waive_criterion(
                &self.workspace,
                &TaskCriterionWaiverCommit {
                    request_id: request.request_id,
                    scope: current.scope,
                    criterion_id: request.criterion_id,
                    reason,
                    confirmation_ref: confirmation.reference().to_owned(),
                    at: context.at.clone(),
                },
            )
            .map_err(closed_error)
    }

    pub fn pending_events(&self, limit: usize) -> Result<Vec<TaskEvidenceEvent>> {
        self.repository.pending_events(&self.workspace, limit)
    }

    pub fn acknowledge_event(&self, event_id: &str) -> Result<()> {
        self.repository.acknowledge_event(&self.workspace, event_id)
    }

    fn waiver_target(&self, request: &WaiveTaskCriterionRequest) -> IssueConfirmationTarget {
        IssueConfirmationTarget::CriterionWaiver {
            workspace: self.workspace.clone(),
            task_id: request.task_id.clone(),
            criterion_id: request.criterion_id.clone(),
            scope_id: request.scope_id.clone(),
            acceptance_revision: request.expected_acceptance_revision,
            reason_digest: request.reason_digest(),
        }
    }

    fn current_criterion(
        &self,
        task_id: &str,
        criterion_id: &str,
        scope_id: &str,
        revision: u64,
    ) -> Result<TaskEvidenceTaskSnapshot> {
        let snapshot = self
            .repository
            .load_current_snapshot(
                &self.workspace,
                &TaskEvidenceSnapshotQuery {
                    task_ids: vec![task_id.to_owned()],
                    after_task_id: None,
                    limit: 1,
                },
            )
            .map_err(closed_error)?;
        let mut tasks = snapshot.tasks.into_iter();
        let current = tasks.next().ok_or(TaskEvidenceError::Missing)?;
        if tasks.next().is_some() || current.task.id != task_id {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        validate_task_scope(&current)?;
        if current.scope.scope_id != scope_id || current.scope.acceptance_revision != revision {
            return Err(TaskEvidenceError::ScopeChanged.into());
        }
        if !current
            .task
            .acceptance
            .iter()
            .any(|value| value.id == criterion_id)
        {
            return Err(TaskEvidenceError::Missing.into());
        }
        Ok(current)
    }

    fn require_current_attempt(
        &self,
        attempt: &crate::domain::planning::evidence::TaskEvidenceAttempt,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()> {
        attempt.validate()?;
        let current = self.current_criterion(
            &attempt.scope.task_id,
            &attempt.criterion_id,
            &attempt.scope.scope_id,
            attempt.scope.acceptance_revision,
        )?;
        if current.scope != attempt.scope
            || !running_lease(&current)?.same_generation_as(lease)
            || lease.session_key() != attempt.session_key
        {
            return Err(TaskEvidenceError::LeaseChanged.into());
        }
        Ok(())
    }
}

fn validate_task_scope(task: &TaskEvidenceTaskSnapshot) -> Result<()> {
    task.scope.validate()?;
    if task.task.id != task.scope.task_id
        || task.task.acceptance_revision != task.scope.acceptance_revision
        || criteria_sha256(&task.task.acceptance)? != task.scope.criteria_sha256
    {
        return Err(TaskEvidenceError::ScopeChanged.into());
    }
    Ok(())
}

fn running_lease(task: &TaskEvidenceTaskSnapshot) -> Result<ParallelModeSlotLeaseSnapshot> {
    // An additional Leased or cleanup generation is ambiguity, not permission to select one.
    if task.leases.len() != 1 {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    let lease = &task.leases[0];
    if lease.task_id != task.task.id
        || lease.state != ParallelModeSlotLeaseState::Running
        || lease.lease_generation.is_none()
        || !lease.has_valid_lease_generation()
    {
        return Err(TaskEvidenceError::LeaseChanged.into());
    }
    Ok(lease.clone())
}

fn closed_error(error: anyhow::Error) -> anyhow::Error {
    error
        .downcast_ref::<TaskEvidenceError>()
        .copied()
        .unwrap_or(TaskEvidenceError::Unavailable)
        .into()
}
