//! A process-local authenticated broker. It never constructs a Gateway backend.

mod protocol;
mod storage;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::private_ipc::{ClientConnection, PrivateIpcListener};
use protocol::{
    ClientRequest, ControlRequest, FrameReader, IO_DEADLINE, Reply, known_tool, write_frame,
};
use storage::{SessionFiles, random_id};

const MAX_REGISTRATIONS: usize = 32;
const MAX_CONNECTIONS: usize = 64;
const CALL_DEADLINE: Duration = Duration::from_secs(60);
const HANDOFF_DEADLINE: Duration = Duration::from_secs(10 * 60);

struct Registration {
    token: String,
    session_file: PathBuf,
    thread_id: Option<String>,
    pending: Option<String>,
    lifetime: watch::Sender<()>,
}

struct PendingCall {
    registration_id: String,
    reply: oneshot::Sender<Reply>,
    deadline: Instant,
}

enum Admission {
    Rejected(Reply),
    Watch {
        registration_id: String,
        lifetime: watch::Receiver<()>,
    },
    Call {
        request_id: String,
        reply: oneshot::Receiver<Reply>,
    },
}

enum ClientEvent {
    Request {
        request: ClientRequest,
        reply: oneshot::Sender<Admission>,
    },
    Disconnected {
        request_id: String,
    },
}

struct Broker {
    files: SessionFiles,
    endpoint: String,
    registrations: HashMap<String, Registration>,
    pending: HashMap<String, PendingCall>,
}

pub(super) async fn run() -> Result<()> {
    let run_dir =
        crate::configuration::akra_home_dir()?.join(crate::gateway_endpoint::RUN_DIRECTORY_NAME);
    run_io(&run_dir, tokio::io::stdin(), tokio::io::stdout()).await
}

async fn run_io<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
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
        pending: HashMap::new(),
    };
    let mut clients = JoinSet::new();
    let result = serve(&mut broker, &mut listener, input, &mut output, &mut clients).await;
    // Stop the listener before deleting its private directory. Aborting the
    // bounded client set closes watches and unauthenticated connections too.
    drop(listener);
    let shutdown = broker.shutdown(&mut output).await;
    clients.abort_all();
    while let Some(joined) = clients.join_next().await {
        if let Err(error) = joined
            && !error.is_cancelled()
        {
            tracing::warn!("browser client task failed during shutdown");
        }
    }
    let cleanup = broker.files.cleanup();
    result.and(shutdown).and(cleanup)
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
        let deadline = broker.pending.values().map(|call| call.deadline).min();
        tokio::select! {
            frame = input.read() => {
                let Some(frame) = frame.context("invalid browser control frame")? else { return Ok(()); };
                broker.control(frame, output).await?;
            }
            accepted = listener.accept(), if clients.len() < MAX_CONNECTIONS => {
                let connection = accepted.context("browser IPC accept failed")?;
                clients.spawn(client_session(connection, events.clone()));
            }
            Some(event) = received.recv() => {
                match event {
                    ClientEvent::Request { request, reply } => {
                        let (admission, call) = broker.admit(request)?;
                        if let Some(call) = call {
                            write_frame(output, &call).await?;
                        }
                        if let Err(Admission::Call { request_id, .. }) = reply.send(admission) {
                            broker.cancel(&request_id, "disconnected", output).await?;
                        }
                    }
                    ClientEvent::Disconnected { request_id } => {
                        broker.cancel(&request_id, "disconnected", output).await?;
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                broker.expire(Instant::now(), output).await?;
            }
            Some(joined) = clients.join_next(), if !clients.is_empty() => {
                match joined {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => tracing::debug!("browser client I/O failed"),
                    Err(error) => return Err(error).context("browser client task failed"),
                }
            }
        }
    }
}

impl Broker {
    async fn control<W: AsyncWrite + Unpin>(&mut self, value: Value, output: &mut W) -> Result<()> {
        let request = match serde_json::from_value::<ControlRequest>(value.clone()) {
            Ok(request) => request,
            Err(_) => {
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .context("invalid browser control request")?;
                return write_frame(
                    output,
                    &Reply::error("invalid_request", "Invalid control request").control(id),
                )
                .await;
            }
        };
        let (id, response) = match request {
            ControlRequest::Prepare { id } => {
                let result = if self.registrations.len() >= MAX_REGISTRATIONS {
                    Reply::error("capacity", "Registration limit reached")
                } else {
                    let registration_id = random_id(16)?;
                    let token = random_id(32)?;
                    let session_file =
                        self.files
                            .prepare(&self.endpoint, &registration_id, &token)?;
                    let result = Reply::result(
                        json!({"registrationId":registration_id,"sessionFile":session_file}),
                    );
                    self.registrations.insert(
                        registration_id,
                        Registration {
                            token,
                            session_file,
                            thread_id: None,
                            pending: None,
                            lifetime: watch::channel(()).0,
                        },
                    );
                    result
                };
                (id, result)
            }
            ControlRequest::Bind {
                id,
                registration_id,
                thread_id,
            } => {
                let reply = self.bind(&registration_id, &thread_id);
                (id, reply)
            }
            ControlRequest::Revoke {
                id,
                registration_id,
            } => {
                if let Some(registration) = self.registrations.remove(&registration_id) {
                    if let Some(request_id) = &registration.pending {
                        self.cancel(request_id, "revoked", output).await?;
                    }
                    self.files.remove(&registration.session_file)?;
                    // Dropping the only sender closes every authenticated watch.
                    drop(registration);
                }
                (id, Reply::result(json!({})))
            }
            ControlRequest::Answer { request_id, reply } => {
                self.finish(&request_id, reply);
                return Ok(());
            }
        };
        write_frame(output, &response.control(&id)).await
    }

