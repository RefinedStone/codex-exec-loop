use super::{
    PrValidationCatchUpState, PrValidationCommitSha, PrValidationCompletion, PrValidationEvent,
    PrValidationFinding, PrValidationFindingKey, PrValidationFindingSource,
    PrValidationObservationProjection, PrValidationObservedCheck, PrValidationObservedCheckStatus,
    PrValidationObservedRunStatus, PrValidationObservedWorkflow, PrValidationOperatorState,
    PrValidationPhase, PrValidationProviderCompletion, PrValidationProviderKey, PrValidationRecord,
    PrValidationRecordKey, PrValidationRemediationCorrelation, PrValidationTarget,
    PrValidationTargetShaSnapshot, PrValidationTerminalReason, PrValidationWorkflowSelectionBasis,
};

const SOURCE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASE_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MERGE_SHA: &str = "cccccccccccccccccccccccccccccccccccccccc";

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

fn settled_with_provider(provider: PrValidationProviderCompletion) -> PrValidationRecord {
    registered()
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::MergeObserved(
            PrValidationCommitSha::new(MERGE_SHA).unwrap(),
        ))
        .unwrap()
        .transition(PrValidationEvent::BeginPostMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::Settle(PrValidationCompletion::new(
            PrValidationCommitSha::new(MERGE_SHA).unwrap(),
            vec![provider],
            Vec::new(),
            PrValidationCatchUpState::NoUnseenRelevantEvents,
        )))
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
    assert_eq!(blocking_summary.akra_id, "queue-42");
    assert_eq!(
        blocking_summary.canonical_pr_url,
        "https://github.com/acme/widgets/pull/42"
    );
    assert_eq!(blocking_summary.target_short_sha, "aaaaaaaaaaaa");
    assert_eq!(blocking_summary.finding_count, 1);
    assert_eq!(blocking_summary.remediation_count, 0);
    assert_eq!(remediation_summary.correlations.len(), 1);
    assert_eq!(
        remediation_summary.correlations[0].remediation_akra_id,
        "remediation-task-1"
    );
    assert!(
        remediation_summary.correlations[0]
            .finding
            .starts_with("check_run:")
    );
    assert!(blocking_summary.reason.contains("check_run finding"));
    assert_eq!(blocking_summary, blocking.operator_summary());

    let visible = format!(
        "{} {} {} {} {} {} {:?}",
        blocking_summary.akra_id,
        blocking_summary.canonical_pr_url,
        blocking_summary.compact_label(),
        blocking_summary.phase_label(),
        blocking_summary.target_short_sha,
        blocking_summary.next_action(),
        remediation_summary.correlations
    );
    assert!(visible.len() < 512, "operator projection must stay bounded");
    assert!(!visible.contains("ghp_secret_canary"));
    assert!(!visible.contains("raw payload"));
    assert!(!visible.contains("provider-event"));
}

#[test]
fn blocked_and_failed_are_durable_terminal_states_with_recovery_reasons() {
    for (event, phase, state, reason, recovery) in [
        (
            PrValidationEvent::Block(PrValidationTerminalReason::PullRequestClosedWithoutMerge),
            PrValidationPhase::Blocked,
            PrValidationOperatorState::Blocked,
            "pull request closed without merge",
            "reopen the pull request",
        ),
        (
            PrValidationEvent::Fail(PrValidationTerminalReason::ObservationFailed),
            PrValidationPhase::Failed,
            PrValidationOperatorState::Failed,
            "trusted validation observation failed",
            "rerun validation",
        ),
    ] {
        let terminal = registered().transition(event).unwrap();
        let summary = terminal.operator_summary();

        assert_eq!(terminal.phase(), phase);
        assert_eq!(summary.state, state);
        assert_eq!(summary.reason, reason);
        assert!(summary.recovery_action.contains(recovery));
        assert!(
            terminal
                .transition(PrValidationEvent::BeginPreMergeObservation)
                .is_err()
        );
    }
}

#[test]
fn validation_record_snapshots_contract_and_migrates_legacy_json_to_v1() {
    let record = registered();
    assert_eq!(record.post_merge_validation_contract().version(), 1);
    assert_eq!(
        record
            .post_merge_validation_contract()
            .required_check_contexts()[0]
            .context(),
        "Post-Merge Gate"
    );

    let mut persisted = serde_json::to_value(&record).unwrap();
    persisted["post_merge_validation_contract"]["version"] = serde_json::json!(7);
    persisted["post_merge_validation_contract"]["required_check_contexts"] =
        serde_json::json!([{"app_slug":"github-actions","context":"Frozen Gate"}]);
    persisted["post_merge_validation_contract"]["optional_check_contexts"] = serde_json::json!([]);
    let frozen: PrValidationRecord = serde_json::from_value(persisted.clone()).unwrap();
    assert_eq!(frozen.post_merge_validation_contract().version(), 7);
    assert_eq!(
        frozen
            .post_merge_validation_contract()
            .required_check_contexts()[0]
            .context(),
        "Frozen Gate"
    );
    assert_eq!(serde_json::to_value(&frozen).unwrap(), persisted);

    let mut legacy = serde_json::to_value(&record).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("post_merge_validation_contract");
    let migrated: PrValidationRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(migrated.post_merge_validation_contract().version(), 1);
    assert_eq!(
        migrated
            .post_merge_validation_contract()
            .required_check_contexts()[0]
            .context(),
        "CI Gate"
    );
}

