use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex;

use anyhow::Result;

use super::{TempGitRepo, test_parallel_mode_service};
use crate::application::port::outbound::github_pr_validation_port::{
    GithubPrMergeState, GithubPrValidationObservationRequest, GithubPrValidationPort,
    GithubPrValidationSnapshot, GithubValidationActivity, GithubValidationActivityKind,
    GithubValidationCheckRun, GithubValidationRunStatus, GithubValidationSource,
    GithubValidationSourceObservation, GithubValidationSourceStatus,
};
use crate::application::port::outbound::pr_validation_remediation_port::{
    PrValidationRemediationPort, PrValidationRemediationRequest,
};
use crate::application::service::parallel_mode::{PrValidationPollRequest, PrValidationPollResult};
use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};
use crate::domain::parallel_mode::{
    PrValidationCommitSha, PrValidationEvent, PrValidationPhase, PrValidationRecord,
    PrValidationRecordKey, PrValidationTarget, PrValidationTargetShaSnapshot,
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
    ) -> Result<GithubPrValidationSnapshot> {
        assert!(
            !self.pool_root.join(".leases").exists(),
            "validation polling must not hold a worktree/slot lease while waiting"
        );
        self.requests.lock().unwrap().push(request.clone());
        Ok(self.snapshots.lock().unwrap().pop_front().unwrap())
    }
}

#[derive(Default)]
struct RemediationPort {
    unique: Mutex<BTreeSet<String>>,
    deliveries: Mutex<Vec<PrValidationRemediationRequest>>,
    after_delivery: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl PrValidationRemediationPort for RemediationPort {
    fn request_remediation(&self, request: &PrValidationRemediationRequest) -> Result<()> {
        let mut unique = self.unique.lock().unwrap();
        if unique.insert(request.idempotency_key.as_str().to_string()) {
            self.deliveries.lock().unwrap().push(request.clone());
        }
        drop(unique);
        if let Some(after_delivery) = self.after_delivery.lock().unwrap().take() {
            after_delivery();
        }
        Ok(())
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
        merge_state: GithubPrMergeState::Open,
        merge_sha: None,
        activities: Vec::new(),
        check_runs: vec![GithubValidationCheckRun::new(
            GithubOpaqueId::new("check:ci"),
            "ci",
            GithubCommitSha::new(sha),
            status,
        )],
        workflow_runs: Vec::new(),
        sources: sources(),
        next_cursor: None,
    }
}

fn merged_snapshot() -> GithubPrValidationSnapshot {
    let mut snapshot = snapshot(HEAD_A, GithubValidationRunStatus::Succeeded);
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
fn terminal_provider_check_and_catch_up_settle_once() {
    let merged = merged_snapshot();
    let mut late_event = merged.clone();
    late_event.activities.push(
        GithubValidationActivity::new(
            GithubOpaqueId::new("review:late"),
            GithubValidationActivityKind::Review,
            "2026-08-07T22:00:00Z",
        )
        .with_commit_sha(GithubCommitSha::new(HEAD_A)),
    );
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
