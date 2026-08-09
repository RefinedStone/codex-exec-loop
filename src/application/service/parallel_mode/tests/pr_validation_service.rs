use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::{TempGitRepo, test_parallel_mode_service};
use crate::application::port::outbound::github_pr_validation_port::{
    GithubPrMergeState, GithubPrValidationObservationRequest, GithubPrValidationPort,
    GithubPrValidationSnapshot, GithubValidationActivity, GithubValidationActivityKind,
    GithubValidationCheckRun, GithubValidationCursor, GithubValidationRunStatus,
    GithubValidationSource, GithubValidationSourceObservation, GithubValidationSourceStatus,
    GithubValidationWorkflowRun,
};
use crate::application::port::outbound::pr_validation_remediation_port::{
    PrValidationRemediationPort, PrValidationRemediationRequest,
};
use crate::application::service::parallel_mode::{PrValidationPollRequest, PrValidationPollResult};
use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};
use crate::domain::parallel_mode::{
    IntegrationAttestation, IntegrationMethod, PrValidationCommitSha, PrValidationEvent,
    PrValidationPhase, PrValidationRecord, PrValidationRecordKey, PrValidationTarget,
    PrValidationTargetShaSnapshot,
};

const HEAD_A: &str = "1111111111111111111111111111111111111111";
const HEAD_B: &str = "2222222222222222222222222222222222222222";
const BASE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MERGE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct SnapshotPort {
    snapshots: Mutex<VecDeque<GithubPrValidationSnapshot>>,
    requests: Mutex<Vec<GithubPrValidationObservationRequest>>,
    pool_root: std::path::PathBuf,
}

impl GithubPrValidationPort for SnapshotPort {
    fn load_validation_snapshot(
        &self,
        request: &GithubPrValidationObservationRequest,
    ) -> std::result::Result<
        GithubPrValidationSnapshot,
        crate::application::port::outbound::github_pr_validation_port::GithubPrValidationError,
    > {
        assert!(
            !self.pool_root.join(".leases").exists(),
            "validation polling must not hold a worktree/slot lease while waiting"
        );
        self.requests.lock().unwrap().push(request.clone());
        Ok(self.snapshots.lock().unwrap().pop_front().unwrap())
    }
}

struct FailingSnapshotPort;

impl GithubPrValidationPort for FailingSnapshotPort {
    fn load_validation_snapshot(
        &self,
        _request: &GithubPrValidationObservationRequest,
    ) -> std::result::Result<
        GithubPrValidationSnapshot,
        crate::application::port::outbound::github_pr_validation_port::GithubPrValidationError,
    > {
        Err(
            crate::application::port::outbound::github_pr_validation_port::GithubPrValidationError::retryable(
                "provider unavailable",
            ),
        )
    }
}

