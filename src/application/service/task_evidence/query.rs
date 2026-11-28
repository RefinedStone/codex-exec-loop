use crate::application::port::inbound::task_evidence_query_port::{
    PreviewTarget, TaskCriterionEvidenceProjection, TaskCriterionWaiverProjection,
    TaskEvidencePlanningSnapshot, TaskEvidenceQueryPort, TaskEvidenceRecordProjection,
    TaskEvidenceSourceKind, TaskEvidenceSourceProjection, TaskEvidenceTaskProjection,
};
use crate::application::port::outbound::task_evidence_repository_port::TaskEvidenceStoredSnapshot;
use crate::domain::planning::evidence::{
    TaskCriterionEvidenceState, TaskEvidenceStatus, TaskEvidenceTree,
};

use super::verification::VerificationEnvironment;
use super::*;

impl TaskEvidenceQueryPort for TaskEvidenceService {
    fn sanitized_preview(&self, target: &PreviewTarget) -> Result<String> {
        self.preview_text(target)
    }

    fn planning_snapshot(
        &self,
        query: &TaskEvidenceSnapshotQuery,
    ) -> Result<TaskEvidencePlanningSnapshot> {
        // Do not combine a cached task definition with a newly read evidence page. A second
        // authority observation makes concurrent criteria/lease/queue changes unavailable.
        let first = match self
            .repository
            .load_current_snapshot(&self.workspace, query)
        {
            Ok(value) => value,
            Err(error) => {
                return Ok(TaskEvidencePlanningSnapshot::unavailable(error_kind(
                    &error,
                )));
            }
        };
        let mut environments = VerificationEnvironment::default();
        let mut tasks: Vec<_> = first
            .tasks
            .iter()
            .map(|task| self.project_task(task, &mut environments))
            .collect();
        let second = match self
            .repository
            .load_current_snapshot(&self.workspace, query)
        {
            Ok(value) => value,
            Err(error) => {
                return Ok(TaskEvidencePlanningSnapshot::unavailable(error_kind(
                    &error,
                )));
            }
        };
        if !same_authority(&first, &second) {
            return Ok(TaskEvidencePlanningSnapshot::unavailable(
                TaskEvidenceError::Stale,
            ));
        }
        environments.finish(self.verification_runner.as_deref(), &mut tasks);
        Ok(TaskEvidencePlanningSnapshot {
            planning_revision: Some(first.planning_revision),
            evidence_revision: Some(first.evidence_revision),
            unavailable: None,
            tasks,
            truncated: first.truncated,
            next_task_id: first.next_task_id,
        })
    }
}

