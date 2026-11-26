use super::*;
use crate::application::port::outbound::github_automation_port::GithubRepositoryVisibility;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorDeliveryTarget;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityMutationAudit;
use crate::application::port::outbound::planning_work_order_repository_port::*;
use crate::domain::planning::{OriginSessionKind, PriorityQueueService, TaskMutationProvenance};

fn automatic_fixture(label: &str, host_recipe: bool, screen_only: bool) -> DeliveryFixture {
    DeliveryFixture::configured_before_lease(label, host_recipe, screen_only, register_auto_work)
}

fn register_auto_work(base: &Fixture) {
    let adapter = &base.adapter;
    let workspace = &base.workspace;
    let runtime = adapter.load_runtime_projections(workspace).unwrap();
    assert!(runtime.slot_leases.is_empty());
    assert!(runtime.distributor_queue_records.is_empty());
    let before = adapter
        .load_task_authority_snapshot(workspace)
        .unwrap()
        .unwrap();
    let mut document = before.task_authority.clone();
    document
        .tasks
        .push(authority_task("z-order-root", "direction-1"));
    let projection = PriorityQueueService::new()
        .build_projection(&test_direction_catalog(&["direction-1"]), &document)
        .unwrap();
    let task_ids = ["z-order-root".to_string()];
    let provenance = TaskMutationProvenance::new(OriginSessionKind::System);
    assert!(matches!(
        adapter
            .register_work_order(
                workspace,
                PlanningWorkOrderRegistration {
                    acceptance_receipt: None,
                    work_order_id: "auto-order",
                    origin: "issue_automation",
                    link_reference: None,
                    initial_task_id: "z-order-root",
                    observed_planning_revision: before.planning_revision,
                    task_authority: &document,
                    queue_projection: &projection,
                    audit: PlanningTaskAuthorityMutationAudit {
                        task_ids: &task_ids,
                        legacy_source_turn_id: None,
                        provenance: &provenance
                    },
                }
            )
            .unwrap(),
        PlanningWorkOrderRegistrationResult::Registered { .. }
    ));
    assert!(
        adapter
            .link_supplemental_task(workspace, "auto-order", &base.lease.task_id)
            .unwrap()
            .linked
    );
    grant_dispatch(base);
}

fn grant_dispatch(base: &Fixture) {
    let adapter = &base.adapter;
    let workspace = &base.workspace;
    // Work lineage and the frozen-base Dispatch authority precede every execution source.
    let basis = PlanningWorkPermitBasis {
        authorization_epoch: 0,
        policy_version: "policy-1".into(),
        field_hash: "field-1".into(),
        gate_revision: 1,
        source_sha: base
            .lease
            .delivery_target
            .as_ref()
            .expect("the delivery target must precede the first Dispatch grant")
            .integration_base_commit_sha
            .clone(),
        delivery_review: None,
    };
    adapter
        .set_work_order_permit_basis(workspace, "auto-order", &basis)
        .unwrap();
    adapter
        .grant_work_order_permit(
            workspace,
            &PlanningWorkPermit {
                work_order_id: "auto-order".into(),
                stage: PlanningWorkPermitStage::Dispatch,
                basis,
                requires_human_review: false,
            },
        )
        .unwrap();
}

fn install_auto(f: &DeliveryFixture) -> PlanningWorkPermitBasis {
    let adapter = &f.base.adapter;
    let workspace = &f.base.workspace;
    assert_eq!(
        adapter
            .load_work_order_for_task(workspace, &f.base.lease.task_id)
            .unwrap()
            .unwrap()
            .work_order_id,
        "auto-order"
    );
    let mut queue = f.queue();
    let target = f.base.lease.delivery_target.as_ref().unwrap();
    queue.delivery_target = Some(PlanningAuthorityDistributorDeliveryTarget::new(
        &target.push_remote,
        &target.github_repository,
        GithubRepositoryVisibility::Private,
        &target.integration_branch,
    ));
    adapter
        .upsert_runtime_distributor_queue_record(workspace, &queue)
        .unwrap();
    let basis = PlanningWorkPermitBasis {
        authorization_epoch: 0,
        policy_version: "policy-1".into(),
        field_hash: "field-1".into(),
        gate_revision: 2,
        source_sha: queue.source_commit_sha.clone(),
        delivery_review: Some(PlanningWorkDeliveryReview {
            base_commit_sha: queue.source_base_commit_sha,
            source_commit_sha: queue.source_commit_sha,
            gate_revision: 2,
            authority: Some(PlanningWorkDeliveryAuthority::Auto {
                merge_gate_revision: 7,
                source_repository: "owner/repo".into(),
            }),
        }),
    };
    grant(f, &basis);
    basis
}

