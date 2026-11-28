//! Production TUI startup through the local Gateway, never a repository writer.

use std::io::ErrorKind;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::adapter::outbound::app_server::gateway_relay::connect_with_retry;
use crate::composition::gateway_kernel_client_runtime::GatewayKernelClientRuntime;
use crate::core::native_client_port::NativeClientPort;

pub(crate) fn build_tui_gateway_client() -> Result<Box<dyn NativeClientPort>> {
    let workspace = std::env::current_dir().context("resolve the TUI workspace")?;
    let endpoint = crate::gateway_endpoint::client_endpoint()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(ensure_gateway(&endpoint))?;
    Ok(Box::new(GatewayKernelClientRuntime::attach(
        workspace.display().to_string(),
    )?))
}

async fn ensure_gateway(endpoint: &str) -> Result<()> {
    match connect_with_retry(endpoint).await {
        Ok(_) => return Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) => {}
        Err(error) => return Err(error).context("connect to the local Akra Gateway"),
    }

    let mut command = gateway_command()?;
    let mut child =
        crate::adapter::outbound::app_server::gateway_relay::start_gateway(&mut command)?;
    let startup = tokio::time::timeout(Duration::from_secs(10), async {
        // A concurrent launch can win the Gateway instance lock. Check the endpoint even when
        // our child has exited, since the winner may still be starting.
        loop {
            match connect_with_retry(endpoint).await {
                Ok(_) => return Ok(()),
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::NotFound | ErrorKind::ConnectionRefused
                    ) => {}
                Err(error) => return Err(error).context("connect to the local Akra Gateway"),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    let exit = child.try_wait().context("check the Akra Gateway process")?;
    if exit.is_none() {
        // The daemon outlives the terminal. Reap it if it happens to stop while the TUI is open.
        super::super::core_effect_worker::try_spawn_worker_with_panic_fallback(
            move || {
                if let Err(error) = child.wait() {
                    tracing::warn!(%error, "failed to reap the Akra Gateway process");
                }
            },
            || tracing::warn!("Akra Gateway process reaper stopped unexpectedly"),
        )
        .context("start the Akra Gateway process reaper")?;
    }
    match startup {
        Ok(result) => result,
        Err(_) => bail!(
            "the Akra Gateway did not open its local endpoint within 10 seconds (process exit: {exit:?}); start akra-gateway directly to see its diagnostics"
        ),
    }
}

fn gateway_command() -> Result<Command> {
    let name = format!("akra-gateway{}", std::env::consts::EXE_SUFFIX);
    let current = std::env::current_exe().context("resolve the TUI executable")?;
    let sibling = current.with_file_name(&name);
    let override_path = std::env::var_os("AKRA_GATEWAY_BIN")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from);
    let (executable, embedded) = match override_path {
        Some(path) => (path, false),
        None if sibling.is_file() => (sibling, false),
        // npm distributes just the CLI binary.
        None => (current, true),
    };
    let mut command = Command::new(&executable);
    if embedded {
        command.arg(crate::adapter::inbound::gateway::EMBEDDED_GATEWAY_COMMAND);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(command)
}
