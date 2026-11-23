//! Application entry contract shared by the attached Gateway and headless clients.
use crate::domain::kernel_extension::{
    ExtensionCatalogEntry, ExtensionContext, ExtensionEvent, ExtensionScope,
};
use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;

pub(crate) trait ExtensionHostPort: Send + Sync {
    fn host_id(&self) -> &str;
    fn generation(&self) -> &str;
    fn scope(&self) -> ExtensionScope;
    fn context(&self, connection_id: String, repository_key: Option<String>) -> ExtensionContext;
    fn catalog(&self) -> Vec<ExtensionCatalogEntry>;
    fn subscribe(&self) -> broadcast::Receiver<ExtensionEvent>;
    fn call(
        self: Arc<Self>,
        context: ExtensionContext,
        extension: &str,
        version: u32,
        action: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<Value>;
}
