//! akra-gateway: 데스크톱 앱 같은 로컬 클라이언트를 codex app-server 에 중계하는 상주 데몬이다
//! (docs/plan/17-akra-desktop P1). 클라이언트 접속마다 upstream 연결을 하나씩 열고 메시지를 바꾸지 않고
//! 전달한다. `akra/*` 메서드만 Gateway 가 직접 답한다.
//!
//! 순수 중계는 application use case 가 아니므로 inbound port 없이 composition 이 조립한 outbound port 를
//! 쓴다. Telegram 입구가 `TelegramBotPort` 를 쓰는 것과 같은 구조이다.

mod kernel;
mod listener;
mod recorder;
mod router;
mod session;
#[cfg(test)]
mod tests;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use self::listener::GatewayListener;
use self::recorder::GatewayRecorder;
use crate::application::port::outbound::gateway_upstream_port::GatewayUpstreamPort;
use crate::composition::production;
use crate::composition::repository_kernel::RepositoryKernelRegistry;

const USAGE: &str = "\
Usage: akra-gateway

Runs the Akra gateway daemon. Local clients connect over WebSocket on a
user-private endpoint and are relayed to a Gateway-owned codex app-server.

  unix:    $AKRA_HOME/run/gateway.sock
  Windows: \\\\.\\pipe\\akra-gateway-<user SID>

Options:
  -h, --help    Show this help
";
const RUN_DIRECTORY_NAME: &str = "run";
const INSTANCE_LOCK_FILE_NAME: &str = "gateway.lock";
/// macOS 의 `sun_path` 는 NUL 을 포함해 104 바이트이다.
#[cfg(unix)]
const MAX_UNIX_SOCKET_PATH_BYTES: usize = 103;
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

#[derive(Debug, PartialEq, Eq)]
enum GatewayCommand {
    Run,
    Help,
}

pub async fn run_from_env() -> Result<()> {
    run_with_args(std::env::args().skip(1).collect()).await
}

pub async fn run_with_args(args: Vec<String>) -> Result<()> {
    if parse_args(&args)? == GatewayCommand::Help {
        print!("{USAGE}");
        return Ok(());
    }
    let akra_home = crate::configuration::akra_home_dir()?;
    let run_dir = akra_home.join(RUN_DIRECTORY_NAME);
    crate::private_directory::ensure_private_directory(&run_dir)?;
    let _instance_lock = acquire_instance_lock(&run_dir)?;
    let recorder = GatewayRecorder::from_env(&akra_home)?.map(Arc::new);
    let listener = GatewayListener::bind(&run_dir)?;
    let production::GatewayBackend { upstream, kernels } =
        production::build_gateway_backend(&run_dir)?;
    println!("akra-gateway listening on {}", listener.endpoint());
    let _ = std::io::stdout().flush();
    tracing::info!(
        endpoint = %listener.endpoint(),
        recording = recorder.is_some(),
        "akra_gateway_started"
    );

    serve(listener, upstream.clone(), kernels.clone(), recorder).await;
    stop_backend(upstream, kernels).await;
    tracing::info!("akra_gateway_stopped");
    Ok(())
}

/// 커널을 먼저 멈추고 그 뒤에 upstream app-server 를 멈춘다. 커널의 app-server 연결과 병렬 작업 턴은 upstream 을
/// 쓴다. 접속 작업이 목록을 아직 쥐고 있어도 커널은 멈춘다. 프로세스는 이 뒤 `exit` 로 끝나 소멸자가 돌지 않는다.
async fn stop_backend(
    upstream: Arc<dyn GatewayUpstreamPort>,
    kernels: Arc<RepositoryKernelRegistry>,
) {
    if let Err(error) = tokio::task::spawn_blocking(move || kernels.shutdown()).await {
        tracing::warn!(error = %error, "repository kernel shutdown failed");
    }
    upstream.shutdown();
}

fn parse_args(args: &[String]) -> Result<GatewayCommand> {
    match args {
        [] => Ok(GatewayCommand::Run),
        [flag] if flag == "-h" || flag == "--help" => Ok(GatewayCommand::Help),
        [unsupported, ..] => bail!("unsupported argument: {unsupported}\n\n{USAGE}"),
    }
}

/// 같은 사용자의 두 번째 Gateway 는 시작하지 않는다. 잠금은 프로세스가 끝나면 OS 가 푼다.
fn acquire_instance_lock(run_dir: &Path) -> Result<File> {
    let path = run_dir.join(INSTANCE_LOCK_FILE_NAME);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open `{}`", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            bail!(
                "another akra-gateway is already running (lock `{}`)",
                path.display()
            )
        }
        Err(std::fs::TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("failed to lock `{}`", path.display()))
        }
    }
}

async fn serve(
    mut listener: GatewayListener,
    upstream: Arc<dyn GatewayUpstreamPort>,
    kernels: Arc<RepositoryKernelRegistry>,
    recorder: Option<Arc<GatewayRecorder>>,
) {
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => return,
            accepted = listener.accept() => match accepted {
                Ok(connection) => {
                    let upstream = upstream.clone();
                    let kernels = kernels.clone();
                    let recorder = recorder.clone();
                    tokio::spawn(async move {
                        if let Err(error) =
                            session::run_session(connection, upstream, kernels, recorder).await
                        {
                            tracing::debug!(error = %format!("{error:#}"), "gateway session failed");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(error = %error, "gateway accept failed");
                    tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                }
            },
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(windows)]
    {
        use tokio::signal::windows::{ctrl_close, ctrl_shutdown};
        match (ctrl_close(), ctrl_shutdown()) {
            (Ok(mut close), Ok(mut shutdown)) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = close.recv() => {}
                    _ = shutdown.recv() => {}
                }
            }
            _ => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
}

#[cfg(unix)]
fn ensure_endpoint_path_fits(path: &Path) -> Result<()> {
    let length = path.as_os_str().len();
    if length > MAX_UNIX_SOCKET_PATH_BYTES {
        bail!(
            "socket path `{}` is {length} bytes; the limit is {MAX_UNIX_SOCKET_PATH_BYTES}. Set AKRA_HOME to a shorter path",
            path.display()
        )
    }
    Ok(())
}
