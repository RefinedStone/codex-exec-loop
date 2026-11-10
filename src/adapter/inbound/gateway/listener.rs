//! Gateway 가 클라이언트 접속을 받는 로컬 IPC 이다 (P-04). TCP 포트는 열지 않는다.
//! - unix: `$AKRA_HOME/run/gateway.sock`. 디렉터리는 0700, 소켓은 0600 이다.
//! - Windows: named pipe `\\.\pipe\akra-gateway-<사용자 SID>`. 현재 사용자 전용 보안 설명자로 만들고,
//!   첫 인스턴스만 이름을 만들 수 있게 해서 선점을 막고, 원격 클라이언트를 거부한다.

use std::path::Path;

use anyhow::Result;

#[cfg(unix)]
pub(super) type ClientConnection = tokio::net::UnixStream;
#[cfg(windows)]
pub(super) type ClientConnection = tokio::net::windows::named_pipe::NamedPipeServer;

#[cfg(unix)]
pub(super) struct GatewayListener {
    listener: tokio::net::UnixListener,
    path: std::path::PathBuf,
}

#[cfg(unix)]
impl GatewayListener {
    /// `run_dir` 는 현재 사용자 전용 디렉터리이고, 호출자는 단일 인스턴스 잠금을 이미 얻었다.
    pub(super) fn bind(run_dir: &Path) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;

        use anyhow::Context;

        let path = crate::gateway_endpoint::unix_socket_path(run_dir);
        super::ensure_endpoint_path_fits(&path)?;
        // 잠금을 얻었으므로 남은 소켓 파일은 이전 실행의 흔적이다.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to remove stale `{}`", path.display()));
            }
        }
        let listener = tokio::net::UnixListener::bind(&path)
            .with_context(|| format!("failed to listen on `{}`", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure `{}`", path.display()))?;
        Ok(Self { listener, path })
    }

    pub(super) async fn accept(&mut self) -> std::io::Result<ClientConnection> {
        let (stream, _address) = self.listener.accept().await?;
        Ok(stream)
    }

    pub(super) fn endpoint(&self) -> String {
        self.path.display().to_string()
    }
}

#[cfg(unix)]
impl Drop for GatewayListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
pub(super) struct GatewayListener {
    name: String,
    security: crate::private_fs::WindowsOwnerOnlySecurityDescriptor,
    next: ClientConnection,
}

#[cfg(windows)]
impl GatewayListener {
    pub(super) fn bind(_run_dir: &Path) -> Result<Self> {
        let name = crate::gateway_endpoint::windows_pipe_name()?;
        let security = crate::private_fs::WindowsOwnerOnlySecurityDescriptor::for_current_user()?;
        let next = create_pipe_instance(&name, &security, true)?;
        Ok(Self {
            name,
            security,
            next,
        })
    }

    pub(super) async fn accept(&mut self) -> std::io::Result<ClientConnection> {
        self.next.connect().await?;
        // 다음 클라이언트를 받을 인스턴스를 먼저 만든 뒤 연결된 인스턴스를 넘긴다.
        let next = create_pipe_instance(&self.name, &self.security, false)
            .map_err(std::io::Error::other)?;
        Ok(std::mem::replace(&mut self.next, next))
    }

    pub(super) fn endpoint(&self) -> String {
        self.name.clone()
    }
}

#[cfg(windows)]
fn create_pipe_instance(
    name: &str,
    security: &crate::private_fs::WindowsOwnerOnlySecurityDescriptor,
    first: bool,
) -> Result<ClientConnection> {
    use anyhow::Context;
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.as_raw(),
        bInheritHandle: 0,
    };
    // SAFETY: `attributes` and the descriptor it points to outlive this call.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    }
    .with_context(|| {
        if first {
            format!("failed to create `{name}`; another akra-gateway may own it")
        } else {
            format!("failed to create the next `{name}` instance")
        }
    })
}
