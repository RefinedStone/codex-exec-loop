use super::*;

#[test]
fn a_source_change_during_seal_verification_cannot_keep_an_earlier_success() {
    let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
    let runtime = fixture.runtime.clone();
    *fixture.artifacts.on_host_read.lock().unwrap() = Some(Box::new(move || {
        runtime
            .trees
            .lock()
            .unwrap()
            .insert("e".repeat(40), "8".repeat(40));
    }));
    assert!(!fixture.effect().unwrap().allowed);
    assert!(
        fixture
            .last()
            .reasons
            .contains(&TaskDeliveryEvidenceReason::EvidenceStale)
    );
}

#[test]
fn a_final_authority_change_cannot_be_committed_as_an_observation_for_the_old_queue() {
    for at_commit in [false, true] {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        let repository = fixture.repository.clone();
        let hook: Hook = Box::new(move || {
            repository.snapshot.lock().unwrap().evidence_revision += 1;
        });
        if at_commit {
            *fixture.repository.on_commit.lock().unwrap() = Some(hook);
        } else {
            *fixture.repository.on_second.lock().unwrap() = Some(hook);
        }
        assert!(fixture.effect().is_err());
        assert!(fixture.repository.observations.lock().unwrap().is_empty());
    }
}

#[test]
fn the_environment_is_reobserved_after_the_last_authority_read_for_all_criteria() {
    for unavailable in [false, true] {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        fixture.change(|snapshot| {
            let task = snapshot.task.as_mut().unwrap();
            let mut criterion = task.task.acceptance[0].clone();
            criterion.id = "second".into();
            task.task.acceptance.push(criterion);
            let mut host = task.host_verifications[0].clone();
            host.attempt.attempt_id = "9".repeat(64);
            host.attempt.request_id = "second-host".into();
            host.attempt.criterion_id = "second".into();
            let receipt = host.receipt.as_mut().unwrap();
            receipt.attempt = host.attempt.clone();
            receipt.event_id = format!("task-verification-finished:{}", host.attempt.attempt_id);
            task.host_verifications.push(host);
        });
        fixture.synchronize_scope();
        let runner = fixture.runner.clone();
        *fixture.repository.on_second.lock().unwrap() = Some(Box::new(move || {
            *runner.current.lock().unwrap() = if unavailable {
                Err(TaskVerificationNotRunReason::Unavailable)
            } else {
                Ok("8".repeat(64))
            };
        }));
        assert!(!fixture.effect().unwrap().allowed);
        let observation = fixture.last();
        assert_eq!(observation.criteria.len(), 2);
        assert!(
            observation
                .criteria
                .iter()
                .all(|row| row.state == TaskDeliveryCriterionState::Blocked)
        );
        assert_eq!(fixture.runner.reads.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn invalid_effective_configuration_is_closed_before_any_evidence_read() {
    let mut fixture = Fixture::new(TaskEvidenceDeliveryMode::Observe);
    fixture.service.mode = Err("invalid or unreadable configuration".into());
    assert!(fixture.effect().is_err());
    assert_eq!(fixture.repository.reads.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.repository.commits.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runtime.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn the_same_path_with_another_or_missing_repository_identity_cannot_reuse_observe_policy() {
    for expected_identity in ["previous-repository-incarnation", ""] {
        let mut fixture = Fixture::new(TaskEvidenceDeliveryMode::Observe);
        fixture.service.dependencies.repository_identity = expected_identity.to_owned();
        // The current authority still resolves the exact same /repo path. Only its
        // incarnation differs from the graph that captured the Observe policy.
        assert!(fixture.effect().is_err());
        assert_eq!(fixture.repository.reads.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.repository.commits.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.runtime.reads.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn rechecking_a_wait_is_never_an_external_effect_authorization() {
    let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
    let queue = fixture.repository.snapshot.lock().unwrap().queue.clone();
    let decision = fixture
        .service
        .evaluate(
            "/repo",
            &queue,
            TaskEvidenceDeliveryStage::Push,
            None,
            &TaskDeliveryEvidenceOperation::Recheck,
        )
        .unwrap();
    assert!(!decision.allowed);
    assert!(fixture.last().satisfied());
}
