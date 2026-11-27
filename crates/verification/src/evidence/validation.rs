use chrono::DateTime;
use sha2::{Digest, Sha256};

use super::*;
use crate::acceptance::{TaskAcceptanceCriterion, normalize_task_acceptance};

pub fn validate_evidence_identifier(value: &str) -> Result<(), TaskEvidenceError> {
    // E1 permits internal whitespace in criterion ids. Hashing, rather than restricting their
    // spelling, makes those ids safe for host-owned filesystem paths.
    if value.trim().is_empty() || value.trim() != value || value.len() > 4096 {
        return Err(TaskEvidenceError::Invalid);
    }
    Ok(())
}

pub fn validate_evidence_request_id(value: &str) -> Result<(), TaskEvidenceError> {
    if value.is_empty()
        || value.len() > 128
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(TaskEvidenceError::Invalid);
    }
    Ok(())
}

pub fn validate_evidence_token(value: &str) -> Result<(), TaskEvidenceError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(TaskEvidenceError::Invalid);
    }
    Ok(())
}

fn validate_git_oid(value: &str) -> Result<(), TaskEvidenceError> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(TaskEvidenceError::Invalid);
    }
    Ok(())
}

fn validate_time(value: &str) -> Result<(), TaskEvidenceError> {
    DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| TaskEvidenceError::Invalid)
}

pub fn criteria_sha256(criteria: &[TaskAcceptanceCriterion]) -> Result<String, TaskEvidenceError> {
    let criteria = normalize_task_acceptance(criteria).map_err(|_| TaskEvidenceError::Invalid)?;
    let bytes = serde_json::to_vec(&criteria).map_err(|_| TaskEvidenceError::Invalid)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn normalize_evidence_note(note: Option<&str>) -> Result<Option<String>, TaskEvidenceError> {
    match note.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.len() <= MAX_TASK_EVIDENCE_NOTE_BYTES => Ok(Some(value.to_string())),
        Some(_) => Err(TaskEvidenceError::Limit),
        None => Ok(None),
    }
}

pub fn normalize_waiver_reason(reason: &str) -> Result<(String, String), TaskEvidenceError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(TaskEvidenceError::Invalid);
    }
    if reason.len() > MAX_TASK_EVIDENCE_NOTE_BYTES {
        return Err(TaskEvidenceError::Limit);
    }
    Ok((
        reason.to_string(),
        format!("{:x}", Sha256::digest(reason.as_bytes())),
    ))
}

impl TaskEvidenceScope {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_evidence_identifier(&self.task_id)?;
        validate_evidence_token(&self.scope_id)?;
        if self.acceptance_revision > MAX_TASK_EVIDENCE_REVISION {
            return Err(TaskEvidenceError::Limit);
        }
        validate_evidence_token(&self.criteria_sha256)
    }
}

impl TaskEvidenceTree {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_git_oid(&self.head_commit_sha)?;
        validate_git_oid(&self.tree_sha)
    }
}

impl TaskEvidenceAttempt {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_evidence_token(&self.attempt_id)?;
        validate_evidence_request_id(&self.request_id)?;
        self.scope.validate()?;
        validate_evidence_identifier(&self.criterion_id)?;
        validate_evidence_identifier(&self.slot_id)?;
        validate_evidence_token(&self.lease_generation)?;
        validate_evidence_identifier(&self.session_key)?;
        self.prepared_tree.validate()?;
        validate_time(&self.prepared_at)?;
        if self.scope.acceptance_revision == 0
            || !(1..=MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION).contains(&self.attempt)
        {
            return Err(TaskEvidenceError::Invalid);
        }
        Ok(())
    }
}

impl TaskEvidenceArtifact {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_evidence_token(&self.sha256)?;
        if self.size_bytes == 0 {
            return Err(TaskEvidenceError::ArtifactMissing);
        }
        if self.size_bytes > MAX_TASK_EVIDENCE_ARTIFACT_BYTES {
            return Err(TaskEvidenceError::Limit);
        }
        Ok(())
    }
}

impl TaskEvidenceReceipt {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_evidence_request_id(&self.event_id)?;
        if self.evidence_revision == 0 || self.evidence_revision > MAX_TASK_EVIDENCE_REVISION {
            return Err(TaskEvidenceError::Invalid);
        }
        let evidence = &self.evidence;
        evidence.attempt.validate()?;
        evidence.artifact.validate()?;
        evidence.recorded_tree.validate()?;
        if evidence.recorded_tree != evidence.attempt.prepared_tree
            || normalize_evidence_note(evidence.note.as_deref())? != evidence.note
        {
            return Err(TaskEvidenceError::Invalid);
        }
        validate_evidence_request_id(&evidence.by)?;
        validate_time(&evidence.recorded_at)
    }
}

impl TaskCriterionWaiverReceipt {
    pub fn validate(&self) -> Result<(), TaskEvidenceError> {
        validate_evidence_request_id(&self.event_id)?;
        if self.evidence_revision == 0 || self.evidence_revision > MAX_TASK_EVIDENCE_REVISION {
            return Err(TaskEvidenceError::Invalid);
        }
        let waiver = &self.waiver;
        validate_evidence_request_id(&waiver.request_id)?;
        waiver.scope.validate()?;
        validate_evidence_identifier(&waiver.criterion_id)?;
        let (reason, digest) = normalize_waiver_reason(&waiver.reason)?;
        if waiver.scope.acceptance_revision == 0
            || reason != waiver.reason
            || digest != waiver.reason_digest
        {
            return Err(TaskEvidenceError::Invalid);
        }
        validate_evidence_request_id(&waiver.confirmation_ref)?;
        validate_time(&waiver.recorded_at)
    }
}
