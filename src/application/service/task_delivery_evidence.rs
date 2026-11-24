//! Evaluate durable evidence against one exact delivery source, independently of UI projections.
use std::path::Path;
use std::sync::Arc;

use crate::application::port::inbound::task_delivery_evidence_port::{
    TaskDeliveryEvidenceDecision, TaskDeliveryEvidenceGatePort, TaskDeliveryEvidenceIntegration,
};
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
};
use crate::application::port::outbound::task_delivery_evidence_repository_port::{
    StoredTaskDeliveryEvidence, TaskDeliveryEvidenceObservationCommit,
    TaskDeliveryEvidenceOperation, TaskDeliveryEvidenceRepositoryPort,
};
use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::application::port::outbound::task_verification_runner_port::TaskVerificationRunnerPort;
use crate::domain::planning::delivery_evidence::*;

mod criteria;
mod environment;
mod source;
#[cfg(test)]
mod tests;

pub struct TaskDeliveryEvidenceDependencies {
    pub workspace: String,
    /// The same authority incarnation that supplied the immutable effective policy.
    pub repository_identity: String,
    pub repository: Arc<dyn TaskDeliveryEvidenceRepositoryPort>,
    pub artifacts: Arc<dyn TaskEvidenceArtifactPort>,
    pub runtime: Arc<dyn ParallelModeRuntimePort>,
    pub authority: Arc<dyn PlanningAuthorityPort>,
    pub runner: Arc<dyn TaskVerificationRunnerPort>,
}

pub struct TaskDeliveryEvidenceService {
    dependencies: TaskDeliveryEvidenceDependencies,
    mode: Result<TaskEvidenceDeliveryMode, String>,
}

impl TaskDeliveryEvidenceService {
    /// The canonical workspace's effective policy is captured once by production composition.
    /// Construction never reads evidence, initializes a verifier owner, or executes a recipe.
    pub fn new(
        dependencies: TaskDeliveryEvidenceDependencies,
        mode: Result<TaskEvidenceDeliveryMode, String>,
    ) -> Self {
        Self { dependencies, mode }
    }

    fn effective_mode(
        record: &PlanningAuthorityDistributorQueueRecord,
        stage: TaskEvidenceDeliveryStage,
        configured: TaskEvidenceDeliveryMode,
    ) -> TaskEvidenceDeliveryMode {
        // Delivery mode is captured in the durable stage observation. A second process
        // configured with Observe must not downgrade a wait created under Enforce.
        if record.evidence_wait == Some(stage)
            && record.evidence_observations.iter().any(|observation| {
                observation.stage == stage && observation.mode == TaskEvidenceDeliveryMode::Enforce
            })
        {
            TaskEvidenceDeliveryMode::Enforce
        } else {
            configured
        }
    }

