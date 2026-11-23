//! Credential migration has its own owner-only endpoint and never enters the public Gateway recorder.
use crate::application::port::outbound::provider_credentials_port::NativeOperationOwner;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialMigrationPort;
use crate::domain::provider_credentials::{ConnectionRef, CredentialResult};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
mod client;
mod protocol;
mod server;
#[cfg(test)]
mod tests;
pub(crate) use client::run;

pub(crate) type LinkValidator =
    Arc<dyn Fn(&ConnectionRef, &str) -> CredentialResult<String> + Send + Sync>;
struct State {
    profile_id: String,
    service: Arc<dyn ProviderCredentialMigrationPort>,
    validate_link: LinkValidator,
    workers: Arc<Semaphore>,
    cancellations: mpsc::SyncSender<(NativeOperationOwner, OwnedSemaphorePermit)>,
}
pub(crate) struct MigrationServer {
    stop: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl MigrationServer {
    pub(crate) async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for MigrationServer {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}
pub(crate) fn start(
    profile_id: String,
    runtime: PathBuf,
    service: Arc<dyn ProviderCredentialMigrationPort>,
    validate_link: LinkValidator,
) -> anyhow::Result<MigrationServer> {
    if !crate::domain::provider_credentials::valid_profile_id(&profile_id) {
        anyhow::bail!("provider migration endpoint is unavailable");
    }
    crate::private_directory::ensure_private_directory(&runtime)
        .map_err(|_| anyhow::anyhow!("provider migration endpoint is unavailable"))?;
    let endpoint = endpoint(&profile_id, &runtime)?;
    let listener = crate::private_ipc::PrivateIpcListener::bind(&endpoint)
        .map_err(|_| anyhow::anyhow!("provider migration endpoint is unavailable"))?;
    let state = new_state(profile_id, service, validate_link);
    let (stop, receiver) = watch::channel(false);
    let task = tokio::spawn(server::listen(listener, state, receiver));
    Ok(MigrationServer {
        stop,
        task: Some(task),
    })
}
fn new_state(
    profile_id: String,
    service: Arc<dyn ProviderCredentialMigrationPort>,
    validate_link: LinkValidator,
) -> Arc<State> {
    // A queued cancellation retains its session permit until durable fencing finishes.
    let (cancellations, receiver) =
        mpsc::sync_channel::<(NativeOperationOwner, OwnedSemaphorePermit)>(8);
    let cancellation_service = service.clone();
    std::thread::spawn(move || {
        while let Ok((owner, _permit)) = receiver.recv() {
            let _ = cancellation_service.cancel_owner(&owner);
        }
    });
    Arc::new(State {
        profile_id,
        service,
        validate_link,
        workers: Arc::new(Semaphore::new(4)),
        cancellations,
    })
}
fn endpoint(profile_id: &str, runtime: &Path) -> anyhow::Result<String> {
    if !crate::domain::provider_credentials::valid_profile_id(profile_id) {
        anyhow::bail!("provider migration endpoint is unavailable");
    }
    #[cfg(windows)]
    {
        let _ = runtime;
        let sid = crate::private_fs::windows_current_user_sid_string()
            .map_err(|_| anyhow::anyhow!("provider migration endpoint is unavailable"))?;
        Ok(format!(
            r"\\.\pipe\akra-provider-credentials-{sid}-{profile_id}"
        ))
    }
    #[cfg(unix)]
    {
        let path = runtime.join("migration.sock");
        if path.as_os_str().len() > 103 {
            anyhow::bail!("provider migration endpoint is unavailable");
        }
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("provider migration endpoint is unavailable"))
    }
}
