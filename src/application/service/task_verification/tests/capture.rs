use super::*;
use sha2::{Digest, Sha256};

#[test]
fn host_verification_capture_mismatch_retains_the_original_execution_without_an_artifact() {
    let f = Fixture::new();
    for hash_only_mismatch in [false, true] {
        let TaskVerificationResult::Executed { mut observation } = fixtures::executed() else {
            unreachable!()
        };
        if hash_only_mismatch {
            observation.stdout.size_bytes = 1;
            observation.stdout.sha256 = format!("{:x}", Sha256::digest(b"a"));
        }
        observation.validate().unwrap();
        let expected = TaskVerificationResult::Executed {
            observation: observation.clone(),
        };
        let (actual, artifact) = execution::capture_executed(
            f.service.dependencies.as_ref(),
            &"2".repeat(64),
            &fixtures::manifest(),
            observation,
            b"b",
            b"",
        );
        assert_eq!(actual, expected);
        assert!(artifact.is_none());
    }
    assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn host_verification_invalid_execution_observation_is_interrupted_without_an_artifact() {
    let f = Fixture::new();
    for invalid_time in [false, true] {
        let TaskVerificationResult::Executed { mut observation } = fixtures::executed() else {
            unreachable!()
        };
        if invalid_time {
            observation.started_at = "invalid-time".into();
        } else {
            observation.stdout.sha256 = "invalid-hash".into();
        }
        assert!(observation.validate().is_err());
        let (result, artifact) = execution::capture_executed(
            f.service.dependencies.as_ref(),
            &"2".repeat(64),
            &fixtures::manifest(),
            observation,
            b"",
            b"",
        );
        assert_eq!(result, TaskVerificationResult::Interrupted {});
        assert!(artifact.is_none());
    }
}

#[test]
fn host_verification_seal_failure_preserves_the_execution_fact_without_a_verified_artifact() {
    let f = Fixture::new();
    let attempt_id = "2".repeat(64);
    assert!(
        f.service
            .dependencies
            .artifacts
            .seal_host_capture("/repo", &attempt_id, b"capture")
            .is_err()
    );
    let expected = fixtures::executed();
    let TaskVerificationResult::Executed { observation } = expected.clone() else {
        unreachable!()
    };
    observation.validate().unwrap();
    let (result, artifact) = execution::capture_executed(
        f.service.dependencies.as_ref(),
        &attempt_id,
        &fixtures::manifest(),
        observation,
        b"",
        b"",
    );
    assert_eq!(result, expected);
    assert!(artifact.is_none());
    assert!(f.repository.outbox.lock().unwrap().is_empty());
}
