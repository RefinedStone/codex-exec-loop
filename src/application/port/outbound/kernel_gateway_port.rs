use anyhow::Result;
use serde_json::Value;

/// A missing Gateway is distinct from a failed request. Failed requests must not execute locally.
pub enum KernelGatewayReply {
    Unavailable,
    Answer(Value),
}

/// JSON-only boundary to the repository kernel owned by the Gateway.
pub trait KernelGatewayPort: Send + Sync {
    fn request(&self, method: &str, params: Value, wait: bool) -> Result<KernelGatewayReply>;
}
