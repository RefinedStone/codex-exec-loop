use crate::application::port::inbound::task_evidence_query_port::{
    PreviewTarget, TaskEvidenceQueryPort,
};
use crate::domain::planning::evidence::TaskEvidenceError;
use std::io::{Read, Write};

pub(super) fn show() -> anyhow::Result<()> {
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
    let text = crate::composition::task_evidence::build(
        workspace.to_str().ok_or(TaskEvidenceError::Invalid)?,
    )
    .sanitized_preview(&target)
    .map_err(|_| TaskEvidenceError::Unavailable)?;
    std::io::stdout().lock().write_all(text.as_bytes())?;
    Ok(())
}
