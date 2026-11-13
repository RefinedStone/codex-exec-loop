use anyhow::Result;
use serde_json::Value;

use crate::domain::kernel_spec::KernelCommandKind;

/// Repository-scoped commands and queries. Transport adapters validate and encode their inputs.
pub trait KernelCommandPort: Send + Sync {
    fn command(
        &self,
        kind: KernelCommandKind,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<Value>;

    /// Waiting is supported for history and operation queries.
    fn query(&self, input: Value, wait: bool) -> Result<Value>;
}
