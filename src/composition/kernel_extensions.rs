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
mod tech_blog;
#[cfg(test)]
mod tests;

/// The host builds one repository package set per attached workspace.
struct RepositoryPackages<'a> {
    workspace: &'a Path,
    runtime: &'a Path,
}
fn registrations(
    root: &Path,
    scope: ExtensionScope,
    probe: Option<crate::adapter::outbound::storage_analysis::StorageRuntimeProbe>,
    repository: Option<&RepositoryPackages<'_>>,
) -> Vec<ExtensionRegistration> {
    let mut entries = vec![example::registration(scope)];
    if scope == ExtensionScope::LocalUser {
        entries.push(crate::composition::storage_extension::registration(
            root.join("storage-analysis"),
            probe,
        ));
    }
    if let Some(repository) = repository {
        entries.push(tech_blog::registration(
            repository.workspace,
            repository.runtime.to_owned(),
        ));
    }
    entries
}
fn build(
    root: &Path,
    scope: ExtensionScope,
    host_id: Option<String>,
    probe: Option<crate::adapter::outbound::storage_analysis::StorageRuntimeProbe>,
    repository: Option<&RepositoryPackages<'_>>,
) -> Result<Arc<ExtensionService>> {
    let registry = ExtensionRegistry::build(registrations(root, scope, probe, repository), scope)?;
    let permissions = BTreeSet::from([
        "example.run".into(),
        "metadata.read".into(),
        "storage.read".into(),
        "git.read".into(),
        "article.write".into(),
        "model.run".into(),
        "workspace.write".into(),
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
    build(&root, ExtensionScope::LocalUser, None, probe, None)
}
pub(crate) fn build_repository(workspace: &str, kernel: &str) -> Result<Arc<ExtensionService>> {
    let runtime = crate::composition::production::delegation_runtime(workspace)?;
    let root = runtime.join("kernel-extensions");
    let packages = RepositoryPackages {
        workspace: Path::new(workspace),
        runtime: &runtime,
    };
    let host = build(
        &root,
        ExtensionScope::Repository,
        Some(kernel.to_owned()),
        None,
        Some(&packages),
    )?;
    adopt_legacy_blog_activation(&host, &runtime)?;
    Ok(host)
}
/// A studio enabled under version 2 stays enabled once the host owns its activation.
fn adopt_legacy_blog_activation(host: &Arc<ExtensionService>, runtime: &Path) -> Result<()> {
    use crate::domain::repository_extension::tech_blog::{ID, VERSION};
    let untouched = host
        .catalog()
        .into_iter()
        .any(|entry| entry.id == ID && entry.revision == 0 && !entry.desired_enabled);
    if untouched && tech_blog::legacy_enabled(runtime) {
        let context = host.context("legacy-blog-activation".into(), Some(host.host_id().into()));
        host.call(
            context,
            ID,
            VERSION,
            "configure",
            serde_json::json!({"enabled": true, "expected_revision": 0}),
            None,
        )?;
    }
    Ok(())
}

pub(crate) fn cli_request(
    scope: &str,
    workspace: &str,
    call: Option<serde_json::Value>,
    thread_id: Option<&str>,
) -> Result<serde_json::Value> {
    crate::adapter::outbound::extension_gateway_client::request(scope, workspace, call, thread_id)
}
