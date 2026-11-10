//! akra-gateway 의 로컬 IPC 이름이다. Gateway listener 와 Rust 클라이언트(`akra mcp`)가 같이 쓴다.
//! - unix: `$AKRA_HOME/run/gateway.sock`
//! - Windows: `\\.\pipe\akra-gateway-<사용자 SID>`. 이름이 `AKRA_HOME` 이 아니라 사용자에 묶인다.

#[cfg(unix)]
use std::path::{Path, PathBuf};

use anyhow::Result;

pub(crate) const RUN_DIRECTORY_NAME: &str = "run";
#[cfg(unix)]
const SOCKET_FILE_NAME: &str = "gateway.sock";

#[cfg(unix)]
pub(crate) fn unix_socket_path(run_dir: &Path) -> PathBuf {
    run_dir.join(SOCKET_FILE_NAME)
}

#[cfg(windows)]
pub(crate) fn windows_pipe_name() -> Result<String> {
    Ok(format!(
        r"\\.\pipe\akra-gateway-{}",
        crate::private_fs::windows_current_user_sid_string()?
    ))
}

/// 클라이언트가 접속할 endpoint 이다. unix 는 소켓 경로, Windows 는 pipe 이름이다.
pub(crate) fn client_endpoint() -> Result<String> {
    #[cfg(unix)]
    {
        let run_dir = crate::configuration::akra_home_dir()?.join(RUN_DIRECTORY_NAME);
        Ok(unix_socket_path(&run_dir).display().to_string())
    }
    #[cfg(windows)]
    {
        windows_pipe_name()
    }
}
