//! Gateway endpoint policy over the shared owner-only IPC listener.

use std::path::Path;

use anyhow::Result;

pub(super) use crate::private_ipc::ClientConnection;

pub(super) struct GatewayListener(crate::private_ipc::PrivateIpcListener);

impl GatewayListener {
    /// The caller has secured run_dir and holds the Gateway instance lock.
    pub(super) fn bind(run_dir: &Path) -> Result<Self> {
        #[cfg(unix)]
        let endpoint = {
            use anyhow::Context;
            let path = crate::gateway_endpoint::unix_socket_path(run_dir);
            super::ensure_endpoint_path_fits(&path)?;
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("failed to remove stale Gateway socket"),
            }
            path.display().to_string()
        };
        #[cfg(windows)]
        let endpoint = {
            let _ = run_dir;
            crate::gateway_endpoint::windows_pipe_name()?
        };
        Ok(Self(crate::private_ipc::PrivateIpcListener::bind(
            &endpoint,
        )?))
    }

    pub(super) async fn accept(&mut self) -> std::io::Result<ClientConnection> {
        self.0.accept().await
    }

    pub(super) fn endpoint(&self) -> String {
        self.0.endpoint()
    }
}
