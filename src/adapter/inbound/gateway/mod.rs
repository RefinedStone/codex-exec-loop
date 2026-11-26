//! akra-gateway: 데스크톱 앱 같은 로컬 클라이언트를 codex app-server 에 중계하는 상주 데몬이다
//! (docs/plan/17-akra-desktop P1). 클라이언트 접속마다 upstream 연결을 하나씩 열고 메시지를 바꾸지 않고
//! 전달한다. `akra/*` 메서드만 Gateway 가 직접 답한다.
//!
//! 순수 중계는 application use case 가 아니므로 inbound port 없이 composition 이 조립한 outbound port 를
//! 쓴다. Telegram 입구가 `TelegramBotPort` 를 쓰는 것과 같은 구조이다.

mod chats;
mod confirmation;
mod context;
mod delegation;
pub(crate) use delegation::Delegations;
pub(crate) use delegation::{advance_pr_review, expire_pr_reviews};
mod conversation_workspace;
mod dots;
mod git;
mod hosts;
mod issue;
mod issue_draft;
mod kernel;
mod listener;
mod mcp;
mod provider_credentials;
mod recorder;
mod resource_version;
mod restart;
mod router;
mod session;
mod storage_context;
#[cfg(test)]
mod tests;
mod turn_activity;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use self::hosts::GatewayHosts;
use self::listener::GatewayListener;
pub(crate) use self::mcp::GatewayMcpHub;
use self::recorder::GatewayRecorder;
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
const INSTANCE_LOCK_FILE_NAME: &str = "gateway.lock";
/// macOS 의 `sun_path` 는 NUL 을 포함해 104 바이트이다.
#[cfg(unix)]
const MAX_UNIX_SOCKET_PATH_BYTES: usize = 103;
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);
/// 한가한 커널을 찾는 간격이다. 커널은 한가한 시간(10분)이 지난 뒤 이 간격 안에 내려간다.
const KERNEL_IDLE_SWEEP_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
enum GatewayCommand {
    Run,
    Help,
    DesktopBrowserBridge,
    DesktopConfirmationBridge,
    DesktopRestoreBridge,
    ProviderCredentialPrompt,
    ProviderCredentialMigration,
}

pub async fn run_from_env() -> Result<()> {
    run_with_args(std::env::args().skip(1).collect()).await
}

pub async fn run_with_args(args: Vec<String>) -> Result<()> {
    match parse_args(&args)? {
        GatewayCommand::Help => {
            print!("{USAGE}");
            return Ok(());
        }
        GatewayCommand::DesktopBrowserBridge => {
            return super::desktop_browser_bridge::run().await;
        }
        GatewayCommand::DesktopConfirmationBridge => {
            return super::desktop_confirmation_bridge::run().await;
        }
        GatewayCommand::DesktopRestoreBridge => {
            return super::desktop_restore_bridge::run().await;
        }
        GatewayCommand::ProviderCredentialPrompt => {
            return tokio::task::spawn_blocking(|| {
                super::provider_credential_prompt::run(
                    crate::composition::provider_credentials::native_input(),
                )
            })
            .await
            .map_err(|_| anyhow::anyhow!("provider credential input unavailable"))?;
        }
        GatewayCommand::ProviderCredentialMigration => {
            let profile = crate::composition::provider_credentials::profile()?;
            return super::provider_credential_migration::run(profile.id, profile.runtime).await;
        }
        GatewayCommand::Run => {}
    }
    let akra_home = crate::configuration::akra_home_dir()?;
    let run_dir = akra_home.join(crate::gateway_endpoint::RUN_DIRECTORY_NAME);
    crate::private_directory::ensure_private_directory(&run_dir)?;
    let _instance_lock = acquire_instance_lock(&run_dir)?;
    let providers = crate::composition::provider_credentials::build()?;
    let recovering = providers.credentials.clone();
    tokio::task::spawn_blocking(move || recovering.recover())
        .await
        .map_err(|_| anyhow::anyhow!("provider credential recovery unavailable"))??;
    providers.owner.prepare_endpoint().await?;
    let validating = providers.work.clone();
    let migration = super::provider_credential_migration::start(
        providers.profile.id.clone(),
        providers.profile.runtime.clone(),
        providers.credentials.clone(),
        Arc::new(move |reference, url| {
            use crate::application::port::outbound::provider_work_port::ProviderLinkPort;
            validating.validate_link(&crate::domain::provider_work::ProviderLinkRequest {
                connection: reference.clone(),
                url: url.to_owned(),
            })
        }),
    )?;
    let recorder = GatewayRecorder::from_env(&akra_home)?.map(Arc::new);
    let listener = GatewayListener::bind(&run_dir)?;
    let production::GatewayBackend {
        upstream,
        other_hosts,
        kernels,
        mcp_hub,
        work_context,
        providers,
        git_graph,
        git_operations,
        turn_activity,
        conversation_workspaces,
        issue_handoff,
        dots,
        chats,
        issue_drafts,
        delegations,
    } = production::build_gateway_backend(&run_dir, providers)?;
    println!("akra-gateway listening on {}", listener.endpoint());
    let _ = std::io::stdout().flush();
    tracing::info!(
        endpoint = %listener.endpoint(),
        recording = recorder.is_some(),
        "akra_gateway_started"
    );

    // `local` 과, 켜졌으면 dots `cloud` 호스트이다 (plan 25 G0, D-19).
    if !other_hosts.is_empty() {
        tracing::info!(
            hosts = ?other_hosts.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            "akra_gateway_extra_hosts"
        );
    }
    let hosts = GatewayHosts::new(upstream, other_hosts);
    let stopping_credentials = providers.credentials.clone();
    serve(
        listener,
        hosts.clone(),
        kernels.clone(),
        session::GatewayServices {
            mcp_hub: Some(mcp_hub.clone()),
            work_context: Some(work_context),
            resource_versions: None,
            providers: Some(providers),
            delegations,
            git_graph: Some(git_graph),
            git_operations: Some(git_operations),
            turn_activity: Some(turn_activity),
            conversation_workspaces: Some(conversation_workspaces),
            issue_handoff: Some(issue_handoff),
            dots,
            chats,
            issue_drafts: Some(issue_drafts),
        },
        recorder,
    )
    .await;
    let _ = tokio::task::spawn_blocking(move || stopping_credentials.shutdown()).await;
    migration.shutdown().await;
    stop_backend(hosts, kernels, mcp_hub).await;
    tracing::info!("akra_gateway_stopped");
    Ok(())
}

