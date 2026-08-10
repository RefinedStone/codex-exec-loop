use anyhow::Result;

pub const PR_VALIDATION_ROLLOUT_EVIDENCE_MAX_DOCUMENTS: usize = 32;
pub const PR_VALIDATION_ROLLOUT_EVIDENCE_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Repository-local rollout evidence as an opaque, bounded document.
///
/// The adapter owns path traversal and file I/O. The application owns JSON parsing,
/// schema validation, freshness, decision semantics, redaction, and pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationRolloutEvidenceDocument {
    pub observed_at: String,
    pub content: Vec<u8>,
}

pub trait PrValidationRolloutEvidencePort: Send + Sync {
    fn load_documents(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<PrValidationRolloutEvidenceDocument>>;
}
