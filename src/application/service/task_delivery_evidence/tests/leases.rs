use super::*;

#[derive(Clone, Copy, Debug)]
enum LeaseDrift {
    Generation,
    Slot,
    LegacyGeneration,
}

fn replace_current_lease(fixture: &Fixture, drift: LeaseDrift) {
    fixture.change(|snapshot| {
        let task = snapshot.task.as_mut().unwrap();
        // Keep the frozen queue, binding, source and original host receipt intact.
        // Only the current lease projection belongs to another execution.
        let mut lease = task.host_verifications[0].lease.clone();
        match drift {
            LeaseDrift::Generation => lease.lease_generation = Some("9".repeat(64)),
            LeaseDrift::Slot => lease.slot_id = "another-slot".into(),
            LeaseDrift::LegacyGeneration => lease.lease_generation = None,
        }
        task.leases = vec![lease];
    });
}

#[test]
fn current_lease_drift_is_a_stale_audit_in_observe_and_a_hold_in_enforce() {
    for mode in [
        TaskEvidenceDeliveryMode::Observe,
        TaskEvidenceDeliveryMode::Enforce,
    ] {
        for drift in [
            LeaseDrift::Generation,
            LeaseDrift::Slot,
            LeaseDrift::LegacyGeneration,
        ] {
            let fixture = Fixture::new(mode);
            let before = fixture.repository.snapshot.lock().unwrap().clone();
            replace_current_lease(&fixture, drift);

            let decision = fixture.effect().unwrap();
            assert_eq!(
                decision.allowed,
                mode == TaskEvidenceDeliveryMode::Observe,
                "{drift:?}"
            );
            let observation = fixture.last();
            assert!(!observation.satisfied(), "{drift:?}");
            assert_eq!(
                observation.reasons,
                vec![TaskDeliveryEvidenceReason::EvidenceStale]
            );
            assert_eq!(
                observation.criteria[0].state,
                TaskDeliveryCriterionState::HostPassed
            );
            assert_eq!(
                observation.source.as_ref().unwrap().source_commit_sha,
                before.queue.source_commit_sha
            );
            let after = fixture.repository.snapshot.lock().unwrap();
            assert_eq!(after.queue, before.queue);
            assert_eq!(
                after.task.as_ref().unwrap().host_verifications,
                before.task.as_ref().unwrap().host_verifications
            );
        }
    }
}

#[test]
fn a_human_criterion_waiver_does_not_override_current_lease_identity() {
    for mode in [
        TaskEvidenceDeliveryMode::Observe,
        TaskEvidenceDeliveryMode::Enforce,
    ] {
        let fixture = Fixture::new(mode);
        replace_current_lease(&fixture, LeaseDrift::Generation);
        fixture.waive();

        assert_eq!(
            fixture.effect().unwrap().allowed,
            mode == TaskEvidenceDeliveryMode::Observe
        );
        let observation = fixture.last();
        assert!(!observation.satisfied());
        assert_eq!(
            observation.reasons,
            vec![TaskDeliveryEvidenceReason::EvidenceStale]
        );
        assert_eq!(
            observation.criteria[0].state,
            TaskDeliveryCriterionState::Waived
        );
        assert_eq!(fixture.runner.reads.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.artifacts.host_reads.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn a_matching_current_lease_or_completed_lease_cleanup_preserves_valid_host_evidence() {
    for retain_lease in [false, true] {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        if retain_lease {
            fixture.change(|snapshot| {
                let task = snapshot.task.as_mut().unwrap();
                task.leases = vec![task.host_verifications[0].lease.clone()];
            });
        }

        assert!(fixture.effect().unwrap().allowed);
        assert!(fixture.last().satisfied());
        assert!(fixture.last().reasons.is_empty());
    }
}
