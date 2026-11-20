//! Desktop main owns inherited stdin. Local clients can only bind, watch, and consume.
pub(crate) mod protocol;
pub(crate) mod storage;
#[cfg(test)]
mod tests;

use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationBinding, IssueConfirmationIntent,
};
use crate::private_ipc::{ClientConnection, PrivateIpcListener};
use anyhow::{Context, Result};
use protocol::{
    ClientRequest, ControlRequest, FrameReader, IO_DEADLINE, Reply, valid_binding, valid_intent,
    write_frame,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use storage::{SessionFiles, random_id};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

const MAX_REGISTRATIONS: usize = 32;
const MAX_CONNECTIONS: usize = 64;
const MAX_PROOFS: usize = 256;
const PROOF_LIFETIME: Duration = Duration::from_secs(5 * 60);

struct Registration {
    token: String,
    session_file: PathBuf,
    binding: Option<IssueConfirmationBinding>,
    watching: bool,
    lifetime: watch::Sender<()>,
}
struct Proof {
    intent: IssueConfirmationIntent,
    confirmation_ref: String,
    expires: Instant,
    consumed: bool,
}
enum Admission {
    Reply(Reply),
    Watch {
        registration_id: String,
        lifetime: watch::Receiver<()>,
    },
}
enum ClientEvent {
    Request {
        request: Box<ClientRequest>,
        reply: oneshot::Sender<Admission>,
    },
    Disconnected {
        registration_id: String,
    },
}
struct Broker {
    files: SessionFiles,
    endpoint: String,
    registrations: HashMap<String, Registration>,
    proofs: HashMap<String, Proof>,
}

pub(super) async fn run() -> Result<()> {
    let run_dir =
        crate::configuration::akra_home_dir()?.join(crate::gateway_endpoint::RUN_DIRECTORY_NAME);
    run_io(&run_dir, tokio::io::stdin(), tokio::io::stdout()).await
}

pub(crate) async fn run_io<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    run_dir: &Path,
    input: R,
    mut output: W,
) -> Result<()> {
    let files = SessionFiles::create(run_dir)?;
    let mut listener = PrivateIpcListener::bind(&files.endpoint())?;
    let mut broker = Broker {
        endpoint: listener.endpoint(),
        files,
        registrations: HashMap::new(),
        proofs: HashMap::new(),
    };
    let mut clients = JoinSet::new();
    let result = serve(&mut broker, &mut listener, input, &mut output, &mut clients).await;
    broker.proofs.clear();
    broker.registrations.clear();
    drop(listener);
    clients.abort_all();
    while clients.join_next().await.is_some() {}
    let cleanup = broker.files.cleanup();
    result.and(cleanup)
}

async fn serve<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    broker: &mut Broker,
    listener: &mut PrivateIpcListener,
    input: R,
    output: &mut W,
    clients: &mut JoinSet<Result<()>>,
) -> Result<()> {
    let mut input = FrameReader::new(input);
    let (events, mut received) = mpsc::channel(MAX_CONNECTIONS);
    write_frame(output, &json!({"type":"ready"})).await?;
    loop {
        tokio::select! {
            frame = input.read() => {
                let Some(frame) = frame.context("invalid confirmation control frame")? else { return Ok(()); };
                broker.control(frame, output).await?;
            }
            accepted = listener.accept(), if clients.len() < MAX_CONNECTIONS => {
                clients.spawn(client_session(accepted.context("confirmation IPC accept failed")?, events.clone()));
            }
            Some(event) = received.recv() => {
                match event {
                    ClientEvent::Request { request, reply } => {
                        let admission = broker.admit(*request);
                        if let Err(Admission::Watch { registration_id, .. }) = reply.send(admission) {
                            broker.revoke(&registration_id)?;
                        }
                    }
                    ClientEvent::Disconnected { registration_id } => broker.revoke(&registration_id)?,
                }
            }
            Some(joined) = clients.join_next(), if !clients.is_empty() => {
                if !matches!(joined, Ok(Ok(()))) { tracing::debug!("confirmation client closed"); }
            }
        }
    }
}