#[test]
fn watchable_completion_settles_one_cycle_but_keeps_the_late_review_boundary_active() {
    let provider = PrValidationProviderKey::new("github:ReviewThreads").unwrap();
    let settled = settled_with_provider(PrValidationProviderCompletion::watchable(provider));

    assert_eq!(settled.phase(), PrValidationPhase::Settled);
    assert!(settled.review_watch_active());
    let reopened = settled
        .transition(PrValidationEvent::LateFindingObserved(finding()))
        .expect("a trusted late event should reopen a watchable settled record");
    assert_eq!(reopened.phase(), PrValidationPhase::PostMergeObservation);
    assert!(!reopened.review_watch_active());
    assert_eq!(reopened.finding_keys().len(), 1);
}

#[test]
fn finite_completion_is_terminal_and_rejects_late_review_reopen() {
    let provider = PrValidationProviderKey::new("github:CheckRuns").unwrap();
    let settled = settled_with_provider(PrValidationProviderCompletion::terminal(provider));

    assert_eq!(settled.phase(), PrValidationPhase::Settled);
    assert!(!settled.review_watch_active());
    assert!(
        settled
            .transition(PrValidationEvent::LateFindingObserved(finding()))
            .is_err()
    );
}

#[test]
fn observation_projection_rejects_unbounded_or_malformed_provider_timestamps() {
    let context =
        super::PrValidationCheckContext::new(Some("github-actions".to_string()), "Post-Merge Gate")
            .unwrap();
    for timestamp in ["not-rfc3339".to_string(), "2".repeat(65)] {
        let error = PrValidationObservationProjection::new(
            vec![PrValidationObservedCheck::new(
                context.clone(),
                PrValidationObservedCheckStatus::Pending,
                Some(1),
                Some(timestamp),
                None,
            )],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap_err();
        assert!(error.contains("bounded RFC3339"));
    }

    let error = PrValidationObservationProjection::new(
        Vec::new(),
        Vec::new(),
        vec![
            PrValidationObservedWorkflow::selected(
                "Native PR Checks",
                PrValidationObservedRunStatus::InProgress,
                1,
                Some("not-rfc3339".to_string()),
                None,
                None,
                PrValidationWorkflowSelectionBasis::NewestRun,
            )
            .unwrap(),
        ],
        Vec::new(),
    )
    .unwrap_err();
    assert!(error.contains("bounded RFC3339"));
}

#[test]
fn observed_workflow_additive_fields_round_trip_and_legacy_json_defaults_safely() {
    let legacy = serde_json::json!({
        "name": "Native PR Checks",
        "status": "in_progress",
        "run_attempt": 2,
        "started_at": "2026-08-10T01:00:00Z",
        "updated_at": "2026-08-10T01:01:00Z"
    });
    let legacy_workflow: PrValidationObservedWorkflow =
        serde_json::from_value(legacy).expect("legacy workflow projection should deserialize");
    assert_eq!(legacy_workflow.created_at(), None);
    assert_eq!(
        legacy_workflow.selection_basis(),
        PrValidationWorkflowSelectionBasis::LegacyUnknown
    );

    let workflow = PrValidationObservedWorkflow::selected(
        "Native PR Checks",
        PrValidationObservedRunStatus::Succeeded,
        3,
        Some("2026-08-10T00:00:00Z".to_string()),
        Some("2026-08-10T00:10:00Z".to_string()),
        Some("2026-08-10T00:11:00Z".to_string()),
        PrValidationWorkflowSelectionBasis::LatestAttempt,
    )
    .unwrap();
    let projection =
        PrValidationObservationProjection::new(Vec::new(), Vec::new(), vec![workflow], Vec::new())
            .unwrap();
    let projected = registered()
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::ObservationProjected(projection))
        .unwrap();

    let serialized = serde_json::to_string(&projected).unwrap();
    assert!(serialized.contains("\"created_at\":\"2026-08-10T00:00:00Z\""));
    assert!(serialized.contains("\"selection_basis\":\"latest_attempt\""));
    let replayed: PrValidationRecord = serde_json::from_str(&serialized).unwrap();
    assert_eq!(replayed, projected);
}
