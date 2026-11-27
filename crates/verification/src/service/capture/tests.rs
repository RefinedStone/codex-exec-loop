use std::path::PathBuf;
use std::sync::Mutex;

use super::*;
use crate::evidence::{TaskEvidenceAttempt, TaskEvidenceTree};
use crate::verification::{TaskVerificationExecutionOutcome, TaskVerificationRecipe};

#[derive(Clone, Copy)]
enum Seal {
    Valid,
    Failure,
    WrongHash,
    WrongSize,
    InvalidHash,
}

struct Captured {
    workspace: String,
    attempt_id: String,
    bytes: Vec<u8>,
}

struct Artifacts {
    mode: Seal,
    captures: Mutex<Vec<Captured>>,
}

impl Artifacts {
    fn new(mode: Seal) -> Self {
        Self {
            mode,
            captures: Mutex::new(Vec::new()),
        }
    }
}

impl TaskEvidenceArtifactPort for Artifacts {
    fn prepare_lease_incoming_root(&self, _: &str, _: &str) -> Result<PathBuf> {
        panic!("capture must not prepare worker writable roots")
    }

    fn prepare_artifact(&self, _: &str, _: &TaskEvidenceAttempt) -> Result<PathBuf> {
        panic!("capture must not prepare worker artifacts")
    }

    fn observe_and_seal(
        &self,
        _: &str,
        _: &TaskEvidenceAttempt,
        _: &TaskEvidenceTree,
    ) -> Result<TaskEvidenceArtifact> {
        panic!("capture must use the separate host namespace")
    }

    fn verify_sealed(
        &self,
        _: &str,
        _: &TaskEvidenceAttempt,
        _: &TaskEvidenceArtifact,
    ) -> Result<()> {
        panic!("capture must not verify worker artifacts")
    }

    fn seal_host_capture(
        &self,
        workspace: &str,
        attempt_id: &str,
        bytes: &[u8],
    ) -> Result<TaskEvidenceArtifact> {
        self.captures.lock().unwrap().push(Captured {
            workspace: workspace.into(),
            attempt_id: attempt_id.into(),
            bytes: bytes.to_vec(),
        });
        let mut artifact = TaskEvidenceArtifact {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size_bytes: bytes.len() as u64,
        };
        match self.mode {
            Seal::Valid => {}
            Seal::Failure => anyhow::bail!("seal unavailable"),
            Seal::WrongHash => artifact.sha256 = "a".repeat(64),
            Seal::WrongSize => artifact.size_bytes += 1,
            Seal::InvalidHash => artifact.sha256 = "invalid".into(),
        }
        Ok(artifact)
    }
}

fn manifest() -> TaskVerificationExecutionManifest {
    TaskVerificationExecutionManifest {
        recipe: TaskVerificationRecipe::GitDiffCheckV1,
        base_commit_sha: "a".repeat(40),
        base_tree_sha: "b".repeat(40),
        source: TaskEvidenceTree {
            head_commit_sha: "c".repeat(40),
            tree_sha: "d".repeat(40),
            worktree_clean: false,
        },
        git_binary_sha256: "a".repeat(64),
        command_sha256: "b".repeat(64),
        environment_sha256: "c".repeat(64),
        configuration_sha256: "d".repeat(64),
    }
}

fn observed(stdout: &[u8], stderr: &[u8]) -> Box<TaskVerificationObservation> {
    let stream = |bytes: &[u8]| TaskVerificationStream {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size_bytes: bytes.len() as u64,
        complete: true,
    };
    Box::new(TaskVerificationObservation {
        outcome: TaskVerificationExecutionOutcome::Exited { exit_code: 0 },
        started_at: "2026-11-23T12:00:00Z".into(),
        finished_at: "2026-11-23T12:00:01Z".into(),
        duration_ms: 1000,
        stdout: stream(stdout),
        stderr: stream(stderr),
    })
}

#[test]
fn capture_seals_exact_binary_streams_and_preserves_the_existing_wire_shape() {
    let artifacts = Artifacts::new(Seal::Valid);
    let service = TaskVerificationCaptureService::new("/workspace", &artifacts);
    let manifest = manifest();
    let observation = observed(&[0, 255, 10], b"stderr\n");
    let expected = TaskVerificationResult::Executed {
        observation: observation.clone(),
    };
    let (result, artifact) = service.capture(
        "attempt-1",
        &manifest,
        observation,
        &[0, 255, 10],
        b"stderr\n",
    );

    assert_eq!(result, expected);
    let captures = artifacts.captures.lock().unwrap();
    assert_eq!(captures.len(), 1);
    let captured = &captures[0];
    assert_eq!(captured.workspace, "/workspace");
    assert_eq!(captured.attempt_id, "attempt-1");
    let artifact = artifact.unwrap();
    artifact.validate().unwrap();
    assert_eq!(artifact.size_bytes, captured.bytes.len() as u64);
    assert_eq!(
        artifact.sha256,
        format!("{:x}", Sha256::digest(&captured.bytes))
    );
    let value: serde_json::Value = serde_json::from_slice(&captured.bytes).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["attempt_id"], "attempt-1");
    assert_eq!(value["manifest"], serde_json::to_value(&manifest).unwrap());
    let TaskVerificationResult::Executed { observation } = result else {
        unreachable!()
    };
    assert_eq!(
        value["observation"],
        serde_json::to_value(observation).unwrap()
    );
    assert_eq!(value["stdout_base64"], "AP8K");
    assert_eq!(value["stderr_base64"], "c3RkZXJyCg==");
    let mut keys = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "attempt_id",
            "manifest",
            "observation",
            "schema_version",
            "stderr_base64",
            "stdout_base64"
        ]
    );
}