#[derive(Default)]
struct RemediationPort {
    unique: Mutex<BTreeSet<String>>,
    deliveries: Mutex<Vec<PrValidationRemediationRequest>>,
    after_delivery: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl PrValidationRemediationPort for RemediationPort {
    fn request_remediation(
        &self,
        request: &PrValidationRemediationRequest,
    ) -> Result<PrValidationRecordKey> {
        let mut unique = self.unique.lock().unwrap();
        if unique.insert(request.idempotency_key.as_str().to_string()) {
            self.deliveries.lock().unwrap().push(request.clone());
        }
        drop(unique);
        if let Some(after_delivery) = self.after_delivery.lock().unwrap().take() {
            after_delivery();
        }
        Ok(
            PrValidationRecordKey::new(format!("task-{}", request.idempotency_key.as_str()))
                .unwrap(),
        )
    }
}

fn sources() -> Vec<GithubValidationSourceObservation> {
    GithubValidationSource::ALL
        .into_iter()
        .map(|source| {
            GithubValidationSourceObservation::new(
                source,
                format!("rest:{source:?}"),
                GithubValidationSourceStatus::Complete,
                None,
            )
        })
        .collect()
}

fn snapshot(sha: &str, status: GithubValidationRunStatus) -> GithubPrValidationSnapshot {
    GithubPrValidationSnapshot {
        target: GithubPullRequestTarget::new("acme/widgets", 42),
        target_sha: GithubCommitSha::new(sha),
        evidence_sha: GithubCommitSha::new(sha),
        merge_state: GithubPrMergeState::Open,
        merge_sha: None,
        activities: Vec::new(),
        check_runs: vec![
            GithubValidationCheckRun::new(
                GithubOpaqueId::new("check:ci"),
                "Post-Merge Gate",
                GithubCommitSha::new(sha),
                status,
            )
            .with_attempt_metadata(
                Some("github-actions".to_string()),
                Some(GithubOpaqueId::new("check-suite:ci")),
                Some("2026-08-08T00:00:00Z".to_string()),
                Some("2026-08-08T00:01:00Z".to_string()),
            ),
        ],
        workflow_runs: Vec::new(),
        sources: sources(),
        next_cursor: None,
        provider_metadata: Default::default(),
    }
}

fn merged_snapshot() -> GithubPrValidationSnapshot {
    let mut snapshot = snapshot(HEAD_A, GithubValidationRunStatus::Succeeded);
    snapshot.evidence_sha = GithubCommitSha::new(MERGE);
    snapshot.check_runs[0].target_sha = GithubCommitSha::new(MERGE);
    snapshot.merge_state = GithubPrMergeState::Merged;
    snapshot.merge_sha = Some(GithubCommitSha::new(MERGE));
    snapshot
}

fn target_shas(sha: &str) -> PrValidationTargetShaSnapshot {
    PrValidationTargetShaSnapshot::new(
        PrValidationCommitSha::new(sha).unwrap(),
        PrValidationCommitSha::new(BASE).unwrap(),
    )
}

fn registered_record() -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        PrValidationTarget::new("acme/widgets", 42).unwrap(),
        target_shas(HEAD_A),
    )
}

fn distributor_attested_record() -> PrValidationRecord {
    let observed_at = DateTime::parse_from_rfc3339("2026-08-10T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    registered_record()
        .transition(PrValidationEvent::IntegrationAttested(
            IntegrationAttestation::new(
                IntegrationMethod::DistributorCherryPick,
                PrValidationCommitSha::new(HEAD_A).unwrap(),
                Some(PrValidationCommitSha::new(BASE).unwrap()),
                PrValidationCommitSha::new(MERGE).unwrap(),
                Some(42),
                None,
                observed_at,
                observed_at,
            )
            .unwrap(),
        ))
        .unwrap()
}

fn request(repo: &TempGitRepo, revision: u64, sha: &str) -> PrValidationPollRequest {
    PrValidationPollRequest {
        workspace_dir: repo.workspace_dir(),
        pool_root: repo.pool_root(),
        record_key: PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        target_shas: target_shas(sha),
        delivery_revision: revision,
    }
}

fn setup(
    name: &str,
    snapshots: Vec<GithubPrValidationSnapshot>,
) -> (TempGitRepo, SnapshotPort, RemediationPort) {
    let repo = TempGitRepo::new(name);
    let observation = SnapshotPort {
        snapshots: Mutex::new(snapshots.into()),
        requests: Mutex::new(Vec::new()),
        pool_root: repo.pool_root(),
    };
    (repo, observation, RemediationPort::default())
}

#[test]
fn actionable_review_comment_and_thread_activity_admit_idempotent_remediation() {
    for (index, kind, expected_source) in [
        (0, GithubValidationActivityKind::Review, "review"),
        (
            1,
            GithubValidationActivityKind::IssueComment,
            "issue_comment",
        ),
        (
            2,
            GithubValidationActivityKind::ReviewThread,
            "review_thread",
        ),
        (
            3,
            GithubValidationActivityKind::ReviewComment,
            "review_comment",
        ),
    ] {
        let mut observed = snapshot(HEAD_A, GithubValidationRunStatus::Succeeded);
        observed.activities.push(
            GithubValidationActivity::new(
                GithubOpaqueId::new(format!("activity:{index}")),
                kind,
                "2026-08-08T00:00:00Z",
            )
            .with_commit_sha(GithubCommitSha::new(HEAD_A)),
        );
        let (repo, observation, remediation) = setup(
            &format!("validation-actionable-{index}"),
            vec![observed.clone(), observed],
        );
        let service = test_parallel_mode_service();
        service
            .persist_pr_validation_record(
                &repo.workspace_dir(),
                &repo.pool_root(),
                None,
                &registered_record(),
            )
            .unwrap();

        assert!(matches!(
            service
                .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
                .unwrap(),
            PrValidationPollResult::RemediationRequested { .. }
        ));
        assert_eq!(
            service
                .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_A))
                .unwrap(),
            PrValidationPollResult::Waiting
        );
        let deliveries = remediation.deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].finding_key.source().as_str(), expected_source);
    }
}

