//! Owner-only local IPC shared by the Gateway and the Desktop browser broker.

use anyhow::{Context, Result};

pub(crate) mod credential_frames;
pub(crate) mod credential_prompt;
mod owner;
#[cfg(test)]
pub(crate) use owner::PrivateIpcClientConnection;
pub(crate) use owner::{connect_owner_checked, require_inherited_pipes, verify_client_owner};

#[cfg(unix)]
pub(crate) type ClientConnection = tokio::net::UnixStream;
#[cfg(windows)]
pub(crate) type ClientConnection = tokio::net::windows::named_pipe::NamedPipeServer;

#[cfg(unix)]
pub(crate) fn provider_credential_socket_path(runtime: &std::path::Path) -> std::path::PathBuf {
    runtime.join("ipc")
}

#[cfg(unix)]
pub(crate) struct PrivateIpcListener {
    listener: tokio::net::UnixListener,
    path: std::path::PathBuf,
}

#[cfg(unix)]
impl PrivateIpcListener {
    /// The caller owns the private parent directory. Never unlink an existing endpoint.
    pub(crate) fn bind(endpoint: &str) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;

        let path = std::path::PathBuf::from(endpoint);
        // macOS sun_path has 104 bytes including the terminating NUL.
        anyhow::ensure!(
            path.as_os_str().len() <= 103,
            "private IPC path is too long; shorten AKRA_HOME"
        );
        let listener =
            tokio::net::UnixListener::bind(&path).context("failed to bind private IPC endpoint")?;
        let listener = Self { listener, path };
        std::fs::set_permissions(&listener.path, std::fs::Permissions::from_mode(0o600))
            .context("failed to secure private IPC endpoint")?;
        Ok(listener)
    }

    pub(crate) async fn accept(&mut self) -> std::io::Result<ClientConnection> {
        self.listener.accept().await.map(|(stream, _)| stream)
    }

    pub(crate) fn endpoint(&self) -> String {
        self.path.display().to_string()
    }
}

#[cfg(unix)]
impl Drop for PrivateIpcListener {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "private IPC endpoint cleanup failed");
        }
    }
}

#[cfg(windows)]
pub(crate) struct PrivateIpcListener {
    name: String,
    security: crate::private_fs::WindowsOwnerOnlySecurityDescriptor,
    next: ClientConnection,
}

#[cfg(windows)]
impl PrivateIpcListener {
    pub(crate) fn bind(endpoint: &str) -> Result<Self> {
        let security = crate::private_fs::WindowsOwnerOnlySecurityDescriptor::for_current_user()?;
        let next = create_pipe_instance(endpoint, &security, true)?;
        Ok(Self {
            name: endpoint.to_owned(),
            security,
            next,
        })
    }

    pub(crate) async fn accept(&mut self) -> std::io::Result<ClientConnection> {
        self.next.connect().await?;
        let next = create_pipe_instance(&self.name, &self.security, false)
            .map_err(std::io::Error::other)?;
        Ok(std::mem::replace(&mut self.next, next))
    }

    pub(crate) fn endpoint(&self) -> String {
        self.name.clone()
    }
}

#[cfg(windows)]
fn create_pipe_instance(
    name: &str,
    security: &crate::private_fs::WindowsOwnerOnlySecurityDescriptor,
    first: bool,
) -> Result<ClientConnection> {
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.as_raw(),
        bInheritHandle: 0,
    };
    // SAFETY: the attributes and immutable descriptor outlive this call.
    // This is the existing Gateway listener's owner-only pipe creation boundary.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    }
    .context("failed to create private IPC pipe instance")
}
