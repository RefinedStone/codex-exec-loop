//! Per-attachment connection to the main-owned confirmation broker.
use crate::adapter::inbound::desktop_confirmation_bridge::protocol::{
    ClientRequest, FrameReader, IO_DEADLINE, Reply, nonce, valid_binding, valid_intent, write_frame,
};
use crate::adapter::inbound::desktop_confirmation_bridge::storage::{SessionRecord, read_session};
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationBinding, IssueConfirmationIntent, VerifiedHumanConfirmation,
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
type Connection = tokio::net::UnixStream;
#[cfg(windows)]
type Connection = tokio::net::windows::named_pipe::NamedPipeClient;

pub(super) struct ConfirmationRegistration {
    binding: IssueConfirmationBinding,
    path: PathBuf,
    run_dir: PathBuf,
    session: SessionRecord,
    alive: Arc<AtomicBool>,
    watch: tokio::task::JoinHandle<()>,
}
impl ConfirmationRegistration {
    pub async fn register(binding: IssueConfirmationBinding, path: &Path) -> Result<Self> {
        let run_dir = crate::configuration::akra_home_dir()?
            .join(crate::gateway_endpoint::RUN_DIRECTORY_NAME);
        Self::register_at(binding, path, &run_dir).await
    }
    pub(super) async fn register_at(
        binding: IssueConfirmationBinding,
        path: &Path,
        run_dir: &Path,
    ) -> Result<Self> {
        ensure!(valid_binding(&binding), "invalid confirmation binding");
        let session = read_session(path, &binding.registration_id, run_dir)?;
        exchange(
            &session.endpoint,
            &ClientRequest::Bind {
                registration_id: binding.registration_id.clone(),
                token: session.token.clone(),
                binding: binding.clone(),
            },
        )
        .await?;
        let mut connection = connect(&session.endpoint).await?;
        write_frame(
            &mut connection,
            &ClientRequest::Watch {
                registration_id: binding.registration_id.clone(),
                token: session.token.clone(),
                binding: binding.clone(),
            },
        )
        .await?;
        let mut reader = FrameReader::new(connection);
        read_reply(&mut reader).await?;
        let alive = Arc::new(AtomicBool::new(true));
        let watch_alive = alive.clone();
        let watch = tokio::spawn(async move {
            let _ = reader.read().await;
            watch_alive.store(false, Ordering::SeqCst);
        });
        Ok(Self {
            binding,
            path: path.to_path_buf(),
            run_dir: run_dir.to_path_buf(),
            session,
            alive,
            watch,
        })
    }
    pub fn binding(&self) -> &IssueConfirmationBinding {
        &self.binding
    }
    pub fn available(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
    pub async fn consume(
        &self,
        proof_id: &str,
        intent: &IssueConfirmationIntent,
    ) -> Result<VerifiedHumanConfirmation> {
        ensure!(
            self.available()
                && nonce(proof_id, 32)
                && valid_intent(intent)
                && intent.binding == self.binding,
            "invalid confirmation"
        );
        // Revalidate the credential file for every action. Revocation removes it.
        let current = read_session(&self.path, &self.binding.registration_id, &self.run_dir)?;
        ensure!(current == self.session, "confirmation registration changed");
        let reply = exchange(
            &self.session.endpoint,
            &ClientRequest::Consume {
                registration_id: self.binding.registration_id.clone(),
                token: self.session.token.clone(),
                proof_id: proof_id.to_owned(),
                intent: Box::new(intent.clone()),
            },
        )
        .await?;
        ensure!(self.available(), "confirmation connection ended");
        let reference = reply
            .get("confirmation_ref")
            .and_then(Value::as_str)
            .context("invalid confirmation response")?;
        VerifiedHumanConfirmation::from_verified(intent.target.clone(), reference.to_owned())
            .map_err(Into::into)
    }
}
impl Drop for ConfirmationRegistration {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.watch.abort();
    }
}
async fn exchange(endpoint: &str, request: &ClientRequest) -> Result<Value> {
    let mut connection = connect(endpoint).await?;
    write_frame(&mut connection, request).await?;
    read_reply(&mut FrameReader::new(connection)).await
}
async fn read_reply<R: tokio::io::AsyncRead + Unpin>(reader: &mut FrameReader<R>) -> Result<Value> {
    let value = tokio::time::timeout(IO_DEADLINE, reader.read())
        .await??
        .context("confirmation broker closed")?;
    serde_json::from_value::<Reply>(value)
        .context("invalid confirmation response")?
        .into_result()
}
async fn connect(endpoint: &str) -> Result<Connection> {
    tokio::time::timeout(IO_DEADLINE, async {
        #[cfg(unix)]
        {
            tokio::net::UnixStream::connect(endpoint)
                .await
                .map_err(anyhow::Error::from)
        }
        #[cfg(windows)]
        {
            loop {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
                    Ok(connection) => return Ok(connection),
                    Err(error) if error.raw_os_error() == Some(231) => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    Err(error) => return Err(anyhow::Error::from(error)),
                }
            }
        }
    })
    .await?
}

#[cfg(test)]
mod tests;