#[test]
fn closed_pr_blocks_but_retryable_provider_error_does_not_persist_terminal_failure() {
    let mut closed = snapshot(HEAD_A, GithubValidationRunStatus::Succeeded);
    closed.merge_state = GithubPrMergeState::Closed;
    let (blocked_repo, observation, remediation) =
        setup("validation-blocked-terminal", vec![closed]);
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &blocked_repo.workspace_dir(),
            &blocked_repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();
    assert_eq!(
        service
            .poll_pr_validation(
                &observation,
                &remediation,
                request(&blocked_repo, 1, HEAD_A),
            )
            .unwrap(),
        PrValidationPollResult::Blocked
    );
    let blocked = service
        .recover_pr_validation_record(
            &blocked_repo.workspace_dir(),
            &blocked_repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap()
        .operator_summary();
    assert_eq!(blocked.phase, PrValidationPhase::Blocked);
    assert_eq!(blocked.reason, "integration evidence is missing");

    let failed_repo = TempGitRepo::new("validation-failed-terminal");
    service
        .persist_pr_validation_record(
            &failed_repo.workspace_dir(),
            &failed_repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();
    let error = service
        .poll_pr_validation(
            &FailingSnapshotPort,
            &remediation,
            request(&failed_repo, 1, HEAD_A),
        )
        .expect_err(
            "retryable provider failure must be scheduled outside the record state machine",
        );
    assert_eq!(error, "provider unavailable");
    let failed = service
        .recover_pr_validation_record(
            &failed_repo.workspace_dir(),
            &failed_repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap()
        .operator_summary();
    assert_eq!(failed.phase, PrValidationPhase::Registered);
    assert_eq!(
        failed.reason,
        "validation sources or final catch-up are not complete"
    );
    assert!(!format!("{failed:?}").contains("ghp_provider_secret_canary"));
}

#[test]
fn validation_waits_for_ci_without_holding_a_worktree_or_slot_lease() {
    let (repo, observation, remediation) = setup(
        "validation-lease-free",
        vec![snapshot(HEAD_A, GithubValidationRunStatus::InProgress)],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    let result = service
        .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
        .unwrap();

    assert_eq!(result, PrValidationPollResult::Waiting);
    assert!(remediation.deliveries.lock().unwrap().is_empty());
}

#[test]
fn replayed_event_and_stale_update_admit_no_duplicate() {
    let failed = snapshot(HEAD_A, GithubValidationRunStatus::Failed);
    let (repo, observation, remediation) =
        setup("validation-idempotency", vec![failed.clone(), failed]);
    let service = test_parallel_mode_service();
    let registered = registered_record();
    service
        .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &registered)
        .unwrap();

    let competing_service = service.clone();
    let workspace_dir = repo.workspace_dir();
    let pool_root = repo.pool_root();
    let expected = registered.clone();
    *remediation.after_delivery.lock().unwrap() = Some(Box::new(move || {
        let competing = expected
            .transition(PrValidationEvent::BeginPreMergeObservation)
            .unwrap()
            .transition(PrValidationEvent::ObservationCheckpointed {
                delivery_revision: 6,
                cursor: None,
                evidence_fingerprint: "competing-observation".to_string(),
            })
            .unwrap();
        competing_service
            .persist_pr_validation_record(&workspace_dir, &pool_root, Some(&expected), &competing)
            .unwrap();
    }));

    let lost_cas =
        service.poll_pr_validation(&observation, &remediation, request(&repo, 7, HEAD_A));
    assert!(
        lost_cas
            .unwrap_err()
            .contains("changed before validation transition")
    );

    let replay = service
        .poll_pr_validation(&observation, &remediation, request(&repo, 8, HEAD_A))
        .unwrap();
    let stale = service
        .poll_pr_validation(&observation, &remediation, request(&repo, 7, HEAD_A))
        .unwrap();

    assert!(matches!(
        replay,
        PrValidationPollResult::RemediationRequested { .. }
    ));
    assert_eq!(stale, PrValidationPollResult::StaleDeliveryIgnored);
    assert_eq!(remediation.deliveries.lock().unwrap().len(), 1);
    assert_eq!(observation.requests.lock().unwrap().len(), 2);
}

#[test]
fn remediation_uses_the_ordinary_task_identity_and_lifecycle_transitions_are_replay_safe() {
    let (repo, observation, remediation) = setup(
        "validation-remediation-lifecycle",
        vec![snapshot(HEAD_A, GithubValidationRunStatus::Failed)],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    service
        .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
        .unwrap();
    let delivery = remediation.deliveries.lock().unwrap()[0].clone();
    let task_id = format!("task-{}", delivery.idempotency_key.as_str());
    let queued = service
        .recover_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap();
    let finding_key = queued.finding_keys().into_iter().next().unwrap();
    assert_eq!(
        queued
            .remediation_for(&finding_key)
            .unwrap()
            .remediation_key()
            .as_str(),
        task_id
    );

    assert!(
        service
            .transition_pr_validation_remediation_started(
                &repo.workspace_dir(),
                &repo.pool_root(),
                &task_id,
            )
            .unwrap()
    );
    assert!(
        !service
            .transition_pr_validation_remediation_started(
                &repo.workspace_dir(),
                &repo.pool_root(),
                &task_id,
            )
            .unwrap()
    );
    assert!(
        service
            .transition_pr_validation_remediation_completed(
                &repo.workspace_dir(),
                &repo.pool_root(),
                &task_id,
            )
            .unwrap()
    );
    assert!(
        !service
            .transition_pr_validation_remediation_completed(
                &repo.workspace_dir(),
                &repo.pool_root(),
                &task_id,
            )
            .unwrap()
    );
    let completed = service
        .recover_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(completed.phase(), PrValidationPhase::PreMergeObservation);
}

#[test]
fn target_sha_change_resets_cursor_and_revision_evidence_before_polling() {
    let (repo, observation, remediation) = setup(
        "validation-sha-reset",
        vec![
            snapshot(HEAD_A, GithubValidationRunStatus::Succeeded),
            snapshot(HEAD_B, GithubValidationRunStatus::InProgress),
        ],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();
    service
        .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
        .unwrap();

    service
        .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_B))
        .unwrap();

    let requests = observation.requests.lock().unwrap();
    assert_eq!(requests[1].target_sha.as_str(), HEAD_B);
    assert!(requests[1].cursor.is_none());
    let record = service
        .recover_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(record.target_shas().source_sha().as_str(), HEAD_B);
    assert!(record.finding_keys().is_empty());
}

#[test]
fn stale_delivery_order_is_ignored_before_observation_or_persistence() {
    let (repo, observation, remediation) = setup(
        "validation-stale-order",
        vec![snapshot(HEAD_A, GithubValidationRunStatus::InProgress)],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();
    service
        .poll_pr_validation(&observation, &remediation, request(&repo, 9, HEAD_A))
        .unwrap();

    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 8, HEAD_A))
            .unwrap(),
        PrValidationPollResult::StaleDeliveryIgnored
    );
    assert_eq!(observation.requests.lock().unwrap().len(), 1);
}

#[test]
fn failed_or_cancelled_evidence_cannot_settle_after_remediation_completion() {
    for (name, status) in [
        ("failed-check", GithubValidationRunStatus::Failed),
        ("cancelled-check", GithubValidationRunStatus::Cancelled),
    ] {
        let mut failed = merged_snapshot();
        failed.check_runs[0].status = status;
        let (repo, observation, remediation) = setup(name, vec![failed.clone(), failed]);
        let service = test_parallel_mode_service();
        service
            .persist_pr_validation_record(
                &repo.workspace_dir(),
                &repo.pool_root(),
                None,
                &registered_record(),
            )
            .unwrap();

        let first = service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap();
        assert!(matches!(
            first,
            PrValidationPollResult::RemediationRequested { .. }
        ));
        let delivery = remediation.deliveries.lock().unwrap()[0].clone();
        let task_id = format!("task-{}", delivery.idempotency_key.as_str());
        assert!(
            service
                .transition_pr_validation_remediation_completed(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    &task_id,
                )
                .unwrap()
        );

        assert_eq!(
            service
                .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_A))
                .unwrap(),
            PrValidationPollResult::Waiting,
            "unsuccessful evidence must not settle: {name}"
        );
    }
}