impl Broker {
    async fn control<W: AsyncWrite + Unpin>(&mut self, value: Value, output: &mut W) -> Result<()> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .context("invalid confirmation control request")?
            .to_owned();
        let (id, reply) = match serde_json::from_value::<ControlRequest>(value) {
            Ok(ControlRequest::Prepare { id }) => (id, self.prepare()?),
            Ok(ControlRequest::Approve {
                id,
                registration_id,
                intent,
            }) => (id, self.approve(&registration_id, *intent)?),
            Ok(ControlRequest::Revoke {
                id,
                registration_id,
            }) => {
                self.revoke(&registration_id)?;
                (id, Reply::result(json!({})))
            }
            Err(_) => (id, Reply::denied()),
        };
        write_frame(output, &reply.control(&id)).await
    }
    fn prepare(&mut self) -> Result<Reply> {
        if self.registrations.len() >= MAX_REGISTRATIONS {
            return Ok(Reply::denied());
        }
        let registration_id = random_id(16)?;
        let token = random_id(32)?;
        let session_file = self
            .files
            .prepare(&self.endpoint, &registration_id, &token)?;
        let reply =
            Reply::result(json!({"registration_id":registration_id,"session_file":session_file}));
        self.registrations.insert(
            registration_id,
            Registration {
                token,
                session_file,
                binding: None,
                watching: false,
                lifetime: watch::channel(()).0,
            },
        );
        Ok(reply)
    }
    fn approve(&mut self, registration_id: &str, intent: IssueConfirmationIntent) -> Result<Reply> {
        self.expire();
        let Some(registration) = self.registrations.get(registration_id) else {
            return Ok(Reply::denied());
        };
        if !valid_intent(&intent)
            || intent.binding.registration_id != registration_id
            || registration.binding.as_ref() != Some(&intent.binding)
            || !registration.watching
            || self.proofs.len() >= MAX_PROOFS
        {
            return Ok(Reply::denied());
        }
        let proof_id = random_id(32)?;
        let confirmation_ref = format!("desktop-confirmation:{}", random_id(16)?);
        self.proofs.insert(
            proof_id.clone(),
            Proof {
                intent,
                confirmation_ref,
                expires: Instant::now() + PROOF_LIFETIME,
                consumed: false,
            },
        );
        Ok(Reply::result(json!({"proof_id":proof_id})))
    }
    fn revoke(&mut self, registration_id: &str) -> Result<()> {
        self.proofs
            .retain(|_, proof| proof.intent.binding.registration_id != registration_id);
        if let Some(registration) = self.registrations.remove(registration_id) {
            self.files.remove(&registration.session_file)?;
        }
        Ok(())
    }
    fn expire(&mut self) {
        let now = Instant::now();
        self.proofs.retain(|_, proof| proof.expires > now);
    }
    fn admit(&mut self, request: ClientRequest) -> Admission {
        self.expire();
        let (registration_id, token) = request.credentials();
        let Some(registration) = self
            .registrations
            .get_mut(registration_id)
            .filter(|registration| token_matches(&registration.token, token))
        else {
            return Admission::Reply(Reply::denied());
        };
        match request {
            ClientRequest::Bind {
                binding,
                registration_id,
                ..
            } => {
                if !valid_binding(&binding)
                    || binding.registration_id != registration_id
                    || registration
                        .binding
                        .as_ref()
                        .is_some_and(|current| current != &binding)
                {
                    return Admission::Reply(Reply::denied());
                }
                registration.binding = Some(binding);
                Admission::Reply(Reply::result(json!({})))
            }
            ClientRequest::Watch {
                binding,
                registration_id,
                ..
            } => {
                if registration.binding.as_ref() != Some(&binding) || registration.watching {
                    return Admission::Reply(Reply::denied());
                }
                registration.watching = true;
                Admission::Watch {
                    registration_id,
                    lifetime: registration.lifetime.subscribe(),
                }
            }
            ClientRequest::Consume {
                proof_id, intent, ..
            } => {
                if !registration.watching || registration.binding.as_ref() != Some(&intent.binding)
                {
                    return Admission::Reply(Reply::denied());
                }
                let Some(proof) = self
                    .proofs
                    .get_mut(&proof_id)
                    .filter(|proof| proof.intent == *intent)
                else {
                    return Admission::Reply(Reply::denied());
                };
                // Only this exact intent can be retried. Kernel CommandMemory owns the single effect.
                proof.consumed = true;
                Admission::Reply(Reply::result(
                    json!({"confirmation_ref":proof.confirmation_ref}),
                ))
            }
        }
    }
}

fn token_matches(expected: &str, supplied: &str) -> bool {
    expected.len() == supplied.len()
        && expected
            .as_bytes()
            .iter()
            .zip(supplied.as_bytes())
            .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
            == 0
}

async fn client_session(
    connection: ClientConnection,
    events: mpsc::Sender<ClientEvent>,
) -> Result<()> {
    let (reader, mut writer) = tokio::io::split(connection);
    let mut reader = FrameReader::new(reader);
    let request = match tokio::time::timeout(IO_DEADLINE, reader.read()).await {
        Ok(Ok(Some(value))) => serde_json::from_value::<ClientRequest>(value).ok(),
        Ok(Ok(None)) => return Ok(()),
        _ => None,
    };
    let Some(request) = request else {
        return write_frame(&mut writer, &Reply::denied()).await;
    };
    let (reply, admission) = oneshot::channel();
    events
        .send(ClientEvent::Request {
            request: Box::new(request),
            reply,
        })
        .await
        .context("confirmation broker closed")?;
    match admission.await.context("confirmation broker closed")? {
        Admission::Reply(reply) => write_frame(&mut writer, &reply).await?,
        Admission::Watch {
            registration_id,
            mut lifetime,
        } => {
            let sent = write_frame(&mut writer, &Reply::result(json!({}))).await;
            if sent.is_ok() {
                tokio::select! { _ = lifetime.changed() => {} _ = reader.read() => {} }
            }
            let _ = events
                .send(ClientEvent::Disconnected { registration_id })
                .await;
            sent?;
        }
    }
    Ok(())
}
