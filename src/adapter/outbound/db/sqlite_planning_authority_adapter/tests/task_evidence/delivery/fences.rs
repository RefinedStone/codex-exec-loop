use super::*;

#[test]
fn effect_requires_the_current_unexpired_claim_even_in_observe_mode() {
    for state in ["missing", "wrong-owner", "expired"] {
        let f = DeliveryFixture::new(&format!("delivery-evidence-claim-{state}"));
        if state != "missing" {
            f.claim();
        }
        if state == "expired" {
            authority_connection(&f.base.workspace).execute(
                "UPDATE runtime_claims SET claimed_at='2000-01-01T00:00:00Z' WHERE scope_key='delivery-queue'", [],
            ).unwrap();
        }
        let operation = if state == "wrong-owner" {
            TaskDeliveryEvidenceOperation::Effect {
                owner_token: "unrelated-owner".into(),
                preflight_permit: f.snapshot(TaskEvidenceDeliveryStage::Push).permit,
            }
        } else {
            f.effect()
        };
        let error = f
            .commit(f.observation(TaskEvidenceDeliveryMode::Observe), operation)
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TaskDeliveryEvidenceError>(),
            Some(&TaskDeliveryEvidenceError::ClaimLost)
        );
        assert_eq!(f.audit_count(), 0);
    }
}

#[test]
fn final_observation_cas_rejects_scope_evidence_lease_and_queue_changes() {
    for change in ["scope", "evidence", "lease", "queue"] {
        let f = DeliveryFixture::new(&format!("delivery-evidence-cas-{change}"));
        f.claim();
        let observation = f.observation(TaskEvidenceDeliveryMode::Observe);
        let commit = TaskDeliveryEvidenceObservationCommit {
            expected: f.snapshot(observation.stage),
            observation,
            operation: f.effect(),
        };
        match change {
            "scope" => {
                let mut task = f.base.snapshot().tasks[0].task.clone();
                task.acceptance[0].scenario = "Changed current acceptance".into();
                task.acceptance_revision += 1;
                f.base.save(vec![task]);
            }
            "evidence" => {
                f.base.prepare("new-evidence-attempt");
            }
            "lease" => {
                let mut lease = f.base.lease.clone();
                lease.lease_generation = Some("c".repeat(64));
                f.base
                    .adapter
                    .upsert_runtime_slot_lease(&f.base.workspace, &lease)
                    .unwrap();
            }
            "queue" => {
                let mut queue = f.queue();
                queue.integration_note = "a newer operator observation".into();
                f.base
                    .adapter
                    .upsert_runtime_distributor_queue_record(&f.base.workspace, &queue)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = f.queue();
        let error = f
            .base
            .adapter
            .commit_delivery_evidence_observation(&f.base.workspace, &commit)
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TaskDeliveryEvidenceError>(),
            Some(&TaskDeliveryEvidenceError::Conflict),
            "{change}"
        );
        assert_eq!(f.queue(), before);
        assert_eq!(f.audit_count(), 0);
    }
}

#[test]
fn legacy_binding_cannot_be_backfilled_by_a_general_upsert_or_waiver() {
    let f = DeliveryFixture::new("delivery-evidence-legacy");
    let original = f.queue();
    let mut old = serde_json::to_value(&original).unwrap();
    old.as_object_mut().unwrap().remove("evidence_binding");
    authority_connection(&f.base.workspace)
        .execute(
            "UPDATE runtime_distributor_queue SET content=?1 WHERE queue_item_id='delivery-queue'",
            [serde_json::to_string(&old).unwrap()],
        )
        .unwrap();
    let legacy = f.queue();
    assert!(legacy.evidence_binding.is_none());
    f.base
        .adapter
        .upsert_runtime_distributor_queue_record(&f.base.workspace, &legacy)
        .unwrap();
    assert!(f.queue().evidence_binding.is_none());
    assert!(
        f.base
            .adapter
            .upsert_runtime_distributor_queue_record(&f.base.workspace, &original)
            .is_err()
    );
    f.claim();
    let approved = f.waive_all();
    assert!(
        f.commit(approved, f.effect()).is_err(),
        "new waiver does not prove old queue origin"
    );
    let mut rejected = f.observation(TaskEvidenceDeliveryMode::Enforce);
    rejected.reasons = vec![TaskDeliveryEvidenceReason::EvidenceStale];
    for criterion in &mut rejected.criteria {
        criterion.reason = Some(TaskDeliveryEvidenceReason::EvidenceStale);
    }
    assert!(!f.commit(rejected, f.effect()).unwrap().effect_allowed);
}

#[test]
fn queue_binding_survives_scope_changes_and_prevents_delete_recreate_aba() {
    for recreate in [false, true] {
        let f = DeliveryFixture::new(&format!("delivery-evidence-scope-aba-{recreate}"));
        let old = f.queue();
        let mut task = f.base.snapshot().tasks[0].task.clone();
        if recreate {
            f.base.save(Vec::new());
        } else {
            task.acceptance_revision += 1;
            task.acceptance[0].scenario = "Updated criterion".into();
        }
        f.base.save(vec![task]);
        let current = f.base.snapshot().tasks[0].scope.clone();
        assert_ne!(
            Some(&current),
            old.evidence_binding.as_ref().map(|binding| &binding.scope)
        );
        let records = f
            .base
            .adapter
            .load_runtime_projections(&f.base.workspace)
            .unwrap()
            .distributor_queue_records;
        if records.is_empty() {
            assert!(
                f.base
                    .adapter
                    .upsert_runtime_distributor_queue_record(&f.base.workspace, &old)
                    .is_err()
            );
        } else {
            assert_eq!(f.queue().evidence_binding, old.evidence_binding);
            f.claim();
            let new_waiver = f.waive_all();
            assert!(f.commit(new_waiver, f.effect()).is_err());
        }
    }
}

#[test]
fn awaiting_user_cannot_be_overridden_by_observe() {
    let f = DeliveryFixture::new("delivery-evidence-control-awaiting-user");
    f.claim();
    let mut task = f.base.snapshot().tasks[0].task.clone();
    task.status = TaskStatus::AwaitingUser;
    f.base.save(vec![task]);
    assert_eq!(
        f.base.snapshot().tasks[0].task.status,
        TaskStatus::AwaitingUser
    );
    let before = f.queue();
    let error = f
        .commit(f.observation(TaskEvidenceDeliveryMode::Observe), f.effect())
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskDeliveryEvidenceError>(),
        Some(&TaskDeliveryEvidenceError::ControlDenied)
    );
    assert_eq!(f.queue(), before);
    assert_eq!(f.audit_count(), 0);
}

