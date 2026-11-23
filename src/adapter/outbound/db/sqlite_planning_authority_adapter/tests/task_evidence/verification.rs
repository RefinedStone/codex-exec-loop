use super::*;
use crate::application::port::outbound::task_verification_repository_port::*;
use crate::domain::parallel_mode::{
    ParallelModeDeliveryTargetSnapshot, ParallelModeRepositoryVisibility,
};
use crate::domain::planning::verification::*;
use sha2::{Digest, Sha256};

mod captured_lease;
mod durability;
mod failure_limit;
mod fences;

struct HostFixture {
    base: Fixture,
    owner: String,
}

impl HostFixture {
    fn new(label: &str) -> Self {
        let mut base = Fixture::new(label);
        let mut task = base.snapshot().tasks[0].task.clone();
        for criterion in &mut task.acceptance {
            criterion.verification_recipe = Some(TaskVerificationRecipe::GitDiffCheckV1);
        }
        task.acceptance_revision += 1;
        base.save(vec![task]);
        base.lease =
            base.lease
                .clone()
                .with_delivery_target(ParallelModeDeliveryTargetSnapshot::new(
                    "origin",
                    "owner/repo",
                    ParallelModeRepositoryVisibility::Private,
                    "prerelease",
                    "b".repeat(40),
                ));
        base.adapter
            .upsert_runtime_slot_lease(&base.workspace, &base.lease)
            .unwrap();
        let owner = "e".repeat(64);
        assert!(
            base.adapter
                .recover_incomplete_host_verifications(&base.workspace, &owner, NOW)
                .unwrap()
                .is_empty()
        );
        Self { base, owner }
    }

    fn request(&self, id: &str) -> TaskVerificationClaimCommit {
        let scope = self.base.snapshot().tasks[0].scope.clone();
        TaskVerificationClaimCommit {
            request_id: id.into(),
            task_id: scope.task_id,
            criterion_id: "criterion one".into(),
            scope_id: scope.scope_id,
            expected_acceptance_revision: scope.acceptance_revision,
            expected_lease: None,
            owner_token: self.owner.clone(),
            at: NOW.into(),
        }
    }

    fn claim(&self, id: &str) -> TaskVerificationClaimResult {
        self.base
            .adapter
            .claim_host_verification(&self.base.workspace, &self.request(id))
            .unwrap()
    }

    fn bind(&self, claim: &TaskVerificationClaimResult) -> TaskVerificationBindCommit {
        let bind = TaskVerificationBindCommit {
            execution: claim.execution.clone().unwrap(),
            manifest: manifest(),
            at: NOW.into(),
        };
        assert!(
            self.base
                .adapter
                .bind_host_verification(&self.base.workspace, &bind)
                .unwrap()
        );
        bind
    }

    fn finish(&self, bind: &TaskVerificationBindCommit) -> TaskVerificationFinishCommit {
        TaskVerificationFinishCommit {
            execution: bind.execution.clone(),
            result: executed(),
            artifact: Some(TaskEvidenceArtifact {
                sha256: "c".repeat(64),
                size_bytes: 123,
            }),
            at: NOW.into(),
        }
    }
}

pub(super) fn manifest() -> TaskVerificationExecutionManifest {
    TaskVerificationExecutionManifest {
        recipe: TaskVerificationRecipe::GitDiffCheckV1,
        base_commit_sha: "b".repeat(40),
        base_tree_sha: "c".repeat(40),
        source: TaskEvidenceTree {
            head_commit_sha: "d".repeat(40),
            tree_sha: "f".repeat(40),
            worktree_clean: false,
        },
        git_binary_sha256: "a".repeat(64),
        command_sha256: "b".repeat(64),
        environment_sha256: "c".repeat(64),
        configuration_sha256: "d".repeat(64),
    }
}

pub(super) fn executed() -> TaskVerificationResult {
    let empty = || TaskVerificationStream {
        sha256: format!("{:x}", Sha256::digest([])),
        size_bytes: 0,
        complete: true,
    };
    TaskVerificationResult::Executed {
        observation: Box::new(TaskVerificationObservation {
            outcome: TaskVerificationExecutionOutcome::Exited { exit_code: 0 },
            started_at: NOW.into(),
            finished_at: NOW.into(),
            duration_ms: 1,
            stdout: empty(),
            stderr: empty(),
        }),
    }
}