#[test]
fn required_missing_or_skipped_check_becomes_a_stable_policy_blocker() {
    for (name, status) in [
        ("missing", None),
        ("skipped", Some(GithubValidationRunStatus::Skipped)),
    ] {
        let mut observed = merged_snapshot();
        if let Some(status) = status {
            observed.check_runs[0].status = status;
        } else {
            observed.check_runs.clear();
            observed.workflow_runs = vec![
                GithubValidationWorkflowRun::new(
                    GithubOpaqueId::new("workflow:terminal-without-required-context"),
                    "Native PR Checks",
                    GithubCommitSha::new(MERGE),
                    GithubValidationRunStatus::Succeeded,
                )
                .with_attempt_metadata(
                    1,
                    Some("2026-08-08T00:00:00Z".to_string()),
                    Some("2026-08-08T00:10:00Z".to_string()),
                ),
            ];
        }
        let (repo, observation, remediation) = setup(
            &format!("validation-required-{name}"),
            vec![observed.clone(), observed],
        );
        let service = test_parallel_mode_service();
        service
            .persist_pr_validation_record(
                &repo.workspace_dir(),
                &repo.pool_root(),
                None,
                &registered_record(),
            )
            .unwrap();

        assert_eq!(
            service
                .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
                .unwrap(),
            PrValidationPollResult::Waiting
        );
        assert_eq!(
            service
                .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_A))
                .unwrap(),
            PrValidationPollResult::Blocked
        );
        assert!(remediation.deliveries.lock().unwrap().is_empty());
    }
}