impl TaskEvidenceService {
    fn project_task(
        &self,
        task: &TaskEvidenceTaskSnapshot,
        environments: &mut VerificationEnvironment,
    ) -> TaskEvidenceTaskProjection {
        let mut projection = TaskEvidenceTaskProjection {
            scope: task.scope.clone(),
            source: None,
            unavailable: None,
            criteria: Vec::new(),
        };
        if let Err(error) = validate_task_scope(task) {
            projection.unavailable = Some(error_kind(&error));
            return projection;
        }
        let needs_source = task.task.acceptance.iter().any(|criterion| {
            !task.waivers.iter().any(|receipt| {
                receipt.waiver.criterion_id == criterion.id && receipt.waiver.scope == task.scope
            }) && task.evidence.iter().any(|receipt| {
                receipt.evidence.attempt.criterion_id == criterion.id
                    && receipt.evidence.attempt.scope == task.scope
            })
        }) || task
            .host_verifications
            .iter()
            .any(|stored| stored.attempt.scope == task.scope && stored.manifest.is_some());
        let source = needs_source.then(|| self.observe_source(task));
        if let Some(Ok((current, _))) = &source {
            projection.source = Some(current.clone());
        }
        if let Some(Err(error)) = &source {
            projection.unavailable = Some(error_kind(error));
        }
        for criterion in &task.task.acceptance {
            let evidence = task
                .evidence
                .iter()
                .find(|value| value.evidence.attempt.criterion_id == criterion.id);
            let waiver = task
                .waivers
                .iter()
                .find(|value| value.waiver.criterion_id == criterion.id);
            let mut row = TaskCriterionEvidenceProjection {
                criterion: criterion.clone(),
                state: Some(TaskCriterionEvidenceState::Pending),
                evidence: evidence.map(record_projection),
                waiver: waiver.map(waiver_projection),
                host_verification: None,
            };
            if evidence.is_some_and(|value| value.validate().is_err())
                || waiver.is_some_and(|value| value.validate().is_err())
            {
                row.state = None;
                projection.unavailable = Some(TaskEvidenceError::Unavailable);
            } else if waiver.is_some_and(|value| value.waiver.scope == task.scope) {
                row.state = Some(TaskCriterionEvidenceState::Waived);
            } else if let Some(receipt) = evidence {
                let record = &receipt.evidence;
                if record.attempt.scope != task.scope {
                    row.state = Some(TaskCriterionEvidenceState::Stale);
                } else if let Some(Ok((current, session))) = &source {
                    if record.recorded_tree.tree_sha != current.tree.tree_sha
                        || record.attempt.session_key != *session
                    {
                        row.state = Some(TaskCriterionEvidenceState::Stale);
                    } else if let Err(error) = self.artifacts.verify_sealed(
                        &self.workspace,
                        &record.attempt,
                        &record.artifact,
                    ) {
                        row.state = None;
                        projection.unavailable = Some(error_kind(&error));
                    } else {
                        row.state = Some(
                            if let Some(review) = task.reviews.iter().find(|r| r.matches(receipt)) {
                                match review.decision {
                                crate::domain::planning::evidence::HumanEvidenceDecision::Pass => TaskCriterionEvidenceState::HumanPassed,
                                crate::domain::planning::evidence::HumanEvidenceDecision::Fail => TaskCriterionEvidenceState::HumanFailed,
                            }
                            } else {
                                match record.status {
                                    TaskEvidenceStatus::Pass => TaskCriterionEvidenceState::Pass,
                                    TaskEvidenceStatus::Fail => TaskCriterionEvidenceState::Fail,
                                    TaskEvidenceStatus::Blocked => {
                                        TaskCriterionEvidenceState::Blocked
                                    }
                                }
                            },
                        );
                    }
                } else {
                    row.state = None;
                }
            } else if waiver.is_some() {
                row.state = Some(TaskCriterionEvidenceState::Stale);
            }
            match self.project_verification(
                task,
                criterion,
                source.as_ref().and_then(|source| source.as_ref().ok()),
                environments,
            ) {
                Ok(host) => row.host_verification = host,
                Err(error) => projection.unavailable = Some(error_kind(&error)),
            }
            projection.criteria.push(row);
        }
        if let Some(Ok((initial, initial_session))) = &source
            && initial.kind == TaskEvidenceSourceKind::RunningLease
        {
            // Artifact verification can take time while the worker continues editing.
            // The second DB read cannot detect those filesystem-only changes. Fence
            // this live source again before presenting any result as current.
            match self.observe_source(task) {
                Ok((current, session))
                    if current.kind == initial.kind
                        && current.tree == initial.tree
                        && session == *initial_session => {}
                Ok(_) => {
                    projection.source = None;
                    projection.unavailable = Some(TaskEvidenceError::Stale);
                }
                Err(error) => {
                    projection.source = None;
                    projection.unavailable = Some(error_kind(&error));
                }
            }
        }
        if projection.unavailable.is_some() {
            // A task has one observed source. If any current artifact cannot be verified,
            // do not mix a seemingly usable Pass with that task's unavailable observation.
            for row in &mut projection.criteria {
                if let Some(host) = &mut row.host_verification
                    && matches!(
                        host.state,
                        crate::domain::planning::verification::TaskVerificationCurrentState::Passed
                            | crate::domain::planning::verification::TaskVerificationCurrentState::Failed
                            | crate::domain::planning::verification::TaskVerificationCurrentState::Running
                    )
                {
                    host.state = crate::domain::planning::verification::TaskVerificationCurrentState::Unavailable;
                    host.unavailable = projection.unavailable;
                }
                if matches!(
                    row.state,
                    Some(
                        TaskCriterionEvidenceState::Pass
                            | TaskCriterionEvidenceState::Fail
                            | TaskCriterionEvidenceState::Blocked
                            | TaskCriterionEvidenceState::HumanPassed
                            | TaskCriterionEvidenceState::HumanFailed
                    )
                ) {
                    row.state = None;
                }
            }
        }
        projection
    }

