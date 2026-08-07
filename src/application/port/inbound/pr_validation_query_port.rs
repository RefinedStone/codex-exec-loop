use anyhow::Result;

use crate::domain::parallel_mode::PrValidationOperatorSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrValidationStatusRequest {
    pub pull_request_number: u64,
}

pub trait PrValidationQueryPort: Send + Sync {
    fn status_for_pr(
        &self,
        request: PrValidationStatusRequest,
    ) -> Result<Option<PrValidationOperatorSummary>>;
}
