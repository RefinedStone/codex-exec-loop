use anyhow::Result;
use tokio::sync::mpsc;

use crate::domain::work_context::ExternalResourceRef;

pub const RESOURCE_VERSION_REQUEST_PREFIX: &str = "akra/resourceVersion/request/";

pub struct ResourceVersionRequest {
    pub id: String,
    pub resource: ExternalResourceRef,
}

/// A connected client can register and answer requests for its Jira installation.
pub trait ResourceVersionProviderPort: Send + Sync {
    fn open(&self) -> Result<(u64, mpsc::Receiver<ResourceVersionRequest>)>;
    fn register(&self, session: u64, site: Option<&str>) -> Result<()>;
    fn close(&self, session: u64);
    fn answer(&self, session: u64, id: &str, answer: Result<String>);
}
