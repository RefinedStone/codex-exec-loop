use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::application::port::outbound::task_delivery_evidence_repository_port::*;
use crate::domain::parallel_mode::{
    ParallelModeDeliveryTargetSnapshot, ParallelModeQueueItemState,
    ParallelModeRepositoryVisibility,
};
use crate::domain::planning::delivery_evidence::*;

mod attention;
mod auto_delivery;
mod fences;
mod proof;

struct DeliveryFixture {
    base: Fixture,
    owner: String,
}

impl DeliveryFixture {
    fn new(label: &str) -> Self {
        Self::new_inner(label, false)
    }

    fn new_inner(label: &str, host_recipe: bool) -> Self {
        Self::configured(label, host_recipe, false)
    }

    fn configured(label: &str, host_recipe: bool, screen_only: bool) -> Self {
        Self::configured_before_lease(label, host_recipe, screen_only, |_| {})
    }

    fn configured_before_lease(
        label: &str,
        host_recipe: bool,
        screen_only: bool,
        before_lease: impl FnOnce(&Fixture),
    ) -> Self {
        let base = Fixture::with_before_lease(label, |base| {
            if host_recipe || screen_only {
                let mut task = base.snapshot().tasks[0].task.clone();
                for criterion in &mut task.acceptance {
                    if screen_only {
                        criterion.channel = TaskAcceptanceChannel::Screen;
                        criterion.verification_recipe = None;
                    } else {
                        criterion.verification_recipe = Some(
                            crate::domain::planning::verification::TaskVerificationRecipe::GitDiffCheckV1,
                        );
                    }
                }
                task.acceptance_revision += 1;
                base.save(vec![task]);
            }
            // The first Running lease already carries the source checked by Dispatch authority.
            base.lease =
                base.lease
                    .clone()
                    .with_delivery_target(ParallelModeDeliveryTargetSnapshot::new(
                        "origin",
                        "owner/repo",
                        ParallelModeRepositoryVisibility::Private,
                        "prerelease",
                        "b".repeat(40),
                    ));
            before_lease(base);
        });
        let mut queue = super::super::queue_record_for_task(
            "delivery-queue",
            &base.lease.session_key(),
            &base.lease.task_id,
        );
        queue.task_title = base.lease.task_title.clone();
        queue.source_branch = base.lease.branch_name.clone();
        queue.branch_name = base.lease.branch_name.clone();
        queue.worktree_path = base.lease.worktree_path.clone();
        queue.source_base_commit_sha = "b".repeat(40);
        queue.source_commit_sha = "d".repeat(40);
        queue.commit_sha = queue.source_commit_sha.clone();
        queue.original_commit_sha = Some(queue.source_commit_sha.clone());
        base.adapter
            .upsert_runtime_distributor_queue_record(&base.workspace, &queue)
            .unwrap();
        let fixture = Self {
            base,
            owner: "delivery-fixture-owner".into(),
        };
        assert!(fixture.queue().evidence_binding.is_some());
        fixture
    }

    fn queue(&self) -> PlanningAuthorityDistributorQueueRecord {
        self.base
            .adapter
            .load_runtime_projections(&self.base.workspace)
            .unwrap()
            .distributor_queue_records
            .into_iter()
            .find(|queue| queue.queue_item_id == "delivery-queue")
            .unwrap()
    }

    fn snapshot(&self, stage: TaskEvidenceDeliveryStage) -> StoredTaskDeliveryEvidence {
        self.base
            .adapter
            .load_delivery_evidence(&self.base.workspace, &self.queue(), stage)
            .unwrap()
    }

    fn claim(&self) {
        assert!(
            self.base
                .adapter
                .try_acquire_distributor_queue_claim(
                    &self.base.workspace,
                    "delivery-queue",
                    &self.owner
                )
                .unwrap()
        );
    }

    fn release(&self) {
        self.base
            .adapter
            .release_distributor_queue_claim(&self.base.workspace, "delivery-queue", &self.owner)
            .unwrap();
    }