    fn bind(&mut self, registration_id: &str, thread_id: &str) -> Reply {
        if thread_id.is_empty() || thread_id.len() > 512 {
            return Reply::error("invalid_thread", "Invalid thread");
        }
        if self.registrations.iter().any(|(id, registration)| {
            id != registration_id && registration.thread_id.as_deref() == Some(thread_id)
        }) {
            return Reply::error("thread_owned", "Thread already has a browser registration");
        }
        let Some(registration) = self.registrations.get_mut(registration_id) else {
            return Reply::error("unavailable", "Registration unavailable");
        };
        if let Some(bound) = &registration.thread_id
            && bound != thread_id
        {
            return Reply::error("thread_mismatch", "Registration is bound to another thread");
        }
        registration.thread_id = Some(thread_id.to_owned());
        Reply::result(json!({}))
    }

    fn admit(&mut self, request: ClientRequest) -> Result<(Admission, Option<Value>)> {
        let (registration_id, token) = match &request {
            ClientRequest::Call {
                registration_id,
                token,
                ..
            }
            | ClientRequest::Watch {
                registration_id,
                token,
            } => (registration_id, token),
        };
        let rejected = |code, message| Ok((Admission::Rejected(Reply::error(code, message)), None));
        let Some(registration) = self
            .registrations
            .get_mut(registration_id)
            .filter(|registration| token_matches(&registration.token, token))
        else {
            return rejected("unauthorized", "Browser session authentication failed");
        };
        match request {
            ClientRequest::Watch {
                registration_id, ..
            } => Ok((
                Admission::Watch {
                    registration_id,
                    lifetime: registration.lifetime.subscribe(),
                },
                None,
            )),
            ClientRequest::Call {
                registration_id,
                thread_id,
                tool,
                arguments,
                ..
            } => {
                if registration.thread_id.as_deref() != Some(thread_id.as_str()) {
                    return rejected("thread_mismatch", "Browser session thread does not match");
                }
                if !known_tool(&tool) || !arguments.is_object() {
                    return rejected("invalid_request", "Invalid browser tool request");
                }
                if registration.pending.is_some() {
                    return rejected("busy", "Browser registration already has a pending call");
                }
                let request_id = random_id(16)?;
                let call = json!({
                    "type":"call","requestId":request_id,"registrationId":registration_id,
                    "threadId":thread_id,"tool":tool,"arguments":arguments
                });
                if serde_json::to_vec(&call)?.len() >= protocol::MAX_FRAME_BYTES {
                    return rejected("invalid_request", "Browser call exceeds frame limit");
                }
                let (sender, reply) = oneshot::channel();
                let deadline = Instant::now()
                    + if tool == "browser_handoff" {
                        HANDOFF_DEADLINE
                    } else {
                        CALL_DEADLINE
                    };
                registration.pending = Some(request_id.clone());
                self.pending.insert(
                    request_id.clone(),
                    PendingCall {
                        registration_id: registration_id.clone(),
                        reply: sender,
                        deadline,
                    },
                );
                Ok((Admission::Call { request_id, reply }, Some(call)))
            }
        }
    }

    fn finish(&mut self, request_id: &str, reply: Reply) -> bool {
        let Some(pending) = self.pending.remove(request_id) else {
            return false;
        };
        if let Some(registration) = self.registrations.get_mut(&pending.registration_id) {
            registration.pending = None;
        }
        if pending.reply.send(reply).is_err() {
            tracing::debug!("browser client disconnected before reply");
        }
        true
    }

    async fn cancel<W: AsyncWrite + Unpin>(
        &mut self,
        request_id: &str,
        reason: &str,
        output: &mut W,
    ) -> Result<()> {
        if self.finish(request_id, Reply::error(reason, "Browser call cancelled")) {
            write_frame(output, &json!({"type":"cancel","requestId":request_id})).await?;
        }
        Ok(())
    }

    async fn expire<W: AsyncWrite + Unpin>(&mut self, now: Instant, output: &mut W) -> Result<()> {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, call)| call.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.cancel(&id, "timeout", output).await?;
        }
        Ok(())
    }

    async fn shutdown<W: AsyncWrite + Unpin>(&mut self, output: &mut W) -> Result<()> {
        self.registrations.clear();
        let pending: Vec<_> = self.pending.keys().cloned().collect();
        for id in pending {
            self.cancel(&id, "closed", output).await?;
        }
        Ok(())
    }
}

fn token_matches(expected: &str, supplied: &str) -> bool {
    expected.len() == supplied.len()
        && expected
            .bytes()
            .zip(supplied.bytes())
            .fold(0u8, |diff, (left, right)| diff | (left ^ right))
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
        return write_frame(
            &mut writer,
            &Reply::error("invalid_request", "Invalid browser request"),
        )
        .await;
    };
    let (reply, admitted) = oneshot::channel();
    events
        .send(ClientEvent::Request { request, reply })
        .await
        .context("browser broker closed")?;
    match admitted.await.context("browser broker closed")? {
        Admission::Rejected(reply) => write_frame(&mut writer, &reply).await?,
        Admission::Watch {
            registration_id,
            mut lifetime,
        } => {
            write_frame(
                &mut writer,
                &Reply::result(json!({"registrationId":registration_id})),
            )
            .await?;
            tokio::select! {
                _ = lifetime.changed() => {}
                _ = reader.read() => {}
            }
        }
        Admission::Call { request_id, reply } => {
            tokio::select! {
                response = reply => {
                    write_frame(&mut writer, &response.context("browser broker closed")?).await?;
                }
                _ = reader.read() => {
                    events.send(ClientEvent::Disconnected { request_id }).await.context("browser broker closed")?;
                }
            }
        }
    }
    Ok(())
}