#[test]
fn missing_required_check_waits_while_the_workflow_container_is_active() {
    let mut observed = merged_snapshot();
    observed.check_runs.clear();
    observed.workflow_runs = vec![
        GithubValidationWorkflowRun::new(
            GithubOpaqueId::new("workflow:active-before-gate"),
            "Native PR Checks",
            GithubCommitSha::new(MERGE),
            GithubValidationRunStatus::InProgress,
        )
        .with_attempt_metadata(
            1,
            Some("2026-08-08T00:00:00Z".to_string()),
            Some("2026-08-08T00:01:00Z".to_string()),
        ),
    ];
    let (repo, observation, remediation) = setup(
        "validation-required-missing-active-workflow",
        vec![observed.clone(), observed.clone(), observed],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    for revision in 1..=3 {
        assert_eq!(
            service
                .poll_pr_validation(&observation, &remediation, request(&repo, revision, HEAD_A),)
                .unwrap(),
            PrValidationPollResult::Waiting
        );
    }
    assert_eq!(
        service
            .recover_pr_validation_record(
                &repo.workspace_dir(),
                &repo.pool_root(),
                &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
            )
            .unwrap()
            .unwrap()
            .phase(),
        PrValidationPhase::PostMergeObservation
    );
    assert!(remediation.deliveries.lock().unwrap().is_empty());
}

#[test]
fn optional_skip_and_workflow_failure_are_diagnostic_when_the_required_check_succeeds() {
    let mut observed = merged_snapshot();
    observed.check_runs.push(
        GithubValidationCheckRun::new(
            GithubOpaqueId::new("check:optional-rust"),
            "Rust Tests",
            GithubCommitSha::new(MERGE),
            GithubValidationRunStatus::Skipped,
        )
        .with_attempt_metadata(
            Some("github-actions".to_string()),
            Some(GithubOpaqueId::new("check-suite:optional-rust")),
            Some("2026-08-08T00:00:00Z".to_string()),
            Some("2026-08-08T00:00:00Z".to_string()),
        ),
    );
    observed.workflow_runs = vec![
        GithubValidationWorkflowRun::new(
            GithubOpaqueId::new("workflow:provider-failure"),
            "Native PR Checks",
            GithubCommitSha::new(MERGE),
            GithubValidationRunStatus::Cancelled,
        )
        .with_attempt_metadata(
            2,
            Some("2026-08-08T00:00:00Z".to_string()),
            Some("2026-08-08T00:02:00Z".to_string()),
        ),
    ];
    let (repo, observation, remediation) = setup(
        "validation-workflow-diagnostic",
        vec![observed.clone(), observed],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Waiting
    );
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Settled
    );
    assert!(remediation.deliveries.lock().unwrap().is_empty());
}

