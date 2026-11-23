use crate::domain::kernel_extension::ExtensionDurableState;
use anyhow::Result;
/// Private single-owner storage. Failed saves leave the previous state intact.
pub(crate) trait ExtensionStateStorePort: Send + Sync {
    fn load(&self) -> Result<ExtensionDurableState>;
    fn save(&self, state: &ExtensionDurableState) -> Result<()>;
}
