use super::*;
use crate::application::port::inbound::extension_host_port::ExtensionHostPort;

impl ExtensionHostPort for ExtensionService {
    fn host_id(&self) -> &str {
        ExtensionService::host_id(self)
    }
    fn generation(&self) -> &str {
        ExtensionService::generation(self)
    }
    fn scope(&self) -> ExtensionScope {
        ExtensionService::scope(self)
    }
    fn context(&self, connection_id: String, repository_key: Option<String>) -> ExtensionContext {
        ExtensionService::context(self, connection_id, repository_key)
    }
    fn catalog(&self) -> Vec<ExtensionCatalogEntry> {
        ExtensionService::catalog(self)
    }
    fn subscribe(&self) -> broadcast::Receiver<ExtensionEvent> {
        ExtensionService::subscribe(self)
    }
    fn call(
        self: Arc<Self>,
        context: ExtensionContext,
        extension: &str,
        version: u32,
        action: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<Value> {
        ExtensionService::call(
            &self,
            context,
            extension,
            version,
            action,
            input,
            idempotency_key,
        )
    }
}