#[test]
fn successful_latest_attempt_suppresses_an_older_failed_attempt() {
    let mut observed = merged_snapshot();
    observed.check_runs.insert(
        0,
        GithubValidationCheckRun::new(
            GithubOpaqueId::new("check:attempt-1"),
            "Post-Merge Gate",
            GithubCommitSha::new(MERGE),
            GithubValidationRunStatus::Failed,
        )
        .with_attempt_metadata(
            Some("github-actions".to_string()),
            Some(GithubOpaqueId::new("check-suite:attempt-1")),
            Some("2026-08-07T23:00:00Z".to_string()),
            Some("2026-08-07T23:01:00Z".to_string()),
        ),
    );
    let (repo, observation, remediation) = setup(
        "validation-latest-attempt-success",
        vec![observed.clone(), observed],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Waiting
    );
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Settled
    );
    assert!(remediation.deliveries.lock().unwrap().is_empty());
}

#[test]
fn active_rerun_cannot_settle_from_the_previous_attempts_successful_gate() {
    let suite_id = GithubOpaqueId::new("check-suite:ci");
    let mut active = merged_snapshot();
    active.workflow_runs = vec![
        GithubValidationWorkflowRun::new(
            GithubOpaqueId::new("workflow:rerun"),
            "Native PR Checks",
            GithubCommitSha::new(MERGE),
            GithubValidationRunStatus::InProgress,
        )
        .with_attempt_metadata(
            2,
            Some("2026-08-08T00:00:00Z".to_string()),
            Some("2026-08-08T01:01:00Z".to_string()),
        )
        .with_attempt_correlation(
            Some(suite_id.clone()),
            Some("2026-08-08T01:00:00Z".to_string()),
        ),
    ];
    let mut completed = active.clone();
    completed.check_runs.push(
        GithubValidationCheckRun::new(
            GithubOpaqueId::new("check:ci-attempt-2"),
            "Post-Merge Gate",
            GithubCommitSha::new(MERGE),
            GithubValidationRunStatus::Succeeded,
        )
        .with_attempt_metadata(
            Some("github-actions".to_string()),
            Some(suite_id),
            Some("2026-08-08T01:10:00Z".to_string()),
            Some("2026-08-08T01:11:00Z".to_string()),
        ),
    );
    completed.workflow_runs[0].status = GithubValidationRunStatus::Succeeded;
    let (repo, observation, remediation) = setup(
        "validation-active-rerun",
        vec![active.clone(), active, completed.clone(), completed],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    for revision in 1..=3 {
        assert_eq!(
            service
                .poll_pr_validation(&observation, &remediation, request(&repo, revision, HEAD_A))
                .unwrap(),
            PrValidationPollResult::Waiting
        );
    }
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 4, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Settled
    );
    assert!(remediation.deliveries.lock().unwrap().is_empty());
}