    fn observation(&self, mode: TaskEvidenceDeliveryMode) -> TaskDeliveryEvidenceObservation {
        let snapshot = self.snapshot(TaskEvidenceDeliveryStage::Push);
        TaskDeliveryEvidenceObservation {
            stage: TaskEvidenceDeliveryStage::Push,
            mode,
            scope: snapshot.task.as_ref().map(|task| task.scope.clone()),
            evidence_revision: snapshot.evidence_revision,
            source: Some(source()),
            criteria: snapshot
                .task
                .as_ref()
                .map(|task| {
                    task.task
                        .acceptance
                        .iter()
                        .filter(|criterion| criterion.essential)
                        .map(|criterion| TaskDeliveryCriterionObservation {
                            criterion_id: criterion.id.clone(),
                            state: TaskDeliveryCriterionState::Blocked,
                            reason: Some(TaskDeliveryEvidenceReason::EvidenceMissing),
                            receipt_event_id: None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            reasons: vec![TaskDeliveryEvidenceReason::EvidenceMissing],
            observed_at: NOW.into(),
        }
    }

    fn commit(
        &self,
        observation: TaskDeliveryEvidenceObservation,
        operation: TaskDeliveryEvidenceOperation,
    ) -> anyhow::Result<TaskDeliveryEvidenceObservationCommitResult> {
        let expected = self.snapshot(observation.stage);
        self.base.adapter.commit_delivery_evidence_observation(
            &self.base.workspace,
            &TaskDeliveryEvidenceObservationCommit {
                expected,
                operation,
                observation,
            },
        )
    }

    fn effect(&self) -> TaskDeliveryEvidenceOperation {
        TaskDeliveryEvidenceOperation::Effect {
            owner_token: self.owner.clone(),
            preflight_permit: self.snapshot(TaskEvidenceDeliveryStage::Push).permit,
        }
    }

    fn waive_all(&self) -> TaskDeliveryEvidenceObservation {
        let task = self.base.snapshot().tasks.remove(0);
        let mut criteria = Vec::new();
        for criterion in task
            .task
            .acceptance
            .iter()
            .filter(|criterion| criterion.essential)
        {
            let receipt = self
                .base
                .adapter
                .waive_criterion(
                    &self.base.workspace,
                    &TaskCriterionWaiverCommit {
                        request_id: format!("waive-{}-{}", task.scope.scope_id, criterion.id),
                        scope: task.scope.clone(),
                        criterion_id: criterion.id.clone(),
                        reason: "Human accepts this criterion for the current scope".into(),
                        confirmation_ref: "native-proof-ref".into(),
                        at: NOW.into(),
                    },
                )
                .unwrap()
                .value;
            criteria.push(TaskDeliveryCriterionObservation {
                criterion_id: criterion.id.clone(),
                state: TaskDeliveryCriterionState::Waived,
                reason: None,
                receipt_event_id: Some(receipt.event_id),
            });
        }
        let mut observation = self.observation(TaskEvidenceDeliveryMode::Enforce);
        observation.criteria = criteria;
        observation.reasons.clear();
        observation
    }

    fn audit_count(&self) -> u64 {
        authority_connection(&self.base.workspace).query_row(
            "SELECT count(*) FROM runtime_events WHERE event_kind='task_delivery_evidence_observed'", [], |row| row.get(0),
        ).unwrap()
    }
}

fn source() -> TaskEvidenceDeliverySource {
    TaskEvidenceDeliverySource {
        base_commit_sha: "b".repeat(40),
        base_tree_sha: "c".repeat(40),
        source_commit_sha: "d".repeat(40),
        source_tree_sha: "f".repeat(40),
        integration: None,
    }
}

#[test]
fn observe_audit_is_stage_deduplicated_without_mutating_evidence_or_delivery_authority() {
    let f = DeliveryFixture::new("delivery-evidence-observe");
    f.claim();
    let before = f.base.counters();
    let first = f
        .commit(f.observation(TaskEvidenceDeliveryMode::Observe), f.effect())
        .unwrap();
    assert!(first.effect_allowed && first.changed);
    assert_eq!(
        first.updated_queue.queue_state,
        ParallelModeQueueItemState::Queued
    );
    assert_eq!(first.updated_queue.evidence_wait, None);
    assert_eq!(f.base.counters(), before);
    let mut same = f.observation(TaskEvidenceDeliveryMode::Observe);
    same.observed_at = "2026-11-23T13:00:00Z".into();
    let repeated = f.commit(same, f.effect()).unwrap();
    assert!(!repeated.changed);
    assert_eq!(repeated.observation, first.observation);
    assert_eq!(f.audit_count(), 1);
    let mut pr = f.observation(TaskEvidenceDeliveryMode::Observe);
    pr.stage = TaskEvidenceDeliveryStage::PullRequest;
    f.commit(pr, f.effect()).unwrap();
    assert_eq!(f.audit_count(), 2);
    assert_eq!(f.queue().evidence_observations.len(), 2);
    assert_eq!(f.base.counters(), before);
}

#[test]
fn enforce_wait_rechecks_only_its_typed_stage_and_never_returns_effect_authority() {
    let f = DeliveryFixture::new("delivery-evidence-rearm");
    f.claim();
    let blocked = f
        .commit(f.observation(TaskEvidenceDeliveryMode::Enforce), f.effect())
        .unwrap();
    assert!(!blocked.effect_allowed);
    assert_eq!(
        blocked.updated_queue.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert_eq!(
        blocked.updated_queue.evidence_wait,
        Some(TaskEvidenceDeliveryStage::Push)
    );
    let success = f.waive_all();
    assert!(
        f.commit(success.clone(), TaskDeliveryEvidenceOperation::Recheck)
            .is_err(),
        "an active distributor owns the row"
    );
    f.release();
    let ready = f
        .commit(success, TaskDeliveryEvidenceOperation::Recheck)
        .unwrap();
    assert!(!ready.effect_allowed);
    assert_eq!(
        ready.updated_queue.queue_state,
        ParallelModeQueueItemState::Queued
    );
    assert_eq!(ready.updated_queue.evidence_wait, None);
    f.claim();
    let passed = f.waive_all();
    assert!(f.commit(passed, f.effect()).unwrap().effect_allowed);
}

#[test]
fn ordinary_queue_updates_preserve_audits_and_cannot_rearm_an_operator_block() {
    let f = DeliveryFixture::new("delivery-evidence-operator-block");
    f.claim();
    f.commit(f.observation(TaskEvidenceDeliveryMode::Enforce), f.effect())
        .unwrap();
    f.release();
    let mut operator = f.queue();
    let binding = operator.evidence_binding.clone();
    operator.integration_note = "operator must resolve the conflict".into();
    operator.evidence_binding = None;
    operator.evidence_observations.clear();
    f.base
        .adapter
        .upsert_runtime_distributor_queue_record(&f.base.workspace, &operator)
        .unwrap();
    let stored = f.queue();
    assert_eq!(stored.evidence_binding, binding);
    assert_eq!(stored.evidence_wait, None);
    assert_eq!(stored.evidence_observations.len(), 1);
    assert!(
        f.commit(
            f.observation(TaskEvidenceDeliveryMode::Observe),
            TaskDeliveryEvidenceOperation::Recheck
        )
        .is_err()
    );
    assert_eq!(f.queue(), stored);
}

#[test]
fn legacy_queue_without_current_task_is_observable_but_cannot_pass_enforcement() {
    let workspace = temp_workspace("delivery-evidence-missing-task");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let queue =
        super::super::queue_record_for_task("legacy-queue", "legacy-session", "missing-task");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace, &queue)
        .unwrap();
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace, &queue.queue_item_id, "legacy-owner")
            .unwrap()
    );
    let mut expected = adapter
        .load_delivery_evidence(&workspace, &queue, TaskEvidenceDeliveryStage::Push)
        .unwrap();
    assert!(expected.task.is_none());
    assert!(expected.queue.evidence_binding.is_none());
    for mode in [
        TaskEvidenceDeliveryMode::Observe,
        TaskEvidenceDeliveryMode::Enforce,
    ] {
        let observation = TaskDeliveryEvidenceObservation {
            stage: TaskEvidenceDeliveryStage::Push,
            mode,
            scope: None,
            evidence_revision: expected.evidence_revision,
            source: None,
            criteria: Vec::new(),
            reasons: vec![TaskDeliveryEvidenceReason::CriteriaMissing],
            observed_at: NOW.into(),
        };
        let committed = adapter
            .commit_delivery_evidence_observation(
                &workspace,
                &TaskDeliveryEvidenceObservationCommit {
                    expected: expected.clone(),
                    operation: TaskDeliveryEvidenceOperation::Effect {
                        owner_token: "legacy-owner".into(),
                        preflight_permit: expected.permit,
                    },
                    observation,
                },
            )
            .unwrap();
        assert_eq!(
            committed.effect_allowed,
            mode == TaskEvidenceDeliveryMode::Observe
        );
        expected = adapter
            .load_delivery_evidence(
                &workspace,
                &committed.updated_queue,
                TaskEvidenceDeliveryStage::Push,
            )
            .unwrap();
    }
    assert_eq!(
        expected.queue.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert_eq!(
        expected.queue.evidence_wait,
        Some(TaskEvidenceDeliveryStage::Push)
    );
}
