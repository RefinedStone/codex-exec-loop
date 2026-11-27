use anyhow::Result;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::evidence::{MAX_TASK_EVIDENCE_ARTIFACT_BYTES, TaskEvidenceArtifact, TaskEvidenceError};
use crate::ports::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::verification::{
    MAX_TASK_VERIFICATION_STREAM_BYTES, TaskVerificationExecutionManifest,
    TaskVerificationObservation, TaskVerificationResult, TaskVerificationStream,
};

/// Captures an observed execution through the artifact port. The caller retains
/// responsibility for reservation, current lease/authority, and committing the result.
/// A matching sealed artifact records execution facts; it grants no delivery authority.
pub struct TaskVerificationCaptureService<'a> {
    workspace: &'a str,
    artifacts: &'a dyn TaskEvidenceArtifactPort,
}

impl<'a> TaskVerificationCaptureService<'a> {
    pub fn new(workspace: &'a str, artifacts: &'a dyn TaskEvidenceArtifactPort) -> Self {
        Self {
            workspace,
            artifacts,
        }
    }

    pub fn capture(
        &self,
        attempt_id: &str,
        manifest: &TaskVerificationExecutionManifest,
        observation: Box<TaskVerificationObservation>,
        stdout: &[u8],
        stderr: &[u8],
    ) -> (TaskVerificationResult, Option<TaskEvidenceArtifact>) {
        if observation.validate().is_err() {
            // The process ran, but there is no valid observation to retain. Never turn
            // this uncertainty into either NotRun or a successful execution.
            return (TaskVerificationResult::Interrupted {}, None);
        }
        let artifact = if stream_matches(&observation.stdout, stdout)
            && stream_matches(&observation.stderr, stderr)
        {
            self.seal_capture(attempt_id, manifest, &observation, stdout, stderr)
                .ok()
        } else {
            None
        };
        // A failed capture retains the execution fact, without proof of a current pass.
        (TaskVerificationResult::Executed { observation }, artifact)
    }

    fn seal_capture(
        &self,
        attempt_id: &str,
        manifest: &TaskVerificationExecutionManifest,
        observation: &TaskVerificationObservation,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<TaskEvidenceArtifact> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "attempt_id": attempt_id,
            "manifest": manifest,
            "observation": observation,
            "stdout_base64": base64::engine::general_purpose::STANDARD.encode(stdout),
            "stderr_base64": base64::engine::general_purpose::STANDARD.encode(stderr),
        }))?;
        if bytes.len() as u64 > MAX_TASK_EVIDENCE_ARTIFACT_BYTES {
            return Err(TaskEvidenceError::Limit.into());
        }
        let artifact = self
            .artifacts
            .seal_host_capture(self.workspace, attempt_id, &bytes)?;
        artifact.validate()?;
        if artifact.size_bytes != bytes.len() as u64
            || artifact.sha256 != format!("{:x}", Sha256::digest(&bytes))
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
        Ok(artifact)
    }
}

fn stream_matches(expected: &TaskVerificationStream, actual: &[u8]) -> bool {
    actual.len() as u64 <= MAX_TASK_VERIFICATION_STREAM_BYTES
        && expected.size_bytes == actual.len() as u64
        && expected.sha256 == format!("{:x}", Sha256::digest(actual))
}

#[cfg(test)]
mod tests;