#[test]
fn malicious_provider_identity_is_bounded_before_remediation() {
    let mut failed = snapshot(HEAD_A, GithubValidationRunStatus::Failed);
    failed.check_runs[0].id = GithubOpaqueId::new(format!("evil\n{}", "x".repeat(2_000)));
    let (repo, observation, remediation) = setup("validation-provider-bounds", vec![failed]);
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    assert!(matches!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap(),
        PrValidationPollResult::RemediationRequested { .. }
    ));
    let delivery = remediation.deliveries.lock().unwrap()[0].clone();
    assert!(delivery.finding_key.provider_event_id().len() <= 96);
    assert!(
        !delivery
            .finding_key
            .provider_event_id()
            .contains(['\r', '\n'])
    );
    assert!(delivery.summary.chars().count() < 256);
    assert!(!delivery.summary.contains(['\r', '\n']));
}

#[test]
fn merge_sha_evidence_mismatch_persists_failure_before_checkpointing() {
    let mut merged = merged_snapshot();
    merged.evidence_sha = GithubCommitSha::new(HEAD_A);
    merged.check_runs[0].target_sha = GithubCommitSha::new(HEAD_A);
    let (repo, observation, remediation) = setup("validation-merge-evidence", vec![merged]);
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Failed
    );
    let record = service
        .recover_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(record.phase(), PrValidationPhase::Failed);
    assert_eq!(
        record.operator_summary().reason,
        "trusted validation identity did not match the record"
    );
    assert_eq!(record.observation_revision(), 0);
}

#[test]
fn closed_pr_with_distributor_attestation_continues_post_merge_validation() {
    let mut closed = snapshot(HEAD_A, GithubValidationRunStatus::Succeeded);
    closed.merge_state = GithubPrMergeState::Closed;
    closed.evidence_sha = GithubCommitSha::new(MERGE);
    closed.check_runs[0].target_sha = GithubCommitSha::new(MERGE);
    let (repo, observation, remediation) = setup(
        "validation-distributor-attested-closed",
        vec![closed.clone(), closed],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &distributor_attested_record(),
        )
        .unwrap();

    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Waiting
    );
    assert_eq!(
        observation.requests.lock().unwrap()[0]
            .evidence_sha
            .as_ref()
            .map(GithubCommitSha::as_str),
        Some(MERGE)
    );
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Settled
    );
    let record = service
        .recover_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(record.phase(), PrValidationPhase::Settled);
    assert_eq!(
        record.evidence_sha().map(PrValidationCommitSha::as_str),
        Some(MERGE)
    );
}

