use super::*;

fn empty(mode: TaskEvidenceDeliveryMode) -> TaskDeliveryEvidenceObservation {
    TaskDeliveryEvidenceObservation {
        stage: TaskEvidenceDeliveryStage::Push,
        mode,
        scope: None,
        evidence_revision: 0,
        source: None,
        criteria: Vec::new(),
        reasons: vec![TaskDeliveryEvidenceReason::CriteriaMissing],
        observed_at: "2026-11-23T12:00:00Z".into(),
    }
}

#[test]
fn mode_defaults_to_observe_and_only_accepts_the_two_exact_settings() {
    assert_eq!(
        TaskEvidenceDeliveryMode::default(),
        TaskEvidenceDeliveryMode::Observe
    );
    assert_eq!(
        TaskEvidenceDeliveryMode::parse("enforce").unwrap(),
        TaskEvidenceDeliveryMode::Enforce
    );
    for invalid in ["off", "ON", "Observe", " enforce", "enforce ", ""] {
        assert!(TaskEvidenceDeliveryMode::parse(invalid).is_err());
    }
}

#[test]
fn empty_essential_criteria_are_never_vacuously_satisfied() {
    for mode in [
        TaskEvidenceDeliveryMode::Observe,
        TaskEvidenceDeliveryMode::Enforce,
    ] {
        let observation = empty(mode);
        observation.validate().unwrap();
        assert!(!observation.satisfied());
        assert_eq!(
            observation.allows_delivery(),
            mode == TaskEvidenceDeliveryMode::Observe
        );
    }
    let mut malformed = empty(TaskEvidenceDeliveryMode::Enforce);
    malformed.reasons.clear();
    assert!(malformed.validate().is_err());
}

#[test]
fn observations_dedupe_only_when_the_entire_assessment_basis_is_unchanged() {
    let first = empty(TaskEvidenceDeliveryMode::Observe);
    let mut second = first.clone();
    second.observed_at = "2026-11-23T12:05:00Z".into();
    assert!(first.same_assessment_as(&second));
    second.evidence_revision += 1;
    assert!(!first.same_assessment_as(&second));
    second = first.clone();
    second.mode = TaskEvidenceDeliveryMode::Enforce;
    assert!(!first.same_assessment_as(&second));
    second = first.clone();
    second.stage = TaskEvidenceDeliveryStage::Integrate;
    assert!(!first.same_assessment_as(&second));
}

#[test]
fn evidence_binding_requires_an_exact_generation_and_closed_json() {
    let binding = TaskEvidenceDeliveryBinding {
        scope: TaskEvidenceScope {
            task_id: "task one".into(),
            scope_id: "a".repeat(64),
            acceptance_revision: 1,
            criteria_sha256: "b".repeat(64),
        },
        lease_generation: "c".repeat(64),
    };
    binding.validate().unwrap();
    let mut invalid = binding.clone();
    invalid.lease_generation.clear();
    assert!(invalid.validate().is_err());
    let mut json = serde_json::to_value(&binding).unwrap();
    json["approved"] = serde_json::json!(true);
    assert!(serde_json::from_value::<TaskEvidenceDeliveryBinding>(json).is_err());
}

#[test]
fn automatic_policy_reasons_remain_denials_in_observe_and_public_round_trip() {
    for reason in [
        TaskDeliveryEvidenceReason::AutoRequiresEnforce,
        TaskDeliveryEvidenceReason::AutoRequiresMachineCriterion,
    ] {
        let mut observation = empty(TaskEvidenceDeliveryMode::Observe);
        observation.reasons.insert(0, reason);
        observation.validate().unwrap();
        assert!(!observation.allows_delivery());
        let restored: TaskDeliveryEvidenceObservation =
            serde_json::from_str(&serde_json::to_string(&observation).unwrap()).unwrap();
        assert!(!restored.allows_delivery());
        assert_eq!(restored.reasons[0], reason);
        let mut ordinary = observation.clone();
        ordinary.reasons.remove(0);
        assert!(ordinary.allows_delivery());
        assert!(!ordinary.same_assessment_as(&observation));
    }
}
