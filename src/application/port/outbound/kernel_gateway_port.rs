use anyhow::Result;
use serde_json::Value;

/// What the Gateway did with one request. A failed request is an error, not a reply: it must not
/// run locally, because the Gateway kernel may have taken it.
pub enum KernelGatewayReply {
    /// No Gateway answers on the endpoint. Nothing was sent.
    NoGateway,
    /// The Gateway answered, but it cannot attach to the kernel of this repository. Another
    /// process (a TUI) runs that kernel. The request did not run.
    KernelHeldElsewhere,
    Answer(Value),
}

/// JSON-only boundary to the repository kernel owned by the Gateway.
pub trait KernelGatewayPort: Send + Sync {
    fn request(&self, method: &str, params: Value, wait: bool) -> Result<KernelGatewayReply>;
}
