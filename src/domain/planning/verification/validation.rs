use chrono::DateTime;

use super::*;
use crate::domain::planning::evidence::{
    MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION, MAX_TASK_EVIDENCE_REVISION, TaskEvidenceError,
    validate_evidence_identifier, validate_evidence_request_id, validate_evidence_token,
};

pub(crate) fn validate_time(value: &str) -> Result<(), TaskEvidenceError> {
    if value.len() > 128 || DateTime::parse_from_rfc3339(value).is_err() {
        return Err(TaskEvidenceError::Invalid);
    }
    Ok(())
}

impl TaskVerificationAttempt {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_evidence_token(&self.attempt_id)?;
        validate_evidence_request_id(&self.request_id)?;
        self.scope.validate()?;
        validate_evidence_identifier(&self.criterion_id)?;
        validate_evidence_identifier(&self.slot_id)?;
        validate_evidence_token(&self.lease_generation)?;
        validate_evidence_identifier(&self.session_key)?;
        validate_time(&self.requested_at)?;
        if self.scope.acceptance_revision == 0
            || !(1..=MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION).contains(&self.attempt)
        {
            return Err(TaskEvidenceError::Invalid);
        }
        Ok(())
    }
}

impl TaskVerificationExecutionManifest {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        self.source.validate()?;
        TaskEvidenceTree {
            head_commit_sha: self.base_commit_sha.clone(),
            tree_sha: self.base_tree_sha.clone(),
            worktree_clean: true,
        }
        .validate()?;
        let length = self.source.tree_sha.len();
        if self.base_commit_sha.len() != length
            || self.base_tree_sha.len() != length
            || self.source.head_commit_sha.len() != length
        {
            return Err(TaskEvidenceError::Invalid);
        }
        for digest in [
            &self.git_binary_sha256,
            &self.command_sha256,
            &self.environment_sha256,
            &self.configuration_sha256,
        ] {
            validate_evidence_token(digest)?;
        }
        Ok(())
    }
}

impl TaskVerificationStream {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_evidence_token(&self.sha256)?;
        if self.size_bytes > MAX_TASK_VERIFICATION_STREAM_BYTES {
            return Err(TaskEvidenceError::Limit);
        }
        if self.size_bytes == 0
            && self.sha256 != "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        {
            return Err(TaskEvidenceError::Invalid);
        }
        Ok(())
    }
}

impl TaskVerificationObservation {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_time(&self.started_at)?;
        validate_time(&self.finished_at)?;
        // Wall-clock adjustments may reverse the timestamps; duration is monotonic.
        if self.duration_ms > MAX_TASK_EVIDENCE_REVISION {
            return Err(TaskEvidenceError::Limit);
        }
        self.stdout.validate()?;
        self.stderr.validate()
    }
}

impl TaskVerificationReceipt {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        self.attempt.validate()?;
        if self.event_id != format!("task-verification-finished:{}", self.attempt.attempt_id)
            || self.evidence_revision == 0
            || self.evidence_revision > MAX_TASK_EVIDENCE_REVISION
        {
            return Err(TaskEvidenceError::Invalid);
        }
        validate_time(&self.finished_at)?;
        if let Some(manifest) = &self.manifest {
            manifest.validate()?;
            if manifest.recipe != self.attempt.recipe {
                return Err(TaskEvidenceError::Invalid);
            }
        }
        match &self.result {
            TaskVerificationResult::Executed { observation } => {
                if self.manifest.is_none() {
                    return Err(TaskEvidenceError::Invalid);
                }
                observation.validate()?;
                if let Some(artifact) = &self.artifact {
                    artifact.validate()?;
                }
            }
            TaskVerificationResult::NotRun { .. } | TaskVerificationResult::Interrupted {} => {
                if self.artifact.is_some() {
                    return Err(TaskEvidenceError::Invalid);
                }
            }
        }
        Ok(())
    }

    /// A historical successful execution is not current authority. Callers must also verify
    /// the current scope, source/base, environment and the sealed artifact before using it.
    pub fn passed(&self) -> bool {
        self.validate().is_ok()
            && self.artifact.is_some()
            && matches!(
                &self.result,
                TaskVerificationResult::Executed { observation }
                    if observation.outcome == (TaskVerificationExecutionOutcome::Exited { exit_code: 0 })
                        && observation.stdout.complete && observation.stderr.complete
            )
    }
}
