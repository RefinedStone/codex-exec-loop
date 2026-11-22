use super::*;

fn registered(key: &str) -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new(key).unwrap(),
        PrValidationTarget::new("synthetic/repository", 1).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new("a".repeat(40)).unwrap(),
            PrValidationCommitSha::new("b".repeat(40)).unwrap(),
        ),
    )
}

#[test]
fn fresh_successor_preserves_contract_and_admitted_correlations_without_reopening_terminal() {
    let finding = PrValidationFinding::new(
        PrValidationFindingKey::new(PrValidationFindingSource::new("ci").unwrap(), "finding")
            .unwrap(),
        PrValidationCommitSha::new("a".repeat(40)).unwrap(),
        "Existing failure",
    )
    .unwrap();
    let mut original = registered("original")
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::FindingObserved(finding.clone()))
        .unwrap()
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                finding.key().clone(),
                PrValidationRecordKey::new("already-admitted-task").unwrap(),
            ),
        ))
        .unwrap()
        .transition(PrValidationEvent::ObservationCheckpointed {
            delivery_revision: 99,
            cursor: Some("old-cursor".into()),
            evidence_fingerprint: "old-evidence".into(),
        })
        .unwrap()
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    original.post_merge_validation_contract = PostMergeValidationContract::legacy_ci_gate_v1();
    let before = original.clone();
    let successor = original
        .fresh_recovery_successor(PrValidationRecordKey::new("successor").unwrap())
        .unwrap();
    assert_eq!(original, before);
    assert_eq!(successor.phase(), PrValidationPhase::Registered);
    assert_eq!(successor.target(), original.target());
    assert_eq!(successor.target_shas(), original.target_shas());
    assert_eq!(
        successor.post_merge_validation_contract(),
        original.post_merge_validation_contract()
    );
    assert_eq!(successor.finding(finding.key()), Some(&finding));
    assert_eq!(
        successor.remediation_for(finding.key()),
        original.remediation_for(finding.key())
    );
    assert_eq!(successor.observation_revision(), 0);
    assert_eq!(successor.observation_cursor(), None);
    assert_eq!(successor.evidence_fingerprint(), None);
    assert!(successor.terminal_reason().is_none());
    assert!(successor.active_remediation.is_none());
    assert!(
        original
            .transition(PrValidationEvent::BeginPreMergeObservation)
            .is_err()
    );
}

#[test]
fn recovery_requires_a_new_identity_and_an_eligible_terminal() {
    let original = registered("original")
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::RemediationAdmissionFailed,
        ))
        .unwrap();
    assert!(
        original
            .fresh_recovery_successor(original.key().clone())
            .is_err()
    );
    assert!(
        original
            .fresh_recovery_successor(PrValidationRecordKey::new("new").unwrap())
            .is_ok()
    );
    assert!(
        registered("active")
            .fresh_recovery_successor(PrValidationRecordKey::new("new").unwrap())
            .is_err()
    );
    let identity_failure = registered("identity")
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::IdentityViolation,
        ))
        .unwrap();
    assert!(
        identity_failure
            .fresh_recovery_successor(PrValidationRecordKey::new("new").unwrap())
            .is_err()
    );
    let integrated = registered("integrated")
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::MergeObserved(
            PrValidationCommitSha::new("c".repeat(40)).unwrap(),
        ))
        .unwrap()
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let successor = integrated
        .fresh_recovery_successor(PrValidationRecordKey::new("new").unwrap())
        .unwrap();
    assert_eq!(successor.phase(), PrValidationPhase::PostMergeObservation);
    assert_eq!(successor.merge_sha(), integrated.merge_sha());
    assert_eq!(successor.evidence_sha(), integrated.evidence_sha());
}

#[test]
fn postintegration_recovery_preserves_trusted_attestation_and_resets_only_observation() {
    let now = DateTime::parse_from_rfc3339("2026-11-21T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let attestation = IntegrationAttestation::new(
        IntegrationMethod::GithubRebaseMerge,
        PrValidationCommitSha::new("a".repeat(40)).unwrap(),
        Some(PrValidationCommitSha::new("b".repeat(40)).unwrap()),
        PrValidationCommitSha::new("c".repeat(40)).unwrap(),
        Some(1),
        Some(PrValidationCommitSha::new("c".repeat(40)).unwrap()),
        now,
        now + chrono::Duration::seconds(1),
    )
    .unwrap();
    let integrated = registered("integrated-attested")
        .transition(PrValidationEvent::IntegrationAttested(attestation.clone()))
        .unwrap()
        .transition(PrValidationEvent::ObservationCheckpointed {
            delivery_revision: 99,
            cursor: Some("post-merge-cursor".into()),
            evidence_fingerprint: "old-post-merge-evidence".into(),
        })
        .unwrap();
    assert!(integrated.has_post_merge_checkpoint());
    let original = integrated
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let before = original.clone();
    let successor = original
        .fresh_recovery_successor(PrValidationRecordKey::new("post-merge-successor").unwrap())
        .unwrap();
    assert_eq!(original, before);
    assert_eq!(successor.phase(), PrValidationPhase::PostMergeObservation);
    assert_eq!(successor.integration_attestation(), Some(&attestation));
    assert_eq!(successor.merge_sha(), original.merge_sha());
    assert_eq!(successor.target_shas(), original.target_shas());
    assert_eq!(
        successor.post_merge_validation_contract(),
        original.post_merge_validation_contract(),
    );
    assert_eq!(successor.observation_revision(), 0);
    assert_eq!(successor.observation_cursor(), None);
    assert_eq!(successor.evidence_fingerprint(), None);
    assert!(!successor.has_post_merge_checkpoint());
    assert!(successor.terminal_reason().is_none());

    let failed_queue = integrated
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::RemediationAdmissionFailed,
        ))
        .unwrap();
    assert!(
        failed_queue
            .fresh_recovery_successor(PrValidationRecordKey::new("queue-successor").unwrap())
            .is_err()
    );
}