fn grant(f: &DeliveryFixture, basis: &PlanningWorkPermitBasis) {
    f.base
        .adapter
        .set_work_order_permit_basis(&f.base.workspace, "auto-order", basis)
        .unwrap();
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
        PlanningWorkPermitStage::Integrate,
    ] {
        f.base
            .adapter
            .grant_work_order_permit(
                &f.base.workspace,
                &PlanningWorkPermit {
                    work_order_id: "auto-order".into(),
                    stage,
                    basis: basis.clone(),
                    requires_human_review: false,
                },
            )
            .unwrap();
    }
}

fn with_policy(
    f: &DeliveryFixture,
    mut observation: TaskDeliveryEvidenceObservation,
) -> TaskDeliveryEvidenceObservation {
    let reasons = f
        .snapshot(observation.stage)
        .auto_policy_reasons(observation.mode);
    observation.reasons.splice(0..0, reasons);
    observation
}

#[test]
fn auto_observe_is_a_durable_wait_and_reopening_cannot_rearm_even_satisfied_evidence() {
    let f = automatic_fixture("auto-delivery-observe-reopen", true, false);
    let mut passed = proof::host_passes(&f);
    install_auto(&f);
    passed.evidence_revision = f.base.snapshot().evidence_revision;
    f.claim();
    let evidence_before = f.base.counters();
    let mut observe = passed.clone();
    observe.mode = TaskEvidenceDeliveryMode::Observe;
    // Omitting the explicit policy denial cannot exploit Observe's legacy permissive behavior.
    assert!(f.commit(observe.clone(), f.effect()).is_err());
    assert_eq!(f.audit_count(), 0);
    observe = with_policy(&f, observe);
    let held = f.commit(observe.clone(), f.effect()).unwrap();
    assert!(!held.effect_allowed);
    assert!(!held.observation.allows_delivery());
    assert_eq!(
        held.updated_queue.evidence_wait,
        Some(TaskEvidenceDeliveryStage::Push)
    );
    assert_eq!(
        held.updated_queue.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    f.release();
    let reopened = SqlitePlanningAuthorityAdapter::new();
    let expected = reopened
        .load_delivery_evidence(
            &f.base.workspace,
            &held.updated_queue,
            TaskEvidenceDeliveryStage::Push,
        )
        .unwrap();
    assert_eq!(expected.permit, PlanningWorkPermitDecision::GrantedAuto);
    let repeated = reopened
        .commit_delivery_evidence_observation(
            &f.base.workspace,
            &TaskDeliveryEvidenceObservationCommit {
                expected,
                operation: TaskDeliveryEvidenceOperation::Recheck,
                observation: observe,
            },
        )
        .unwrap();
    assert!(!repeated.effect_allowed && !repeated.changed);
    assert_eq!(repeated.updated_queue, held.updated_queue);
    assert_eq!(f.audit_count(), 1);
    assert_eq!(f.base.counters(), evidence_before);
    let rearmed = f
        .commit(passed.clone(), TaskDeliveryEvidenceOperation::Recheck)
        .unwrap();
    assert!(!rearmed.effect_allowed);
    assert_eq!(
        rearmed.updated_queue.queue_state,
        ParallelModeQueueItemState::Queued
    );
    f.claim();
    assert!(f.commit(passed, f.effect()).unwrap().effect_allowed);
}

#[test]
fn auto_enforce_missing_and_reported_only_results_stay_held_until_host_or_human_evidence() {
    let f = automatic_fixture("auto-delivery-reported-only", true, false);
    let attempt = f.base.prepare("reported-only");
    f.base
        .adapter
        .record_evidence(&f.base.workspace, &f.base.record_commit(&attempt))
        .unwrap();
    install_auto(&f);
    f.claim();
    let missing = f.observation(TaskEvidenceDeliveryMode::Enforce);
    let held = f.commit(missing.clone(), f.effect()).unwrap();
    assert!(!held.effect_allowed);
    f.release();
    let still = f
        .commit(missing, TaskDeliveryEvidenceOperation::Recheck)
        .unwrap();
    assert_eq!(
        still.updated_queue.evidence_wait,
        Some(TaskEvidenceDeliveryStage::Push)
    );
    assert!(!still.effect_allowed);
    let waived = f.waive_all();
    assert_eq!(
        f.commit(waived.clone(), TaskDeliveryEvidenceOperation::Recheck)
            .unwrap()
            .updated_queue
            .queue_state,
        ParallelModeQueueItemState::Queued
    );
    f.claim();
    assert!(f.commit(waived, f.effect()).unwrap().effect_allowed);
}

#[test]
fn auto_cannot_promote_all_screen_or_unsupported_command_waivers_to_delivery_authority() {
    for screen in [false, true] {
        let f = automatic_fixture(
            &format!("auto-delivery-manual-only-{screen}"),
            false,
            screen,
        );
        assert_eq!(
            f.queue().evidence_binding.as_ref().unwrap().scope,
            f.base.snapshot().tasks[0].scope
        );
        let mut waived = f.waive_all();
        install_auto(&f);
        waived.evidence_revision = f.base.snapshot().evidence_revision;
        f.claim();
        assert!(f.commit(waived.clone(), f.effect()).is_err());
        let blocked = with_policy(&f, waived);
        let held = f.commit(blocked.clone(), f.effect()).unwrap();
        assert_eq!(
            held.observation.reasons,
            vec![TaskDeliveryEvidenceReason::AutoRequiresMachineCriterion]
        );
        assert!(!held.effect_allowed && !held.observation.allows_delivery());
        f.release();
        let repeated = f
            .commit(blocked, TaskDeliveryEvidenceOperation::Recheck)
            .unwrap();
        assert_eq!(
            repeated.updated_queue.queue_state,
            ParallelModeQueueItemState::Blocked
        );
        assert!(!repeated.effect_allowed);
    }
}

#[test]
fn auto_mixed_case_frozen_target_survives_load_commit_and_reopen() {
    let f = automatic_fixture("auto-delivery-target-case", true, false);
    let mut passed = proof::host_passes(&f);
    let basis = install_auto(&f);
    passed.evidence_revision = f.base.snapshot().evidence_revision;
    let mut queue = f.queue();
    queue.delivery_target.as_mut().unwrap().github_repository = "Owner/Repo".into();
    let original_target = queue.delivery_target.clone();
    f.base
        .adapter
        .upsert_runtime_distributor_queue_record(&f.base.workspace, &queue)
        .unwrap();
    f.claim();
    let stored = f.snapshot(TaskEvidenceDeliveryStage::Push);
    assert_eq!(stored.queue.delivery_target, original_target);
    assert_eq!(stored.permit_basis, Some(basis.clone()));
    let committed = f.commit(passed, f.effect()).unwrap();
    assert!(committed.effect_allowed);
    assert_eq!(committed.updated_queue.delivery_target, original_target);
    let reopened = SqlitePlanningAuthorityAdapter::new()
        .load_delivery_evidence(
            &f.base.workspace,
            &committed.updated_queue,
            TaskEvidenceDeliveryStage::Push,
        )
        .unwrap();
    assert_eq!(reopened.queue.delivery_target, original_target);
    assert_eq!(reopened.permit_basis, Some(basis));
    assert_eq!(f.audit_count(), 1);
}

#[test]
fn auto_frozen_repository_and_visibility_are_checked_again_in_final_transaction() {
    for change in ["repository", "rename", "public", "missing"] {
        let f = automatic_fixture(&format!("auto-delivery-target-{change}"), true, false);
        let mut passed = proof::host_passes(&f);
        install_auto(&f);
        passed.evidence_revision = f.base.snapshot().evidence_revision;
        f.claim();
        let expected = f.snapshot(TaskEvidenceDeliveryStage::Push);
        let operation = f.effect();
        let mut changed = f.queue();
        match change {
            "repository" => {
                changed.delivery_target.as_mut().unwrap().github_repository = "Other/Repo".into()
            }
            "rename" => {
                changed.delivery_target.as_mut().unwrap().github_repository = "Owner/Renamed".into()
            }
            "public" => {
                changed
                    .delivery_target
                    .as_mut()
                    .unwrap()
                    .repository_visibility = GithubRepositoryVisibility::Public
            }
            "missing" => changed.delivery_target = None,
            _ => unreachable!(),
        }
        f.base
            .adapter
            .upsert_runtime_distributor_queue_record(&f.base.workspace, &changed)
            .unwrap();
        assert!(
            f.base
                .adapter
                .commit_delivery_evidence_observation(
                    &f.base.workspace,
                    &TaskDeliveryEvidenceObservationCommit {
                        expected,
                        operation,
                        observation: passed
                    }
                )
                .is_err()
        );
        assert!(
            f.base
                .adapter
                .load_delivery_evidence(
                    &f.base.workspace,
                    &f.queue(),
                    TaskEvidenceDeliveryStage::Push
                )
                .is_err()
        );
        assert_eq!(f.audit_count(), 0);
    }
}

#[test]
fn auto_merge_authority_is_cas_bound_even_when_the_decision_and_evidence_revision_are_unchanged() {
    let f = automatic_fixture("auto-delivery-merge-cas", true, false);
    let mut passed = proof::host_passes(&f);
    let mut basis = install_auto(&f);
    passed.evidence_revision = f.base.snapshot().evidence_revision;
    f.claim();
    let old = f.snapshot(TaskEvidenceDeliveryStage::Push);
    let operation = f.effect();
    basis.gate_revision += 1;
    let review = basis.delivery_review.as_mut().unwrap();
    review.gate_revision = basis.gate_revision;
    let Some(PlanningWorkDeliveryAuthority::Auto {
        merge_gate_revision,
        ..
    }) = &mut review.authority
    else {
        panic!("Auto fixture")
    };
    *merge_gate_revision += 1;
    grant(&f, &basis);
    let current = f.snapshot(TaskEvidenceDeliveryStage::Push);
    assert_eq!(old.permit, current.permit);
    assert_eq!(old.evidence_revision, current.evidence_revision);
    assert_ne!(old.permit_basis, current.permit_basis);
    assert!(
        f.base
            .adapter
            .commit_delivery_evidence_observation(
                &f.base.workspace,
                &TaskDeliveryEvidenceObservationCommit {
                    expected: old,
                    operation,
                    observation: passed
                }
            )
            .is_err()
    );
    assert_eq!(f.audit_count(), 0);
}

#[test]
fn auto_effect_rejects_a_legacy_or_human_preflight_even_with_the_current_claim() {
    let f = automatic_fixture("auto-delivery-preflight-cas", true, false);
    let mut passed = proof::host_passes(&f);
    install_auto(&f);
    passed.evidence_revision = f.base.snapshot().evidence_revision;
    f.claim();
    for preflight_permit in [
        PlanningWorkPermitDecision::Legacy,
        PlanningWorkPermitDecision::Granted {
            requires_human_review: false,
        },
        PlanningWorkPermitDecision::Granted {
            requires_human_review: true,
        },
    ] {
        assert!(
            f.commit(
                passed.clone(),
                TaskDeliveryEvidenceOperation::Effect {
                    owner_token: f.owner.clone(),
                    preflight_permit
                }
            )
            .is_err()
        );
    }
    assert_eq!(f.audit_count(), 0);
    assert!(f.commit(passed, f.effect()).unwrap().effect_allowed);
}

#[test]
fn current_reported_fail_or_blocked_vetoes_an_older_host_pass_under_auto() {
    for status in [TaskEvidenceStatus::Fail, TaskEvidenceStatus::Blocked] {
        let f = automatic_fixture(&format!("auto-delivery-veto-{status:?}"), true, false);
        let mut passed = proof::host_passes(&f);
        let mut prepare = f.base.prepare_commit("later-report");
        prepare.prepared_tree = super::super::verification::manifest().source;
        let attempt = f
            .base
            .adapter
            .prepare_attempt(&f.base.workspace, &prepare)
            .unwrap()
            .value;
        let mut record = f.base.record_commit(&attempt);
        record.status = status;
        f.base
            .adapter
            .record_evidence(&f.base.workspace, &record)
            .unwrap();
        install_auto(&f);
        f.claim();
        passed.evidence_revision = f.base.snapshot().evidence_revision;
        assert!(f.commit(passed, f.effect()).is_err());
        assert_eq!(f.audit_count(), 0);
        let blocked = f
            .commit(f.observation(TaskEvidenceDeliveryMode::Enforce), f.effect())
            .unwrap();
        assert!(!blocked.effect_allowed);
        assert_eq!(
            blocked.updated_queue.evidence_wait,
            Some(TaskEvidenceDeliveryStage::Push)
        );
    }
}
