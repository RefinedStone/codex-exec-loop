//! A retained admission covers the actual RPC response, including an uncertain response.
//! The adapter never substitutes an unguarded call when admission fails.
use anyhow::Result;

pub trait AgentActionGuard: Send {
    /// Memory-only: no filesystem or authority reads after the final effect preflight.
    fn validate(&self) -> Result<()>;
}

pub trait AgentActionAuthorization: Send + Sync {
    fn acquire(&self) -> Result<Box<dyn AgentActionGuard>>;
}

impl AgentActionGuard for () {
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

impl<F> AgentActionAuthorization for F
where
    F: Fn() -> Result<Box<dyn AgentActionGuard>> + Send + Sync,
{
    fn acquire(&self) -> Result<Box<dyn AgentActionGuard>> {
        self()
    }
}
