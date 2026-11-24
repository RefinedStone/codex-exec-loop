use super::*;
use crate::application::port::outbound::task_verification_repository_port::*;

fn host_passes(f: &DeliveryFixture) -> TaskDeliveryEvidenceObservation {
    let owner = "e".repeat(64);
    f.base
        .adapter
        .recover_incomplete_host_verifications(&f.base.workspace, &owner, NOW)
        .unwrap();
    let task = f.base.snapshot().tasks.remove(0);
    let mut criteria = Vec::new();
    for criterion in &task.task.acceptance {
        let claim = f
            .base
            .adapter
            .claim_host_verification(
                &f.base.workspace,
                &TaskVerificationClaimCommit {
                    request_id: format!("host-{}", criterion.id),
                    task_id: task.task.id.clone(),
                    criterion_id: criterion.id.clone(),
                    scope_id: task.scope.scope_id.clone(),
                    expected_acceptance_revision: task.scope.acceptance_revision,
                    owner_token: owner.clone(),
                    at: NOW.into(),
                },
            )
            .unwrap();
        let execution = claim.execution.unwrap();
        let mut manifest = super::super::verification::manifest();
        // The host may commit this exact verified tree after the observation.
        manifest.source.head_commit_sha = manifest.base_commit_sha.clone();
        f.base
            .adapter
            .bind_host_verification(
                &f.base.workspace,
                &TaskVerificationBindCommit {
                    execution: execution.clone(),
                    manifest,
                    at: NOW.into(),
                },
            )
            .unwrap();
        let receipt = f
            .base
            .adapter
            .finish_host_verification(
                &f.base.workspace,
                &TaskVerificationFinishCommit {
                    execution,
                    result: super::super::verification::executed(),
                    artifact: Some(TaskEvidenceArtifact {
                        sha256: "c".repeat(64),
                        size_bytes: 100,
                    }),
                    at: NOW.into(),
                },
            )
            .unwrap()
            .value;
        criteria.push(TaskDeliveryCriterionObservation {
            criterion_id: criterion.id.clone(),
            state: TaskDeliveryCriterionState::HostPassed,
            reason: None,
            receipt_event_id: Some(receipt.event_id),
        });
    }
    let mut observation = f.observation(TaskEvidenceDeliveryMode::Enforce);
    observation.criteria = criteria;
    observation.reasons.clear();
    observation
}

#[test]
fn all_essential_host_receipts_are_required_and_worker_failure_vetoes_host_success() {
    let f = DeliveryFixture::new_inner("delivery-evidence-host-proof", true);
    f.claim();
    let approved = host_passes(&f);
    let mut incomplete = approved.clone();
    incomplete.criteria.pop();
    assert!(f.commit(incomplete, f.effect()).is_err());
    assert!(
        f.commit(approved.clone(), f.effect())
            .unwrap()
            .effect_allowed
    );
    let mut prepare = f.base.prepare_commit("reported-failure");
    prepare.prepared_tree = super::super::verification::manifest().source;
    let attempt = f
        .base
        .adapter
        .prepare_attempt(&f.base.workspace, &prepare)
        .unwrap()
        .value;
    let mut record = f.base.record_commit(&attempt);
    record.status = TaskEvidenceStatus::Fail;
    f.base
        .adapter
        .record_evidence(&f.base.workspace, &record)
        .unwrap();
    let mut stale_success = approved;
    stale_success.evidence_revision = f.base.snapshot().evidence_revision;
    assert!(f.commit(stale_success, f.effect()).is_err());
    let human_override = f.waive_all();
    assert!(f.commit(human_override, f.effect()).unwrap().effect_allowed);
}

