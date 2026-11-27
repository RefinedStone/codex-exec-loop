use super::*;
use crate::acceptance::{
    TaskAcceptanceChannel, TaskAcceptanceCriterion, normalize_task_acceptance,
};
use crate::evidence::{TaskEvidenceError, criteria_sha256};
use sha2::{Digest, Sha256};

fn receipt() -> TaskVerificationReceipt {
    let empty = || TaskVerificationStream {
        sha256: format!("{:x}", Sha256::digest([])),
        size_bytes: 0,
        complete: true,
    };
    TaskVerificationReceipt {
        event_id: format!("task-verification-finished:{}", "a".repeat(64)),
        evidence_revision: 3,
        attempt: TaskVerificationAttempt {
            attempt_id: "a".repeat(64),
            request_id: "request-1".into(),
            scope: TaskEvidenceScope {
                task_id: "task-1".into(),
                scope_id: "b".repeat(64),
                acceptance_revision: 1,
                criteria_sha256: "c".repeat(64),
            },
            criterion_id: "criterion one".into(),
            recipe: TaskVerificationRecipe::GitDiffCheckV1,
            attempt: 1,
            slot_id: "slot-1".into(),
            lease_generation: "d".repeat(64),
            session_key: "session-1".into(),
            requested_at: "2026-11-23T12:00:00Z".into(),
        },
        manifest: Some(TaskVerificationExecutionManifest {
            recipe: TaskVerificationRecipe::GitDiffCheckV1,
            base_commit_sha: "a".repeat(40),
            base_tree_sha: "b".repeat(40),
            source: TaskEvidenceTree {
                head_commit_sha: "a".repeat(40),
                tree_sha: "c".repeat(40),
                worktree_clean: false,
            },
            git_binary_sha256: "1".repeat(64),
            command_sha256: "2".repeat(64),
            environment_sha256: "3".repeat(64),
            configuration_sha256: "4".repeat(64),
        }),
        result: TaskVerificationResult::Executed {
            observation: Box::new(TaskVerificationObservation {
                outcome: TaskVerificationExecutionOutcome::Exited { exit_code: 0 },
                started_at: "2026-11-23T12:00:01Z".into(),
                finished_at: "2026-11-23T12:00:02Z".into(),
                duration_ms: 1000,
                stdout: empty(),
                stderr: empty(),
            }),
        },
        artifact: Some(TaskEvidenceArtifact {
            sha256: "e".repeat(64),
            size_bytes: 100,
        }),
        finished_at: "2026-11-23T12:00:03Z".into(),
    }
}

#[test]
fn host_verification_requires_a_manifest_seal_and_complete_successful_execution() {
    let original = receipt();
    assert!(original.passed());
    let mut missing_seal = original.clone();
    missing_seal.artifact = None;
    missing_seal.validate().unwrap();
    assert!(!missing_seal.passed());
    let mut missing_manifest = original.clone();
    missing_manifest.manifest = None;
    assert!(missing_manifest.validate().is_err());
    for outcome in [
        TaskVerificationExecutionOutcome::Exited { exit_code: 2 },
        TaskVerificationExecutionOutcome::TimedOut {},
        TaskVerificationExecutionOutcome::Cancelled {},
        TaskVerificationExecutionOutcome::OutputLimit {},
        TaskVerificationExecutionOutcome::Signalled {},
        TaskVerificationExecutionOutcome::IoFailure {},
    ] {
        let mut value = original.clone();
        let TaskVerificationResult::Executed { observation } = &mut value.result else {
            unreachable!()
        };
        observation.outcome = outcome;
        value.validate().unwrap();
        assert!(!value.passed());
    }
    let mut incomplete = original;
    let TaskVerificationResult::Executed { observation } = &mut incomplete.result else {
        unreachable!()
    };
    observation.stdout.complete = false;
    assert!(!incomplete.passed());
}

#[test]
fn host_verification_closed_variants_reject_extra_authority_fields() {
    for kind in [
        "timed_out",
        "cancelled",
        "output_limit",
        "signalled",
        "io_failure",
    ] {
        assert!(
            serde_json::from_value::<TaskVerificationExecutionOutcome>(
                serde_json::json!({"kind":kind,"by":"human"})
            )
            .is_err()
        );
    }
    assert!(
        serde_json::from_value::<TaskVerificationResult>(
            serde_json::json!({"kind":"interrupted","approved":true})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<TaskVerificationResult>(
            serde_json::json!({"kind":"not_run","reason":"allow"})
        )
        .is_err()
    );
    let mut value = serde_json::to_value(receipt()).unwrap();
    value["owner_token"] = serde_json::json!("secret");
    assert!(serde_json::from_value::<TaskVerificationReceipt>(value).is_err());
}

#[test]
fn host_verification_bounds_hashes_oid_formats_and_capture_sizes() {
    let mut value = receipt();
    value.manifest.as_mut().unwrap().base_tree_sha = "b".repeat(64);
    assert!(value.validate().is_err());
    let mut value = receipt();
    value.manifest.as_mut().unwrap().command_sha256 = "A".repeat(64);
    assert!(value.validate().is_err());
    let mut value = receipt();
    let TaskVerificationResult::Executed { observation } = &mut value.result else {
        unreachable!()
    };
    observation.stdout.size_bytes = MAX_TASK_VERIFICATION_STREAM_BYTES + 1;
    assert_eq!(value.validate(), Err(TaskEvidenceError::Limit));
    let mut value = receipt();
    value.event_id = "task-verification-finished:other".into();
    assert!(value.validate().is_err());
}

#[test]
fn host_verification_keeps_monotonic_duration_when_wall_clock_reverses() {
    let mut value = receipt();
    let TaskVerificationResult::Executed { observation } = &mut value.result else {
        unreachable!()
    };
    observation.finished_at = "2026-11-23T11:59:58Z".into();
    value.finished_at = "2026-11-23T11:59:59Z".into();
    value.validate().unwrap();
    assert!(value.passed());
}

#[test]
fn host_verification_absent_recipe_preserves_legacy_criteria_bytes_and_hash() {
    let old =
        r#"[{"id":"A01","scenario":"Check the result","channel":"command","essential":true}]"#;
    let criteria: Vec<TaskAcceptanceCriterion> = serde_json::from_str(old).unwrap();
    assert_eq!(criteria[0].verification_recipe, None);
    assert_eq!(serde_json::to_string(&criteria).unwrap(), old);
    assert_eq!(
        criteria_sha256(&criteria).unwrap(),
        format!("{:x}", Sha256::digest(old.as_bytes()))
    );
    let mut opted_in = criteria.clone();
    opted_in[0].verification_recipe = Some(TaskVerificationRecipe::GitDiffCheckV1);
    assert_ne!(
        criteria_sha256(&opted_in).unwrap(),
        criteria_sha256(&criteria).unwrap()
    );
    for channel in [
        TaskAcceptanceChannel::Data,
        TaskAcceptanceChannel::Http,
        TaskAcceptanceChannel::Screen,
    ] {
        opted_in[0].channel = channel;
        assert!(normalize_task_acceptance(&opted_in).is_err());
    }
}