#[test]
fn malformed_observations_are_interrupted_without_sealing_or_claiming_not_run() {
    for invalid in 0..3 {
        let artifacts = Artifacts::new(Seal::Valid);
        let service = TaskVerificationCaptureService::new("/workspace", &artifacts);
        let mut observation = observed(b"", b"");
        match invalid {
            0 => observation.started_at = "invalid".into(),
            1 => observation.stdout.sha256 = "invalid".into(),
            _ => observation.stderr.size_bytes = MAX_TASK_VERIFICATION_STREAM_BYTES + 1,
        }
        let (result, artifact) = service.capture("attempt", &manifest(), observation, b"", b"");
        assert_eq!(result, TaskVerificationResult::Interrupted {});
        assert!(artifact.is_none());
        assert!(artifacts.captures.lock().unwrap().is_empty());
    }
}

#[test]
fn stream_size_or_hash_mismatch_retains_execution_without_a_seal() {
    for (stdout, stderr) in [
        (&b"longer"[..], &b"b"[..]),
        (&b"x"[..], &b"b"[..]),
        (&b"a"[..], &b"longer"[..]),
        (&b"a"[..], &b"x"[..]),
    ] {
        let artifacts = Artifacts::new(Seal::Valid);
        let service = TaskVerificationCaptureService::new("/workspace", &artifacts);
        let observation = observed(b"a", b"b");
        let expected = TaskVerificationResult::Executed {
            observation: observation.clone(),
        };
        let (result, artifact) =
            service.capture("attempt", &manifest(), observation, stdout, stderr);
        assert_eq!(result, expected);
        assert!(artifact.is_none());
        assert!(artifacts.captures.lock().unwrap().is_empty());
    }
}

#[test]
fn a_failed_or_mismatched_artifact_cannot_remove_execution_or_prove_a_pass() {
    for mode in [
        Seal::Failure,
        Seal::WrongHash,
        Seal::WrongSize,
        Seal::InvalidHash,
    ] {
        let artifacts = Artifacts::new(mode);
        let service = TaskVerificationCaptureService::new("/workspace", &artifacts);
        let observation = observed(b"", b"");
        let expected = TaskVerificationResult::Executed {
            observation: observation.clone(),
        };
        let (result, artifact) = service.capture("attempt", &manifest(), observation, b"", b"");
        assert_eq!(result, expected);
        assert!(artifact.is_none());
        assert_eq!(artifacts.captures.lock().unwrap().len(), 1);
    }
}

#[test]
fn capture_preserves_failed_outcomes_and_incomplete_streams() {
    for outcome in [
        TaskVerificationExecutionOutcome::Exited { exit_code: 2 },
        TaskVerificationExecutionOutcome::TimedOut {},
        TaskVerificationExecutionOutcome::Cancelled {},
        TaskVerificationExecutionOutcome::OutputLimit {},
        TaskVerificationExecutionOutcome::Signalled {},
        TaskVerificationExecutionOutcome::IoFailure {},
    ] {
        let artifacts = Artifacts::new(Seal::Valid);
        let service = TaskVerificationCaptureService::new("/workspace", &artifacts);
        let mut observation = observed(b"", b"");
        observation.outcome = outcome;
        observation.stdout.complete = false;
        let expected = TaskVerificationResult::Executed {
            observation: observation.clone(),
        };
        let (result, artifact) = service.capture("attempt", &manifest(), observation, b"", b"");
        assert_eq!(result, expected);
        assert!(artifact.is_some());
    }
}

#[test]
fn capture_accepts_the_stream_limit_and_rejects_an_oversized_envelope() {
    let artifacts = Artifacts::new(Seal::Valid);
    let service = TaskVerificationCaptureService::new("/workspace", &artifacts);
    let stdout = vec![0; MAX_TASK_VERIFICATION_STREAM_BYTES as usize];
    let (result, artifact) =
        service.capture("attempt", &manifest(), observed(&stdout, b""), &stdout, b"");
    assert!(matches!(result, TaskVerificationResult::Executed { .. }));
    assert!(artifact.is_some());
    assert_eq!(artifacts.captures.lock().unwrap().len(), 1);

    let mut oversized = manifest();
    oversized.base_commit_sha = "a".repeat(MAX_TASK_EVIDENCE_ARTIFACT_BYTES as usize);
    let observation = observed(b"", b"");
    let expected = TaskVerificationResult::Executed {
        observation: observation.clone(),
    };
    let (result, artifact) = service.capture("attempt", &oversized, observation, b"", b"");
    assert_eq!(result, expected);
    assert!(artifact.is_none());
    assert_eq!(artifacts.captures.lock().unwrap().len(), 1);
}
