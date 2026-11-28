use super::*;
use crate::application::port::inbound::task_evidence_query_port::{PreviewKind, PreviewTarget};
use crate::domain::issue_masking::{IssueTextField, plan_issue_masking};
use crate::domain::issue_policy::{IssueDisclosureTarget, IssuePolicy};
use crate::domain::issue_secret_spans::secret_spans;
use akra_verification::ports::task_evidence_artifact_port::SealedEvidence;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

impl TaskEvidenceService {
    pub(super) fn preview_text(&self, target: &PreviewTarget) -> Result<String> {
        let task = self.current_criterion(
            &target.task_id,
            &target.criterion_id,
            &target.scope_id,
            target.revision,
        )?;
        let receipt = match target.kind {
            PreviewKind::Worker => {
                let receipt = task
                    .evidence
                    .iter()
                    .find(|r| {
                        r.event_id == target.event_id
                            && r.evidence.attempt.scope == task.scope
                            && r.evidence.attempt.criterion_id == target.criterion_id
                            && r.evidence.artifact.sha256 == target.sha256
                    })
                    .ok_or(TaskEvidenceError::Stale)?;
                receipt.validate()?;
                SealedEvidence::Worker(&receipt.evidence.attempt, &receipt.evidence.artifact)
            }
            PreviewKind::Host => {
                let receipt = task
                    .host_verifications
                    .iter()
                    .filter_map(|r| r.receipt.as_ref())
                    .find(|r| {
                        r.event_id == target.event_id
                            && r.attempt.scope == task.scope
                            && r.attempt.criterion_id == target.criterion_id
                    })
                    .ok_or(TaskEvidenceError::Stale)?;
                receipt.validate()?;
                let artifact = receipt
                    .artifact
                    .as_ref()
                    .filter(|a| a.sha256 == target.sha256)
                    .ok_or(TaskEvidenceError::Stale)?;
                SealedEvidence::Host(&receipt.attempt.attempt_id, artifact)
            }
        };
        let bytes = self
            .artifacts
            .preview_bytes(&self.workspace, receipt)
            .map_err(|_| TaskEvidenceError::Unavailable)?;
        match target.kind {
            PreviewKind::Worker => sanitized_text(&bytes),
            PreviewKind::Host => {
                #[derive(Deserialize)]
                struct Capture {
                    schema_version: u8,
                    stdout_base64: String,
                    stderr_base64: String,
                }
                let c: Capture =
                    serde_json::from_slice(&bytes).map_err(|_| TaskEvidenceError::Invalid)?;
                if c.schema_version != 1 {
                    return Err(TaskEvidenceError::Invalid.into());
                }
                let stream = |encoded: &str| {
                    sanitized_text(
                        &STANDARD
                            .decode(encoded)
                            .map_err(|_| TaskEvidenceError::Invalid)?,
                    )
                };
                Ok(format!(
                    "stdout:\n{}\n\nstderr:\n{}",
                    stream(&c.stdout_base64)?,
                    stream(&c.stderr_base64)?
                ))
            }
        }
    }
}

pub(super) fn sanitized_text(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| TaskEvidenceError::Invalid)?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
    {
        return Err(TaskEvidenceError::Invalid.into());
    }
    let spans = secret_spans(IssueTextField::Body, text);
    let policy = IssuePolicy::builtin();
    Ok(plan_issue_masking(
        "",
        text,
        &spans,
        policy.disclosure_for(IssueDisclosureTarget::AiHandoff),
    )
    .map_err(|_| TaskEvidenceError::Invalid)?
    .shared_copy
    .body)
}