#[test]
fn admin_mutation_after_runtime_reset_rejects_late_observe_and_queue_recreation() {
    let f = DeliveryFixture::new("delivery-evidence-control-admin-reset");
    f.claim();
    let observation = f.observation(TaskEvidenceDeliveryMode::Observe);
    let commit = TaskDeliveryEvidenceObservationCommit {
        expected: f.snapshot(observation.stage),
        observation,
        operation: f.effect(),
    };
    let tasks = vec![f.base.lease.task_id.clone()];
    // Admin editing first stops runtime ownership; it cannot take a guard over an active lease
    // or queue. A delivery observer may still return afterward with its captured snapshot.
    f.base
        .adapter
        .clear_parallel_runtime_projections_for_tasks(
            &f.base.workspace,
            &tasks,
            "operator stopped runtime before editing",
        )
        .unwrap();
    f.base
        .adapter
        .acquire_admin_task_mutation_guard(&f.base.workspace, &tasks, "admin-owner")
        .unwrap();

    let error = f
        .base
        .adapter
        .commit_delivery_evidence_observation(&f.base.workspace, &commit)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskDeliveryEvidenceError>(),
        Some(&TaskDeliveryEvidenceError::Missing)
    );
    let recreate = f
        .base
        .adapter
        .upsert_runtime_distributor_queue_record(&f.base.workspace, &commit.expected.queue)
        .unwrap_err();
    assert!(recreate.to_string().contains("admin mutation guard"));
    assert!(
        f.base
            .adapter
            .load_runtime_projections(&f.base.workspace)
            .unwrap()
            .distributor_queue_records
            .is_empty()
    );
    assert_eq!(f.audit_count(), 0);
}
