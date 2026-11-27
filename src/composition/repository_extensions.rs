//! Repository-scoped extension host lifetime. Every package registers on the common host.
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::application::port::inbound::extension_host_port::ExtensionHostPort;
use crate::application::service::kernel_extensions::ExtensionService;

pub(crate) struct RepositoryExtensions {
    workspace: PathBuf,
    common: std::sync::Mutex<Option<Arc<ExtensionService>>>,
}

impl RepositoryExtensions {
    pub(crate) fn new(workspace: &str) -> Arc<Self> {
        Arc::new(Self {
            workspace: workspace.into(),
            common: std::sync::Mutex::new(None),
        })
    }

    pub(crate) fn common(&self, kernel: &str) -> Result<Arc<dyn ExtensionHostPort>> {
        let mut common = self.common.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(host) = &*common {
            return Ok(host.clone());
        }
        let host = crate::composition::kernel_extensions::build_repository(
            &self.workspace.to_string_lossy(),
            kernel,
        )?;
        *common = Some(host.clone());
        Ok(host)
    }

    pub(crate) fn shutdown(&self) {
        if let Some(host) = self
            .common
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            host.shutdown();
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.common
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|h| h.busy())
    }
}
