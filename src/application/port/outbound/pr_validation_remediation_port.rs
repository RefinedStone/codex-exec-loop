use anyhow::Result;

use crate::domain::parallel_mode::{
    PrValidationCommitSha, PrValidationFindingKey, PrValidationRecordKey, PrValidationTarget,
};

/// Deterministic identity supplied to remediation delivery. Implementations must treat repeated
/// requests with the same key as the same logical request, including across process restarts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrValidationRemediationKey(String);

impl PrValidationRemediationKey {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err("PR validation remediation key must not be empty".to_string());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationRemediationRequest {
    pub idempotency_key: PrValidationRemediationKey,
    pub validation_record_key: PrValidationRecordKey,
    pub target: PrValidationTarget,
    pub target_sha: PrValidationCommitSha,
    pub finding_key: PrValidationFindingKey,
    pub summary: String,
}

/// Write boundary for handing actionable validation findings to the task/remediation runtime.
/// Idempotency belongs to this boundary rather than an in-memory service cache.
pub trait PrValidationRemediationPort: Send + Sync {
    /// Returns the durable ordinary planning-task identity admitted for this request. Replays of
    /// the same idempotency key must return the same identity.
    fn request_remediation(
        &self,
        request: &PrValidationRemediationRequest,
    ) -> Result<PrValidationRecordKey>;
}
