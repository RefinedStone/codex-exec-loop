//! Exact queue selection and the final authority CAS for an evidence observation.
use anyhow::Result;

use super::github_automation_port::GithubRepositoryVisibility;
use super::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use super::planning_work_order_repository_port::{
    PlanningWorkDeliveryAuthority, PlanningWorkPermitBasis, PlanningWorkPermitDecision,
};
use super::task_evidence_repository_port::TaskEvidenceTaskSnapshot;
use crate::domain::planning::delivery_evidence::{
    TaskDeliveryEvidenceError, TaskDeliveryEvidenceObservation, TaskDeliveryEvidenceReason,
    TaskEvidenceDeliveryMode, TaskEvidenceDeliveryStage,
};

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTaskDeliveryEvidence {
    pub planning_revision: i64,
    pub evidence_revision: u64,
    pub queue: PlanningAuthorityDistributorQueueRecord,
    /// Missing legacy task/scope is an observable lack of evidence, not a fabricated empty task.
    pub task: Option<TaskEvidenceTaskSnapshot>,
    pub permit: PlanningWorkPermitDecision,
    /// Includes the exact reviewed authority, repository and Merge revision in the final CAS.
    pub permit_basis: Option<PlanningWorkPermitBasis>,
    pub requires_preservation: bool,
}

impl StoredTaskDeliveryEvidence {
    /// Automatic delivery cannot relax policy or replace all machine-checkable criteria with
    /// human waivers. Human/legacy delivery retains the existing Observe/Enforce semantics.
    pub fn auto_policy_reasons(
        &self,
        mode: TaskEvidenceDeliveryMode,
    ) -> Vec<TaskDeliveryEvidenceReason> {
        if !self.permit.requires_auto_delivery() {
            return Vec::new();
        }
        let mut reasons = Vec::new();
        if mode != TaskEvidenceDeliveryMode::Enforce {
            reasons.push(TaskDeliveryEvidenceReason::AutoRequiresEnforce);
        }
        let machine_criterion = self.task.as_ref().is_some_and(|task| task.task.acceptance.iter().any(|criterion| {
            criterion.essential
                && criterion.channel == crate::domain::planning::TaskAcceptanceChannel::Command
                && criterion.verification_recipe == Some(crate::domain::planning::verification::TaskVerificationRecipe::GitDiffCheckV1)
        }));
        if !machine_criterion {
            reasons.push(TaskDeliveryEvidenceReason::AutoRequiresMachineCriterion);
        }
        reasons
    }

    /// An Auto grant belongs to one reviewed source and repository. The adapter repeats this
    /// check while holding the final transaction; a live visibility read remains the caller's job.
    pub fn require_auto_authority(&self) -> Result<()> {
        if !self.permit.requires_auto_delivery() {
            return Ok(());
        }
        let basis = self
            .permit_basis
            .as_ref()
            .ok_or(TaskDeliveryEvidenceError::ControlDenied)?;
        basis
            .validate()
            .map_err(|_| TaskDeliveryEvidenceError::ControlDenied)?;
        let review = basis
            .delivery_review
            .as_ref()
            .ok_or(TaskDeliveryEvidenceError::ControlDenied)?;
        let Some(PlanningWorkDeliveryAuthority::Auto {
            source_repository, ..
        }) = &review.authority
        else {
            return Err(TaskDeliveryEvidenceError::ControlDenied.into());
        };
        let target = self
            .queue
            .delivery_target
            .as_ref()
            .ok_or(TaskDeliveryEvidenceError::ControlDenied)?;
        // Frozen targets retain the provider's display casing. Compare their canonical spelling
        // with the validated lowercase source without rewriting the persisted target identity.
        let canonical_target = target.github_repository.to_ascii_lowercase();
        if source_repository != &canonical_target
            || review.source_commit_sha != self.queue.effective_source_commit_sha()
            || review.base_commit_sha != self.queue.source_base_commit_sha
            || !matches!(
                target.repository_visibility,
                GithubRepositoryVisibility::Private | GithubRepositoryVisibility::Internal
            )
        {
            return Err(TaskDeliveryEvidenceError::ControlDenied.into());
        }
        Ok(())
    }
}

#[derive(Clone)]
pub enum TaskDeliveryEvidenceOperation {
    /// Only the active distributor claim may authorize this immediately following effect.
    Effect {
        owner_token: String,
        preflight_permit: PlanningWorkPermitDecision,
    },
    /// A read/rearm of one typed evidence wait; never authorizes an external effect.
    Recheck,
}

impl std::fmt::Debug for TaskDeliveryEvidenceOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Effect { .. } => "Effect { .. }",
            Self::Recheck => "Recheck",
        })
    }
}

#[derive(Debug, Clone)]
pub struct TaskDeliveryEvidenceObservationCommit {
    pub expected: StoredTaskDeliveryEvidence,
    pub operation: TaskDeliveryEvidenceOperation,
    pub observation: TaskDeliveryEvidenceObservation,
}

#[derive(Debug, Clone)]
pub struct TaskDeliveryEvidenceObservationCommitResult {
    pub updated_queue: PlanningAuthorityDistributorQueueRecord,
    /// The original immutable observation is returned when a duplicate is suppressed.
    pub observation: TaskDeliveryEvidenceObservation,
    pub changed: bool,
    /// A Recheck can only rearm a queue, even when the evidence is satisfied.
    pub effect_allowed: bool,
}

pub trait TaskDeliveryEvidenceRepositoryPort: Send + Sync {
    fn load_delivery_evidence(
        &self,
        _workspace: &str,
        _expected_queue: &PlanningAuthorityDistributorQueueRecord,
        _stage: TaskEvidenceDeliveryStage,
    ) -> Result<StoredTaskDeliveryEvidence> {
        Err(TaskDeliveryEvidenceError::Unavailable.into())
    }

    fn commit_delivery_evidence_observation(
        &self,
        _workspace: &str,
        _commit: &TaskDeliveryEvidenceObservationCommit,
    ) -> Result<TaskDeliveryEvidenceObservationCommitResult> {
        Err(TaskDeliveryEvidenceError::Unavailable.into())
    }
}