    fn observe_source(
        &self,
        task: &TaskEvidenceTaskSnapshot,
    ) -> Result<(TaskEvidenceSourceProjection, String)> {
        if task
            .leases
            .iter()
            .any(|lease| lease.state == ParallelModeSlotLeaseState::Running)
        {
            let lease = running_lease(task)?;
            let tree = with_running_worker(
                self.authority.as_ref(),
                self.runtime.as_ref(),
                &self.workspace,
                &lease,
                || snapshot_worker_tree(self.runtime.as_ref(), &lease),
            )?;
            return Ok((
                TaskEvidenceSourceProjection {
                    kind: TaskEvidenceSourceKind::RunningLease,
                    tree,
                },
                lease.session_key(),
            ));
        }
        if task
            .leases
            .iter()
            .any(|lease| lease.state == ParallelModeSlotLeaseState::Leased)
            || task.queue_records.len() != 1
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        let queue = &task.queue_records[0];
        if queue.task_id != task.task.id
            || queue.delivery_target.is_none()
            || queue.source_branch.trim().is_empty()
            || queue.source_base_commit_sha.trim().is_empty()
            || queue.session_key.trim().is_empty()
            || task
                .leases
                .iter()
                .any(|lease| lease.session_key() != queue.session_key)
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        // The frozen source is mandatory. Mutable branch/commit legacy fallbacks cannot prove
        // which bytes the worker recorded, and this observation never fetches remote objects.
        let tree_sha = self
            .runtime
            .read_commit_tree(&self.workspace, &queue.source_commit_sha)
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        let tree = TaskEvidenceTree {
            head_commit_sha: queue.source_commit_sha.clone(),
            tree_sha,
            worktree_clean: true,
        };
        tree.validate()?;
        Ok((
            TaskEvidenceSourceProjection {
                kind: TaskEvidenceSourceKind::QueuedSource,
                tree,
            },
            queue.session_key.clone(),
        ))
    }
}

fn error_kind(error: &anyhow::Error) -> TaskEvidenceError {
    error
        .downcast_ref::<TaskEvidenceError>()
        .copied()
        .unwrap_or(TaskEvidenceError::Unavailable)
}

fn same_authority(first: &TaskEvidenceStoredSnapshot, second: &TaskEvidenceStoredSnapshot) -> bool {
    first == second
}

fn record_projection(receipt: &TaskEvidenceReceipt) -> TaskEvidenceRecordProjection {
    let record = &receipt.evidence;
    TaskEvidenceRecordProjection {
        event_id: receipt.event_id.clone(),
        evidence_revision: receipt.evidence_revision,
        scope: record.attempt.scope.clone(),
        criterion_id: record.attempt.criterion_id.clone(),
        attempt_id: record.attempt.attempt_id.clone(),
        attempt: record.attempt.attempt,
        status: record.status,
        artifact: record.artifact.clone(),
        recorded_tree: record.recorded_tree.clone(),
        recorded_at: record.recorded_at.clone(),
        by: record.by.clone(),
    }
}

fn waiver_projection(receipt: &TaskCriterionWaiverReceipt) -> TaskCriterionWaiverProjection {
    TaskCriterionWaiverProjection {
        event_id: receipt.event_id.clone(),
        evidence_revision: receipt.evidence_revision,
        scope: receipt.waiver.scope.clone(),
        criterion_id: receipt.waiver.criterion_id.clone(),
        reason_digest: receipt.waiver.reason_digest.clone(),
        recorded_at: receipt.waiver.recorded_at.clone(),
    }
}
