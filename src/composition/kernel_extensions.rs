//! Static package assembly. New packages register here; transport dispatch stays generic.
use crate::adapter::outbound::db::extension_state_store::SqliteExtensionStateStore;
use crate::application::service::kernel_extensions::ExtensionService;
use crate::application::service::kernel_extensions::registry::{
    ExtensionRegistration, ExtensionRegistry,
};
use crate::domain::kernel_extension::ExtensionScope;
use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
mod example;
#[cfg(test)]
mod tests;

fn registrations(
    root: &Path,
    scope: ExtensionScope,
    probe: Option<crate::adapter::outbound::storage_analysis::StorageRuntimeProbe>,
) -> Vec<ExtensionRegistration> {
    let mut entries = vec![example::registration(scope)];
    if scope == ExtensionScope::LocalUser {
        entries.push(crate::composition::storage_extension::registration(
            root.join("storage-analysis"),
            probe,
        ));
    }
    entries
}
fn build(
    root: &Path,
    scope: ExtensionScope,
    host_id: Option<String>,
    probe: Option<crate::adapter::outbound::storage_analysis::StorageRuntimeProbe>,
) -> Result<Arc<ExtensionService>> {
    let registry = ExtensionRegistry::build(registrations(root, scope, probe), scope)?;
    let permissions = BTreeSet::from([
        "example.run".into(),
        "metadata.read".into(),
        "storage.read".into(),
        "git.read".into(),
    ]);
    ExtensionService::new(
        registry,
        Arc::new(SqliteExtensionStateStore::open(root)?),
        scope,
        host_id,
        permissions,
    )
}
pub(crate) fn build_local(
    probe: Option<crate::adapter::outbound::storage_analysis::StorageRuntimeProbe>,
) -> Result<Arc<ExtensionService>> {
    #[cfg(not(test))]
    let root = crate::stable_user_data_root::stable_user_data_root()?
        .join("extensions")
        .join("local-user");
    #[cfg(test)]
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-extension-host-{:032x}",
        rand::random::<u128>()
    ));
    build(&root, ExtensionScope::LocalUser, None, probe)
}
pub(crate) fn build_repository(workspace: &str, kernel: &str) -> Result<Arc<ExtensionService>> {
    let root =
        crate::composition::production::delegation_runtime(workspace)?.join("kernel-extensions");
    build(
        &root,
        ExtensionScope::Repository,
        Some(kernel.to_owned()),
        None,
    )
}

pub(crate) fn cli_request(
    scope: &str,
    workspace: &str,
    call: Option<serde_json::Value>,
    thread_id: Option<&str>,
) -> Result<serde_json::Value> {
    crate::adapter::outbound::extension_gateway_client::request(scope, workspace, call, thread_id)
}