#[test]
fn github_merge_cannot_replace_distributor_integration_authority() {
    let (repo, observation, remediation) = setup(
        "validation-integration-authority-conflict",
        vec![merged_snapshot()],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &distributor_attested_record(),
        )
        .unwrap();

    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Failed
    );
    let record = service
        .recover_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(record.phase(), PrValidationPhase::Failed);
    assert_eq!(
        record.operator_summary().reason,
        "integration authority conflicts with trusted evidence"
    );
}

#[test]
fn terminal_provider_check_and_catch_up_settle_once() {
    let merged = merged_snapshot();
    let mut late_event = merged.clone();
    late_event.check_runs.push(GithubValidationCheckRun::new(
        GithubOpaqueId::new("check:late-success"),
        "late successful check",
        GithubCommitSha::new(MERGE),
        GithubValidationRunStatus::Succeeded,
    ));
    let (repo, observation, remediation) = setup(
        "validation-post-merge",
        vec![merged, late_event.clone(), late_event],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Waiting
    );
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Waiting
    );
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 3, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Settled
    );
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 3, HEAD_A))
            .unwrap(),
        PrValidationPollResult::StaleDeliveryIgnored
    );
    let record = service
        .recover_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            &PrValidationRecordKey::new("acme/widgets#42").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(record.phase(), PrValidationPhase::Settled);
}

#[test]
fn terminal_pagination_cursor_is_reused_for_stable_catch_up() {
    let merged = merged_snapshot();
    let mut partial = merged.clone();
    partial.sources[0].status = GithubValidationSourceStatus::Paginated;
    partial.sources[0].next_cursor = Some(GithubValidationCursor::new("source:page-2"));
    partial.next_cursor = Some(GithubValidationCursor::new("page-2"));

    let (repo, observation, remediation) = setup(
        "validation-terminal-pagination-cursor",
        vec![partial, merged.clone(), merged],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 1, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Waiting
    );
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 2, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Waiting
    );
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 3, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Settled
    );
    assert_eq!(
        observation
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| {
                request
                    .cursor
                    .as_ref()
                    .map(|cursor| cursor.as_str().to_string())
            })
            .collect::<Vec<_>>(),
        vec![None, Some("page-2".to_string()), Some("page-2".to_string()),]
    );
}

#[test]
fn merge_observation_clears_pre_merge_terminal_cursor() {
    let mut pre_merge_partial = snapshot(HEAD_A, GithubValidationRunStatus::Succeeded);
    pre_merge_partial.sources[0].status = GithubValidationSourceStatus::Paginated;
    pre_merge_partial.sources[0].next_cursor = Some(GithubValidationCursor::new("source:page-2"));
    pre_merge_partial.next_cursor = Some(GithubValidationCursor::new("page-2"));
    let pre_merge_complete = snapshot(HEAD_A, GithubValidationRunStatus::Succeeded);
    let merged = merged_snapshot();
    let (repo, observation, remediation) = setup(
        "validation-merge-clears-terminal-cursor",
        vec![
            pre_merge_partial,
            pre_merge_complete,
            merged.clone(),
            merged,
        ],
    );
    let service = test_parallel_mode_service();
    service
        .persist_pr_validation_record(
            &repo.workspace_dir(),
            &repo.pool_root(),
            None,
            &registered_record(),
        )
        .unwrap();

    for revision in 1..=3 {
        assert_eq!(
            service
                .poll_pr_validation(&observation, &remediation, request(&repo, revision, HEAD_A),)
                .unwrap(),
            PrValidationPollResult::Waiting
        );
    }
    assert_eq!(
        service
            .poll_pr_validation(&observation, &remediation, request(&repo, 4, HEAD_A))
            .unwrap(),
        PrValidationPollResult::Settled
    );
    assert_eq!(
        observation
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| {
                request
                    .cursor
                    .as_ref()
                    .map(|cursor| cursor.as_str().to_string())
            })
            .collect::<Vec<_>>(),
        vec![
            None,
            Some("page-2".to_string()),
            Some("page-2".to_string()),
            None,
        ]
    );
}