    fn observe(
        &self,
        snapshot: &StoredTaskDeliveryEvidence,
        stage: TaskEvidenceDeliveryStage,
        integration: Option<&TaskDeliveryEvidenceIntegration>,
        mode: TaskEvidenceDeliveryMode,
    ) -> TaskDeliveryEvidenceObservation {
        let mut observation = TaskDeliveryEvidenceObservation {
            stage,
            mode,
            scope: snapshot.task.as_ref().map(|task| task.scope.clone()),
            evidence_revision: snapshot.evidence_revision,
            source: None,
            criteria: Vec::new(),
            reasons: Vec::new(),
            observed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        match source::observe(&self.dependencies, &snapshot.queue, stage, integration) {
            Ok(source) => {
                if source.integration.as_ref().is_some_and(|result| {
                    result.base_commit_sha != source.base_commit_sha
                        || result.result_tree_sha != source.source_tree_sha
                }) {
                    add_reason(
                        &mut observation,
                        TaskDeliveryEvidenceReason::IntegrationSourceMismatch,
                    );
                }
                observation.source = Some(source);
            }
            Err(reason) => add_reason(&mut observation, reason),
        }
        let mut environments = environment::Environments::default();
        criteria::evaluate(
            &self.dependencies,
            snapshot,
            &mut observation,
            &mut environments,
        );
        if let Some(initial) = &observation.source {
            match source::observe(&self.dependencies, &snapshot.queue, stage, integration) {
                Ok(current) if current == *initial => {}
                Ok(_) => add_reason(&mut observation, TaskDeliveryEvidenceReason::EvidenceStale),
                Err(reason) => add_reason(&mut observation, reason),
            }
        }
        // This second read catches authority changes made during artifact/source observations.
        // The final write repeats the comparison atomically with the active distributor claim.
        match self.dependencies.repository.load_delivery_evidence(
            &self.dependencies.workspace,
            &snapshot.queue,
            stage,
        ) {
            Ok(current) if current == *snapshot => {}
            Ok(_) => add_reason(&mut observation, TaskDeliveryEvidenceReason::EvidenceStale),
            Err(_) => add_reason(
                &mut observation,
                TaskDeliveryEvidenceReason::EvidenceUnavailable,
            ),
        }
        environments.finish(self.dependencies.runner.as_ref(), &mut observation);
        observation
    }
}

impl TaskDeliveryEvidenceGatePort for TaskDeliveryEvidenceService {
    fn evaluate(
        &self,
        workspace: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
        stage: TaskEvidenceDeliveryStage,
        integration: Option<&TaskDeliveryEvidenceIntegration>,
        operation: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String> {
        let configured_mode = *self.mode.as_ref().map_err(|_| {
            "evidence_unavailable: the effective delivery evidence policy could not be resolved"
                .to_owned()
        })?;
        let mode = Self::effective_mode(record, stage, configured_mode);
        let is_recheck = matches!(operation, TaskDeliveryEvidenceOperation::Recheck);
        let location = self
            .dependencies
            .authority
            .resolve_authority_location(workspace)
            .map_err(|_| {
                "evidence_unavailable: repository authority could not be resolved".to_owned()
            })?;
        if self.dependencies.repository_identity.trim().is_empty()
            || location.repository_identity != self.dependencies.repository_identity
        {
            return Err("evidence_unavailable: repository identity changed since delivery policy was resolved".into());
        }
        if !self.dependencies.runtime.paths_match_securely(
            Path::new(&location.canonical_repo_root),
            Path::new(&self.dependencies.workspace),
        ) {
            return Err("evidence_unavailable: delivery belongs to another repository".into());
        }
        let snapshot = match self.dependencies.repository.load_delivery_evidence(
            &self.dependencies.workspace,
            record,
            stage,
        ) {
            Ok(snapshot) => snapshot,
            Err(error)
                if is_recheck
                    && error
                        .downcast_ref::<TaskDeliveryEvidenceError>()
                        .is_some_and(|error| *error == TaskDeliveryEvidenceError::Conflict) =>
            {
                return Err(
                    "evidence_retry: queue changed while the typed wait was being rechecked".into(),
                );
            }
            Err(_) => {
                return Err(
                    "evidence_unavailable: exact delivery authority could not be read".into(),
                );
            }
        };
        if snapshot.queue != *record {
            return Err(if is_recheck {
                "evidence_retry: queue changed while the typed wait was being rechecked".into()
            } else {
                "evidence_stale: queue changed before the delivery effect was authorized".into()
            });
        }
        if snapshot.requires_preservation || snapshot.permit.blocked_reason().is_some() {
            if is_recheck {
                return Ok(waiting_recheck_decision(
                    record,
                    TaskDeliveryEvidenceReason::EvidenceStale,
                ));
            }
            return Err(
                "evidence_stale: current task authority does not permit this delivery".into(),
            );
        }
        if stage == TaskEvidenceDeliveryStage::Integrate
            && matches!(operation, TaskDeliveryEvidenceOperation::Effect { .. })
            && integration.and_then(|value| value.preflight_requires_human_review)
                != Some(snapshot.permit.requires_human_review())
        {
            return Err("evidence_stale: integration review requirements changed after the final pull request check".into());
        }
        let observation = self.observe(&snapshot, stage, integration, mode);
        let committed = match self
            .dependencies
            .repository
            .commit_delivery_evidence_observation(
                &self.dependencies.workspace,
                &TaskDeliveryEvidenceObservationCommit {
                    expected: snapshot,
                    operation: operation.clone(),
                    observation,
                },
            ) {
            Ok(committed) => committed,
            Err(error) if is_recheck => match error.downcast_ref::<TaskDeliveryEvidenceError>() {
                Some(
                    TaskDeliveryEvidenceError::ControlDenied | TaskDeliveryEvidenceError::ClaimLost,
                ) => {
                    return Ok(waiting_recheck_decision(
                        record,
                        TaskDeliveryEvidenceReason::EvidenceUnavailable,
                    ));
                }
                Some(TaskDeliveryEvidenceError::Conflict) => {
                    return Err(format!("evidence_retry: {}", error));
                }
                _ => {
                    return Err(
                            "evidence_unavailable: delivery authority could not commit the wait recheck"
                                .into(),
                        );
                }
            },
            Err(_) => {
                return Err(
                    "evidence_stale: delivery authority changed before the final observation was committed"
                        .into(),
                );
            }
        };
        Ok(TaskDeliveryEvidenceDecision {
            updated_queue: committed.updated_queue,
            allowed: committed.effect_allowed,
            reason: committed.observation.reasons.first().copied(),
        })
    }
}

fn waiting_recheck_decision(
    record: &PlanningAuthorityDistributorQueueRecord,
    reason: TaskDeliveryEvidenceReason,
) -> TaskDeliveryEvidenceDecision {
    TaskDeliveryEvidenceDecision {
        updated_queue: record.clone(),
        allowed: false,
        reason: Some(reason),
    }
}

fn add_reason(
    observation: &mut TaskDeliveryEvidenceObservation,
    reason: TaskDeliveryEvidenceReason,
) {
    if !observation.reasons.contains(&reason) {
        observation.reasons.push(reason);
    }
}