/// 커널을 먼저 멈추고 그 뒤에 upstream app-server 를 멈춘다. 커널의 app-server 연결과 병렬 작업 턴은 upstream 을
/// 쓴다. 접속 작업이 목록을 아직 쥐고 있어도 커널은 멈춘다. main은 ExitCode를 반환하여 진단 로그를 flush한다.
/// Tokio runtime의 drop은 남은 blocking 작업의 종료를 기다리므로 각 backend의 제한된 종료 계약을 유지한다.
async fn stop_backend(
    hosts: GatewayHosts,
    kernels: Arc<RepositoryKernelRegistry>,
    mcp_hub: Arc<GatewayMcpHub>,
) {
    if let Err(error) = tokio::task::spawn_blocking(move || kernels.shutdown()).await {
        tracing::warn!(error = %error, "repository kernel shutdown failed");
    }
    // MCP hub 엔진은 Gateway upstream 과 별개인 app-server 이다.
    let _ = tokio::task::spawn_blocking(move || mcp_hub.shutdown()).await;
    hosts.shutdown();
}

fn parse_args(args: &[String]) -> Result<GatewayCommand> {
    match args {
        [] => Ok(GatewayCommand::Run),
        [command] if command == "desktop-browser-bridge" => {
            Ok(GatewayCommand::DesktopBrowserBridge)
        }
        [command] if command == "desktop-confirmation-bridge" => {
            Ok(GatewayCommand::DesktopConfirmationBridge)
        }
        [command] if command == "desktop-restore-bridge" => {
            Ok(GatewayCommand::DesktopRestoreBridge)
        }
        [command] if command == "provider-credential-prompt" => {
            Ok(GatewayCommand::ProviderCredentialPrompt)
        }
        [command] if command == "provider-credential-migration" => {
            Ok(GatewayCommand::ProviderCredentialMigration)
        }
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

struct IdleNotification(tokio::sync::Notify);

impl std::task::Wake for IdleNotification {
    fn wake(self: Arc<Self>) {
        self.0.notify_one();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.notify_one();
    }
}

fn register_idle_notification(kernels: &RepositoryKernelRegistry) -> Arc<IdleNotification> {
    let notification = Arc::new(IdleNotification(tokio::sync::Notify::new()));
    kernels.notify_when_idle(std::task::Waker::from(notification.clone()));
    notification
}

async fn serve(
    mut listener: GatewayListener,
    hosts: GatewayHosts,
    kernels: Arc<RepositoryKernelRegistry>,
    services: session::GatewayServices,
    recorder: Option<Arc<GatewayRecorder>>,
) {
    let idle_notification = register_idle_notification(&kernels);
    // Discovery is blocking I/O; keep accepting clients and shutdown signals during recovery.
    let _restart = restart::start(
        kernels.clone(),
        crate::composition::issue_collection::build_restart_registry(),
    );
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let mut idle_sweep = tokio::time::interval(KERNEL_IDLE_SWEEP_INTERVAL);
    idle_sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = &mut shutdown => return,
            () = idle_notification.0.notified() => {
                unload_idle_kernels(kernels.clone());
            }
            _ = idle_sweep.tick() => {
                unload_idle_kernels(kernels.clone());
                if let Some(mcp_hub) = services.mcp_hub.clone() {
                    tokio::task::spawn_blocking(move || mcp_hub.release_if_idle());
                }
            }
            accepted = listener.accept() => match accepted {
                Ok(connection) => {
                    let hosts = hosts.clone();
                    let kernels = kernels.clone();
                    let services = services.clone();
                    let recorder = recorder.clone();
                    tokio::spawn(async move {
                        if let Err(error) =
                            session::run_session(connection, hosts, kernels, services, recorder)
                                .await
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

/// 오래 한가한 커널을 내린다. 내리기는 호스트 스레드를 기다리므로 블로킹 스레드에서 한다.
fn unload_idle_kernels(kernels: Arc<RepositoryKernelRegistry>) {
    tokio::task::spawn_blocking(move || {
        for repository_key in kernels.unload_idle_kernels(Instant::now()) {
            tracing::info!(repository = %repository_key, "idle repository kernel unloaded");
        }
    });
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
