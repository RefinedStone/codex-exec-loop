use super::{
    PrValidationCommitSha, PrValidationEvent, PrValidationFinding, PrValidationFindingKey,
    PrValidationFindingSource, PrValidationOperatorState, PrValidationRecord,
    PrValidationRecordKey, PrValidationRemediationCorrelation, PrValidationTarget,
    PrValidationTargetShaSnapshot,
};

const SOURCE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASE_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn registered() -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new("queue-42").unwrap(),
        PrValidationTarget::new("acme/widgets", 42).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new(SOURCE_SHA).unwrap(),
            PrValidationCommitSha::new(BASE_SHA).unwrap(),
        ),
    )
}

fn finding() -> PrValidationFinding {
    PrValidationFinding::new(
        PrValidationFindingKey::new(
            PrValidationFindingSource::new("check_run").unwrap(),
            "provider-event-with-token-ghp_secret_canary",
        )
        .unwrap(),
        PrValidationCommitSha::new(SOURCE_SHA).unwrap(),
        "raw payload token=ghp_secret_canary ".to_string() + &"x".repeat(20_000),
    )
    .unwrap()
}

#[test]
fn pr_validation_operator_projection_is_deterministic_bounded_and_secret_free() {
    let pending = registered()
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap();
    let blocking = pending
        .transition(PrValidationEvent::FindingObserved(finding()))
        .unwrap();
    let remediation = blocking
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                finding().key().clone(),
                PrValidationRecordKey::new("remediation-task-1").unwrap(),
            ),
        ))
        .unwrap();

    let pending_summary = pending.operator_summary();
    let blocking_summary = blocking.operator_summary();
    let remediation_summary = remediation.operator_summary();

    assert_eq!(pending_summary.state, PrValidationOperatorState::Pending);
    assert_eq!(blocking_summary.state, PrValidationOperatorState::Blocking);
    assert_eq!(
        remediation_summary.state,
        PrValidationOperatorState::Remediation
    );
    assert_eq!(blocking_summary.target_short_sha, "aaaaaaaaaaaa");
    assert_eq!(blocking_summary.finding_count, 1);
    assert_eq!(blocking_summary.remediation_count, 0);
    assert_eq!(blocking_summary, blocking.operator_summary());

    let visible = format!(
        "{} {} {} {}",
        blocking_summary.compact_label(),
        blocking_summary.phase_label(),
        blocking_summary.target_short_sha,
        blocking_summary.next_action()
    );
    assert!(visible.len() < 256, "operator projection must stay bounded");
    assert!(!visible.contains("ghp_secret_canary"));
    assert!(!visible.contains("raw payload"));
    assert!(!visible.contains("provider-event"));
}