#[test]
fn host_verification_claim_bind_finish_and_ack_preserve_one_original_fact() {
    let f = HostFixture::new("host-verification-replay");
    let request = f.request("request-1");
    let claim = f
        .base
        .adapter
        .claim_host_verification(&f.base.workspace, &request)
        .unwrap();
    assert!(claim.changed);
    assert!(claim.stored.manifest.is_none());
    let before = f.base.counters();
    let mut retry = request.clone();
    retry.at = "2026-11-23T11:59:59Z".into();
    let replay = f
        .base
        .adapter
        .claim_host_verification(&f.base.workspace, &retry)
        .unwrap();
    assert!(!replay.changed);
    assert!(replay.execution.is_none());
    assert_eq!(replay.stored, claim.stored);
    assert_eq!(f.base.counters(), before);
    retry.criterion_id = "criterion two".into();
    assert_eq!(
        reason(
            f.base
                .adapter
                .claim_host_verification(&f.base.workspace, &retry)
                .unwrap_err()
        ),
        TaskEvidenceError::Conflict
    );
    let bind = f.bind(&claim);
    assert!(
        !f.base
            .adapter
            .bind_host_verification(&f.base.workspace, &bind)
            .unwrap()
    );
    let finish = f.finish(&bind);
    let saved = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &finish)
        .unwrap();
    assert!(saved.changed);
    assert!(saved.value.passed());
    let after = f.base.counters();
    assert_eq!(after.1, 1);
    assert_eq!(after.2, 123);
    assert_eq!(after.3, 0);
    let mut retry = finish.clone();
    retry.at = "2026-11-23T11:00:00Z".into();
    let replay = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &retry)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.value, saved.value);
    assert_eq!(f.base.counters(), after);
    let events = f
        .base
        .adapter
        .pending_events(&f.base.workspace, 10)
        .unwrap();
    assert_eq!(
        events,
        vec![TaskEvidenceEvent::VerificationFinished {
            receipt: Box::new(saved.value.clone())
        }]
    );
    f.base
        .adapter
        .acknowledge_event(&f.base.workspace, &saved.value.event_id)
        .unwrap();
    assert!(
        f.base
            .adapter
            .pending_events(&f.base.workspace, 10)
            .unwrap()
            .is_empty()
    );
    let latest = &f.base.snapshot().tasks[0].host_verifications;
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].receipt.as_ref(), Some(&saved.value));
    assert!(f.base.snapshot().tasks[0].evidence.is_empty());
}

#[test]
fn host_verification_foreign_tokens_manifest_mutation_and_second_terminal_are_rejected() {
    let f = HostFixture::new("host-verification-authority");
    let claim = f.claim("request-1");
    let mut bind = TaskVerificationBindCommit {
        execution: claim.execution.clone().unwrap(),
        manifest: manifest(),
        at: NOW.into(),
    };
    bind.execution.run_token = "0".repeat(64);
    assert_eq!(
        reason(
            f.base
                .adapter
                .bind_host_verification(&f.base.workspace, &bind)
                .unwrap_err()
        ),
        TaskEvidenceError::Conflict
    );
    bind.execution = claim.execution.clone().unwrap();
    bind.manifest.base_commit_sha = "a".repeat(40);
    assert_eq!(
        reason(
            f.base
                .adapter
                .bind_host_verification(&f.base.workspace, &bind)
                .unwrap_err()
        ),
        TaskEvidenceError::Stale
    );
    let premature = f.finish(&bind);
    assert!(
        f.base
            .adapter
            .finish_host_verification(&f.base.workspace, &premature)
            .is_err()
    );
    let bind = f.bind(&claim);
    let mut changed = bind.clone();
    changed.manifest.command_sha256 = "f".repeat(64);
    assert!(
        f.base
            .adapter
            .bind_host_verification(&f.base.workspace, &changed)
            .is_err()
    );
    let finish = f.finish(&bind);
    let original = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &finish)
        .unwrap();
    let mut altered = finish;
    altered.result = TaskVerificationResult::NotRun {
        reason: TaskVerificationNotRunReason::Cancelled,
    };
    altered.artifact = None;
    assert_eq!(
        reason(
            f.base
                .adapter
                .finish_host_verification(&f.base.workspace, &altered)
                .unwrap_err()
        ),
        TaskEvidenceError::Conflict
    );
    assert_eq!(
        f.base
            .adapter
            .load_host_verification(&f.base.workspace, &claim.stored.attempt.attempt_id)
            .unwrap()
            .unwrap()
            .receipt,
        Some(original.value)
    );
}

#[test]
fn host_verification_missing_seal_keeps_exit_without_a_verified_pass_or_released_reservation() {
    let f = HostFixture::new("host-verification-seal-failure");
    let claim = f.claim("request-1");
    let bind = f.bind(&claim);
    let mut finish = f.finish(&bind);
    finish.artifact = None;
    let saved = f
        .base
        .adapter
        .finish_host_verification(&f.base.workspace, &finish)
        .unwrap();
    assert_eq!(saved.value.result, executed());
    assert!(!saved.value.passed());
    assert_eq!(f.base.counters().3, MAX_TASK_EVIDENCE_ARTIFACT_BYTES);
    assert_eq!(
        f.base
            .adapter
            .pending_events(&f.base.workspace, 10)
            .unwrap()
            .len(),
        1
    );
}
