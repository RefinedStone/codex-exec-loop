use crate::application::port::inbound::task_evidence_query_port::{
    PreviewTarget, TaskEvidenceQueryPort,
};
use crate::domain::planning::evidence::TaskEvidenceError;
use std::io::{Read, Write};

pub(super) fn show(
    decision: Option<crate::domain::planning::evidence::HumanEvidenceDecision>,
) -> anyhow::Result<()> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take(16_385)
        .read_to_end(&mut input)?;
    if input.len() > 16_384 {
        return Err(TaskEvidenceError::Limit.into());
    }
    let target: PreviewTarget =
        serde_json::from_slice(&input).map_err(|_| TaskEvidenceError::Invalid)?;
    let workspace = std::env::current_dir()?;
    let service = crate::composition::task_evidence::build(
        workspace.to_str().ok_or(TaskEvidenceError::Invalid)?,
    );
    if let Some(decision) = decision {
        return service.review_evidence(
            &target,
            decision,
            akra_verification_native::confirm_evidence_review,
        );
    }
    let text = service
        .sanitized_preview(&target)
        .map_err(|_| TaskEvidenceError::Unavailable)?;
    std::io::stdout().lock().write_all(text.as_bytes())?;
    Ok(())
}
