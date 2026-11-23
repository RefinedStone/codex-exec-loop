//! Blocking OS calls are serialized on one dedicated thread. No secret crosses the public API.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, SyncSender},
};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

use crate::application::port::outbound::provider_credentials_port::{
    KeyringPart, ProviderKeyStoreBackend, ProviderKeyStorePort,
};
use crate::domain::provider_credentials::{
    CredentialResult, MAX_SECRET_PARTS, ProviderCredentialError, valid_connection_id,
    valid_profile_id,
};

const SERVICE: &str = "akra-provider-credentials-v1";
const MAX_CAPSULE_BYTES: usize = 2560;
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const CANCELLATION_POLL: Duration = Duration::from_millis(25);
type Reply<T> = SyncSender<CredentialResult<T>>;

#[derive(Clone)]
struct JobContext {
    cancelled: Arc<AtomicBool>,
    abandoned: Arc<AtomicBool>,
    deadline: Instant,
}
impl JobContext {
    fn live(&self) -> bool {
        !self.cancelled.load(Ordering::SeqCst)
            && !self.abandoned.load(Ordering::SeqCst)
            && Instant::now() < self.deadline
    }
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

enum Command {
    Read(KeyringPart, JobContext, Reply<Option<Zeroizing<Vec<u8>>>>),
    Write(KeyringPart, Zeroizing<Vec<u8>>, JobContext, Reply<()>),
    Delete(KeyringPart, JobContext, Reply<()>),
}

pub struct OsProviderKeyStoreAdapter {
    sender: SyncSender<Command>,
    supported: bool,
    reply_timeout: Duration,
}

impl OsProviderKeyStoreAdapter {
    /// Construction starts a worker but does not open, enumerate, unlock, or read the OS key store.
    pub fn new() -> CredentialResult<Self> {
        Self::with_backend(Box::new(OsBackend))
    }

    /// Tests must inject a fake. The same thread owns all calls for each adapter instance.
    pub fn with_backend(backend: Box<dyn ProviderKeyStoreBackend>) -> CredentialResult<Self> {
        Self::with_backend_timeout(backend, REPLY_TIMEOUT)
    }

    fn with_backend_timeout(
        mut backend: Box<dyn ProviderKeyStoreBackend>,
        reply_timeout: Duration,
    ) -> CredentialResult<Self> {
        let (sender, receiver) = mpsc::sync_channel(32);
        // Deliberately do not retain/join this handle: an OS call may be uninterruptible.
        // Each queued job has its own deadline/fence, so a late worker skips abandoned writes.
        std::thread::Builder::new()
            .name("provider-key-store".to_owned())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    match command {
                        Command::Read(part, context, reply) => {
                            let result = if context.live() {
                                backend.read_part(&part)
                            } else {
                                Err(ProviderCredentialError::Cancelled)
                            };
                            let _ = reply.send(result);
                        }
                        Command::Write(part, capsule, context, reply) => {
                            let result = if context.live() {
                                backend.write_part(&part, &capsule)
                            } else {
                                Err(ProviderCredentialError::OutcomeUnknown)
                            };
                            let _ = reply.send(result);
                        }
                        Command::Delete(part, context, reply) => {
                            let result = if context.live() {
                                backend.delete_part(&part)
                            } else {
                                Err(ProviderCredentialError::OutcomeUnknown)
                            };
                            let _ = reply.send(result);
                        }
                    }
                }
            })
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        Ok(Self {
            sender,
            reply_timeout,
            supported: cfg!(any(
                target_os = "windows",
                target_os = "macos",
                target_os = "linux"
            )),
        })
    }

    fn request<T>(
        &self,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
        mutation: bool,
        make: impl FnOnce(JobContext, Reply<T>) -> Command,
    ) -> CredentialResult<T> {
        if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Err(ProviderCredentialError::Cancelled);
        }
        let context = JobContext {
            cancelled,
            abandoned: Arc::new(AtomicBool::new(false)),
            deadline: deadline.min(Instant::now() + self.reply_timeout),
        };
        let _cancel_on_return = CancelOnDrop(context.abandoned.clone());
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(make(context.clone(), sender))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => ProviderCredentialError::Busy,
                mpsc::TrySendError::Disconnected(_) => ProviderCredentialError::Unavailable,
            })?;
        let unknown = if mutation {
            ProviderCredentialError::OutcomeUnknown
        } else {
            ProviderCredentialError::Cancelled
        };
        loop {
            if !context.live() {
                return Err(unknown);
            }
            let remaining = context.deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(remaining.min(CANCELLATION_POLL)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(unknown),
            }
        }
    }
}

