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

/*
 * 단위 시험은 개발자의 실제 Gateway 에 붙지 않는다. 실제 조립으로 도는 시험(CLI, admin 의 초기화 등)이 Gateway
 * 클라이언트를 지나기 때문이다 (plan 18 K4-9a). 시험 빌드의 endpoint 는 아무도 듣지 않는 이름이다. 그래서 그 시험은
 * "Gateway 가 없다"의 길로 간다. planning 저장소가 시험에서 임시 홈을 쓰는 것과 같은 규칙이다.
 */
#[cfg(test)]
pub(crate) fn client_endpoint() -> Result<String> {
    #[cfg(unix)]
    {
        Ok(crate::test_utils::platform_safe_temp_dir()
            .join(".akra")
            .join("tests")
            .join(RUN_DIRECTORY_NAME)
            .join("no-gateway-in-unit-tests.sock")
            .display()
            .to_string())
    }
    #[cfg(windows)]
    {
        Ok(r"\\.\pipe\akra-gateway-no-gateway-in-unit-tests".to_string())
    }
}

/// 클라이언트가 접속할 endpoint 이다. unix 는 소켓 경로, Windows 는 pipe 이름이다.
#[cfg(not(test))]
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