#[test]
fn queued_source_accepts_the_verified_tree_after_host_commit_without_requiring_old_head_identity() {
    let f = DeliveryFixture::new_inner("delivery-evidence-committed-tree", true);
    f.claim();
    let mut approved = host_passes(&f);
    let mut queue = f.queue();
    // A different integration result is never inferred from patch equivalence.
    queue.integration_base_commit_sha = Some(queue.source_base_commit_sha.clone());
    queue.integration_commit_sha = Some("e".repeat(40));
    f.base
        .adapter
        .upsert_runtime_distributor_queue_record(&f.base.workspace, &queue)
        .unwrap();
    approved.stage = TaskEvidenceDeliveryStage::Integrate;
    approved.source.as_mut().unwrap().integration = Some(TaskEvidenceIntegrationResult {
        base_commit_sha: "b".repeat(40),
        result_commit_sha: "e".repeat(40),
        result_tree_sha: "f".repeat(40),
    });
    assert!(
        f.commit(approved.clone(), f.effect())
            .unwrap()
            .effect_allowed
    );
    approved
        .source
        .as_mut()
        .unwrap()
        .integration
        .as_mut()
        .unwrap()
        .result_tree_sha = "a".repeat(40);
    assert!(f.commit(approved.clone(), f.effect()).is_err());
    approved.reasons = vec![TaskDeliveryEvidenceReason::IntegrationSourceMismatch];
    let blocked = f.commit(approved.clone(), f.effect()).unwrap();
    assert!(!blocked.effect_allowed);
    assert_eq!(blocked.observation.source, approved.source);
    assert_eq!(
        blocked.updated_queue.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert_eq!(blocked.updated_queue.evidence_wait, None);
    assert!(
        blocked
            .updated_queue
            .integration_note
            .contains("operator required"),
        "an immutable source mismatch must not become an automatically rechecked evidence wait"
    );
}

#[test]
fn an_existing_integration_evidence_wait_with_source_mismatch_becomes_an_operator_block() {
    let f = DeliveryFixture::new_inner("delivery-evidence-source-mismatch-recheck", true);
    f.claim();

    let mut queue = f.queue();
    queue.integration_base_commit_sha = Some("c".repeat(40));
    queue.integration_commit_sha = Some("e".repeat(40));
    queue.pull_request_number = Some(123);
    f.base
        .adapter
        .upsert_runtime_distributor_queue_record(&f.base.workspace, &queue)
        .unwrap();

    let mut initial = host_passes(&f);
    initial.stage = TaskEvidenceDeliveryStage::Integrate;
    initial.reasons = vec![TaskDeliveryEvidenceReason::EvidenceMissing];
    initial.source.as_mut().unwrap().integration = Some(TaskEvidenceIntegrationResult {
        base_commit_sha: "c".repeat(40),
        result_commit_sha: "e".repeat(40),
        result_tree_sha: "f".repeat(40),
    });
    let blocked = f.commit(initial, f.effect()).unwrap();
    assert_eq!(
        blocked.updated_queue.evidence_wait,
        Some(TaskEvidenceDeliveryStage::Integrate)
    );
    f.release();

    let mut mismatch = blocked.observation;
    mismatch.reasons = vec![TaskDeliveryEvidenceReason::IntegrationSourceMismatch];
    let result = f
        .commit(mismatch, TaskDeliveryEvidenceOperation::Recheck)
        .unwrap();

    assert!(!result.effect_allowed);
    assert_eq!(
        result.updated_queue.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert_eq!(result.updated_queue.evidence_wait, None);
    assert!(
        result
            .updated_queue
            .integration_note
            .contains("operator required"),
        "rechecking an integration base that moved after dispatch must not rearm the same wait"
    );
}

#[test]
fn worker_reported_pass_cannot_be_presented_as_a_host_receipt() {
    let f = DeliveryFixture::new("delivery-evidence-reported-pass");
    f.claim();
    let attempt = f.base.prepare("reported-only");
    let receipt = f
        .base
        .adapter
        .record_evidence(&f.base.workspace, &f.base.record_commit(&attempt))
        .unwrap()
        .value;
    let mut false_success = f.observation(TaskEvidenceDeliveryMode::Enforce);
    false_success.reasons.clear();
    for criterion in &mut false_success.criteria {
        criterion.state = TaskDeliveryCriterionState::HostPassed;
        criterion.reason = None;
        criterion.receipt_event_id = Some(receipt.event_id.clone());
    }
    assert!(f.commit(false_success, f.effect()).is_err());
    assert_eq!(f.audit_count(), 0);
}