impl ProviderKeyStorePort for OsProviderKeyStoreAdapter {
    fn supported(&self) -> bool {
        self.supported
    }
    fn read_part(&self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        self.read_part_until(
            part,
            Arc::new(AtomicBool::new(false)),
            Instant::now() + self.reply_timeout,
        )
    }
    fn write_part(&self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()> {
        self.write_part_until(
            part,
            capsule,
            Arc::new(AtomicBool::new(false)),
            Instant::now() + self.reply_timeout,
        )
    }
    fn delete_part(&self, part: &KeyringPart) -> CredentialResult<()> {
        self.delete_part_until(
            part,
            Arc::new(AtomicBool::new(false)),
            Instant::now() + self.reply_timeout,
        )
    }
    fn read_part_until(
        &self,
        part: &KeyringPart,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
    ) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        validate_part(part)?;
        let value = self.request(cancelled, deadline, false, |context, reply| {
            Command::Read(part.clone(), context, reply)
        })?;
        if value
            .as_ref()
            .is_some_and(|value| value.len() > MAX_CAPSULE_BYTES)
        {
            return Err(ProviderCredentialError::Corrupt);
        }
        Ok(value)
    }
    fn write_part_until(
        &self,
        part: &KeyringPart,
        capsule: &[u8],
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
    ) -> CredentialResult<()> {
        validate_part(part)?;
        if capsule.is_empty() || capsule.len() > MAX_CAPSULE_BYTES {
            return Err(ProviderCredentialError::TooLarge);
        }
        self.request(cancelled, deadline, true, |context, reply| {
            Command::Write(
                part.clone(),
                Zeroizing::new(capsule.to_vec()),
                context,
                reply,
            )
        })
    }
    fn delete_part_until(
        &self,
        part: &KeyringPart,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
    ) -> CredentialResult<()> {
        validate_part(part)?;
        self.request(cancelled, deadline, true, |context, reply| {
            Command::Delete(part.clone(), context, reply)
        })
    }
}

fn validate_part(part: &KeyringPart) -> CredentialResult<()> {
    if !valid_profile_id(&part.profile_id)
        || !valid_connection_id(&part.connection_id)
        || usize::from(part.index) >= MAX_SECRET_PARTS
    {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    Ok(())
}

struct OsBackend;
impl OsBackend {
    fn entry(part: &KeyringPart) -> CredentialResult<keyring::Entry> {
        validate_part(part)?;
        let account = format!(
            "{}:{}:{}:{}",
            part.profile_id,
            part.provider.as_str(),
            part.connection_id,
            part.index
        );
        keyring::Entry::new(SERVICE, &account).map_err(keyring_error)
    }
}
impl ProviderKeyStoreBackend for OsBackend {
    fn read_part(&mut self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        match Self::entry(part)?.get_secret() {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(error)),
        }
    }
    fn write_part(&mut self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()> {
        Self::entry(part)?
            .set_secret(capsule)
            .map_err(keyring_error)
    }
    fn delete_part(&mut self, part: &KeyringPart) -> CredentialResult<()> {
        match Self::entry(part)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(error)),
        }
    }
}
fn keyring_error(error: keyring::Error) -> ProviderCredentialError {
    match error {
        keyring::Error::NoStorageAccess(_) => ProviderCredentialError::Locked,
        keyring::Error::BadEncoding(mut bytes) => {
            bytes.zeroize();
            ProviderCredentialError::Corrupt
        }
        keyring::Error::TooLong(_, _) => ProviderCredentialError::TooLarge,
        _ => ProviderCredentialError::Unavailable,
    }
}

#[cfg(test)]
mod tests;
