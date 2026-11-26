//! 실제 OS credential 저장소 대신 별도 프로세스의 합성 저장소를 사용한다.
use super::super::{EvidenceVaultUnlockService, VaultSharedCopyReason, record_key};
use crate::adapter::outbound::vault_key_store::test_support::{
    CredentialBackend, adapter_with_store, diagnostics as initialization_diagnostics,
};
use crate::application::port::outbound::evidence_vault_port::VaultKey;
use crate::application::port::outbound::vault_key_store_port::{
    VaultKeyStoreError, VaultKeyStorePort,
};
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{HashSet, VecDeque};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

mod child;
use child::ManagedChild;

const SERVER_TEST: &str = "application::service::evidence_vault_unlock::process_initialization_tests::credential_server_process";
const WORKER_TEST: &str = "application::service::evidence_vault_unlock::process_initialization_tests::sealing_worker_process";
const CONTROL_ENV: &str = "AKRA_TEST_VAULT_KEY_CONTROL";
const CREDENTIAL_ENV: &str = "AKRA_TEST_VAULT_KEY_CREDENTIAL";
const WORKER_ENV: &str = "AKRA_TEST_VAULT_KEY_WORKER";
const RUNTIME_ENV: &str = "AKRA_TEST_VAULT_KEY_RUNTIME";
const MODE_ENV: &str = "AKRA_TEST_VAULT_KEY_MODE";
const GATE_ENV: &str = "AKRA_TEST_VAULT_KEY_LOAD_GATE";
const LIFETIME: Duration = Duration::from_secs(30);
const MAX_FRAME: u64 = 16 * 1024;
const PROOF: &[u8] = b"synthetic-key-initialization-proof";

// 키 원문이 포함되는 DTO에는 Debug를 구현하지 않는다.
#[derive(Serialize, Deserialize)]
enum CredentialRequest {
    Load { worker: u8 },
    BeforeStore { worker: u8 },
    Store { worker: u8, key: Vec<u8> },
    Finished(WorkerResult),
}
#[derive(Serialize, Deserialize)]
enum CredentialReply {
    Loaded(Option<Vec<u8>>),
    Stored,
    Received,
}
#[derive(Serialize, Deserialize)]
enum Control {
    ReleaseInitialLoads,
    CommitNextStore { acknowledge: bool },
    Snapshot,
    Verify { ciphertext: Vec<u8> },
    Stop,
}
#[derive(Serialize, Deserialize)]
pub(super) enum Notice {
    Ready { address: SocketAddr },
    LoadWaiting { worker: u8 },
    BeforeStoreWaiting { worker: u8 },
    StoreWaiting { worker: u8, key_id: String },
    Finished(WorkerResult),
    Snapshot(Snapshot),
    Verified(bool),
}
impl Notice {
    pub(super) fn summary(&self) -> String {
        match self {
            Self::Ready { .. } => "Ready".into(),
            Self::LoadWaiting { worker } => format!("LoadWaiting worker={worker}"),
            Self::BeforeStoreWaiting { worker } => format!("BeforeStoreWaiting worker={worker}"),
            Self::StoreWaiting { worker, .. } => format!("StoreWaiting worker={worker}"),
            Self::Finished(result) => format!(
                "Finished worker={} failure={} has_key={} has_ciphertext={} diagnostics={:?}",
                result.worker,
                match result.failure.as_deref() {
                    None => "none",
                    Some("KeyStoreUnavailable") => "KeyStoreUnavailable",
                    Some(_) => "unexpected_failure",
                },
                result.key_id.is_some(),
                result.ciphertext.is_some(),
                result.diagnostics,
            ),
            Self::Snapshot(_) => "Snapshot".into(),
            Self::Verified(matches) => format!("Verified matches={matches}"),
        }
    }
}
#[derive(Serialize, Deserialize)]
pub(super) struct WorkerResult {
    pub worker: u8,
    pub key_id: Option<String>,
    pub ciphertext: Option<Vec<u8>>,
    pub failure: Option<String>,
    pub diagnostics: Vec<initialization_diagnostics::Diagnostic>,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub key_id: Option<String>,
    pub loads: usize,
    pub store_requests: usize,
    pub applied_stores: usize,
    pub pending_stores: usize,
    pub before_store_waiters: usize,
}
enum ServerEvent {
    Credential(TcpStream, CredentialRequest),
    Control(Control),
}
struct PendingStore {
    stream: TcpStream,
    key: VaultKey,
}

fn write_frame<T: Serialize>(stream: &mut TcpStream, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)
}

fn read_frame<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<T> {
    let mut line = String::new();
    let count = Read::take(reader, MAX_FRAME + 1).read_line(&mut line)?;
    if count == 0 || count as u64 > MAX_FRAME || !line.ends_with('\n') {
        return Err(io::Error::other("invalid synthetic credential frame"));
    }
    serde_json::from_str(&line).map_err(io::Error::other)
}

fn configure(stream: &TcpStream) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(LIFETIME))?;
    stream.set_write_timeout(Some(LIFETIME))?;
    stream.set_nodelay(true)
}

struct TemporaryRuntime(PathBuf);
impl TemporaryRuntime {
    fn new() -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-key-initialization-{}-{}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&path).expect("private test runtime");
        // macOS의 /var -> /private/var 별칭을 실제 coordinator의 symlink 검사와 혼동하지 않는다.
        Self(std::fs::canonicalize(path).expect("canonical test runtime"))
    }
}
impl Drop for TemporaryRuntime {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Controller {
    reader: BufReader<TcpStream>,
    deadline: Instant,
}
impl Controller {
    fn send(&mut self, command: Control) {
        write_frame(self.reader.get_mut(), &command).expect("parent control write");
    }

    fn next(&mut self) -> Result<Notice, String> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("parent_control_deadline".into());
        }
        self.reader
            .get_mut()
            .set_read_timeout(Some(remaining))
            .map_err(|error| {
                format!(
                    "control_timeout kind={:?} code={:?}",
                    error.kind(),
                    error.raw_os_error()
                )
            })?;
        read_frame(&mut self.reader).map_err(|error| {
            format!(
                "control_read kind={:?} code={:?}",
                error.kind(),
                error.raw_os_error()
            )
        })
    }
}

fn accept_before(listener: &TcpListener, deadline: Instant) -> TcpStream {
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                configure(&stream).expect("control socket");
                return stream;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => panic!("synthetic control accept failed"),
        }
        assert!(
            Instant::now() < deadline,
            "synthetic server startup deadline"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Clone, Copy)]
pub(super) enum WorkerMode {
    Seal,
    PauseBeforeStore,
    ConstructOnly,
    ReadOnly,
}
impl WorkerMode {
    fn name(self) -> &'static str {
        match self {
            Self::Seal => "seal",
            Self::PauseBeforeStore => "pause_before_store",
            Self::ConstructOnly => "construct_only",
            Self::ReadOnly => "read_only",
        }
    }
}

// 선언 순서대로 worker -> control -> server -> runtime을 정리한다.
pub(super) struct Harness {
    workers: Vec<(u8, ManagedChild)>,
    controller: Controller,
    server: ManagedChild,
    runtime: TemporaryRuntime,
    address: SocketAddr,
    deadline: Instant,
    last_notice: String,
}
impl Harness {
    pub fn new(initial_loads: usize) -> Self {
        let deadline = Instant::now() + LIFETIME;
        let runtime = TemporaryRuntime::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("parent listener");
        let mut server = ManagedChild::spawn(
            SERVER_TEST,
            &[
                (
                    CONTROL_ENV,
                    listener.local_addr().unwrap().to_string().into(),
                ),
                (GATE_ENV, initial_loads.to_string().into()),
            ],
        );
        let mut controller = Controller {
            reader: BufReader::new(accept_before(&listener, deadline)),
            deadline,
        };
        let notice = controller.next().unwrap_or_else(|error| {
            panic!("server readiness failed: {error}; {}", server.summary())
        });
        let Notice::Ready { address } = notice else {
            panic!("expected server readiness, got {}", notice.summary())
        };
        Self {
            workers: Vec::new(),
            controller,
            server,
            runtime,
            address,
            deadline,
            last_notice: "Ready".into(),
        }
    }

    pub fn spawn(&mut self, worker: u8, mode: WorkerMode) {
        assert!(
            self.workers.iter().all(|(id, _)| *id != worker),
            "unique worker id"
        );
        let child = ManagedChild::spawn(
            WORKER_TEST,
            &[
                (CREDENTIAL_ENV, self.address.to_string().into()),
                (WORKER_ENV, worker.to_string().into()),
                (
                    RUNTIME_ENV,
                    self.runtime
                        .0
                        .join("coordination")
                        .as_os_str()
                        .to_os_string(),
                ),
                (MODE_ENV, mode.name().into()),
            ],
        );
        self.workers.push((worker, child));
    }

    pub fn release_loads(&mut self, workers: &[u8]) {
        self.try_release_loads(workers)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    pub fn try_release_loads(&mut self, workers: &[u8]) -> Result<(), String> {
        let expected: HashSet<_> = workers.iter().copied().collect();
        let mut waiting = HashSet::new();
        for _ in workers {
            let notice = self.next_result("initial_load_barrier")?;
            let Notice::LoadWaiting { worker } = notice else {
                return Err(format!(
                    "initial_load_barrier expected={expected:?} waiting={waiting:?} unexpected={}; {}",
                    notice.summary(),
                    self.diagnostics()
                ));
            };
            if !expected.contains(&worker) || !waiting.insert(worker) {
                return Err(format!(
                    "initial_load_barrier unexpected/duplicate worker={worker} expected={expected:?} waiting={waiting:?}; {}",
                    self.diagnostics()
                ));
            }
        }
        assert_eq!(waiting, expected);
        self.controller.send(Control::ReleaseInitialLoads);
        Ok(())
    }

    fn next_result(&mut self, stage: &str) -> Result<Notice, String> {
        match self.controller.next() {
            Ok(notice) => {
                self.last_notice = notice.summary();
                Ok(notice)
            }
            Err(error) => Err(format!("stage={stage} {error}; {}", self.diagnostics())),
        }
    }

    pub fn next(&mut self) -> Notice {
        self.next_result("next_notice")
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn diagnostics(&mut self) -> String {
        let children: Vec<_> = self
            .workers
            .iter_mut()
            .map(|(worker, child)| format!("worker={worker} {}", child.summary()))
            .collect();
        format!(
            "last_notice={} workers={children:?} server={}",
            self.last_notice,
            self.server.summary()
        )
    }

    pub fn block_runtime_with_file(&self) {
        std::fs::write(
            self.runtime.0.join("coordination"),
            b"synthetic-runtime-blocker",
        )
        .expect("owned runtime blocker");
    }

    pub fn apply_next_store(&mut self, acknowledge: bool) {
        self.controller
            .send(Control::CommitNextStore { acknowledge });
    }

    pub fn result(&mut self, worker: u8) -> WorkerResult {
        let Notice::Finished(result) = self.next() else {
            panic!("expected worker result without another store")
        };
        assert_eq!(result.worker, worker);
        self.wait_worker(worker);
        result
    }

    pub fn wait_worker(&mut self, worker: u8) {
        self.workers
            .iter_mut()
            .find(|(id, _)| *id == worker)
            .expect("known worker")
            .1
            .wait_success(self.deadline);
    }

    pub fn kill_worker(&mut self, worker: u8) {
        self.workers
            .iter_mut()
            .find(|(id, _)| *id == worker)
            .expect("known worker")
            .1
            .kill_and_wait();
        assert!(
            self.server
                .process
                .try_wait()
                .expect("server status")
                .is_none(),
            "credential server survives worker death"
        );
    }

    pub fn snapshot(&mut self) -> Snapshot {
        self.controller.send(Control::Snapshot);
        let Notice::Snapshot(snapshot) = self.next() else {
            panic!("expected durable snapshot")
        };
        snapshot
    }

    pub fn opens(&mut self, result: &WorkerResult) -> bool {
        self.controller.send(Control::Verify {
            ciphertext: result
                .ciphertext
                .as_ref()
                .expect("successful sealing proof")
                .clone(),
        });
        let Notice::Verified(matches) = self.next() else {
            panic!("expected proof verification")
        };
        matches
    }

    pub fn assert_runtime_empty(&self) {
        assert_eq!(
            std::fs::read_dir(&self.runtime.0)
                .expect("test runtime")
                .count(),
            0,
            "construction and native reads must not create coordination state"
        );
    }
}

struct CredentialClient {
    address: SocketAddr,
    worker: u8,
    pause_before_store: bool,
}
impl CredentialClient {
    fn request(&self, request: CredentialRequest) -> Result<CredentialReply, VaultKeyStoreError> {
        initialization_diagnostics::step(match &request {
            CredentialRequest::Load { .. } => "credential_load_rpc",
            CredentialRequest::Store { .. } => "credential_store_rpc",
            CredentialRequest::BeforeStore { .. } => "credential_before_store_rpc",
            CredentialRequest::Finished(_) => "credential_finished_rpc",
        });
        let mut stream = TcpStream::connect_timeout(&self.address, LIFETIME)
            .map_err(|error| Self::rpc_failure("credential_rpc_connect", &error))?;
        configure(&stream)
            .map_err(|error| Self::rpc_failure("credential_rpc_configure", &error))?;
        write_frame(&mut stream, &request)
            .map_err(|error| Self::rpc_failure("credential_rpc_write", &error))?;
        read_frame(&mut BufReader::new(stream))
            .map_err(|error| Self::rpc_failure("credential_rpc_read", &error))
    }

    fn rpc_failure(stage: &'static str, error: &io::Error) -> VaultKeyStoreError {
        initialization_diagnostics::failure(stage, error);
        VaultKeyStoreError::Unavailable("synthetic credential RPC failed".into())
    }
}
impl CredentialBackend for CredentialClient {
    fn load(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
        match self.request(CredentialRequest::Load {
            worker: self.worker,
        })? {
            CredentialReply::Loaded(key) => Ok(key.map(VaultKey::new)),
            _ => Err(VaultKeyStoreError::Unavailable(
                "unexpected synthetic load reply".to_string(),
            )),
        }
    }

    fn store(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError> {
        if self.pause_before_store {
            // coordinator의 intent가 durable해진 뒤, credential STORE RPC를 보내기 직전이다.
            let _ = self.request(CredentialRequest::BeforeStore {
                worker: self.worker,
            })?;
        }
        match self.request(CredentialRequest::Store {
            worker: self.worker,
            key: key.expose().to_vec(),
        })? {
            CredentialReply::Stored => Ok(()),
            _ => Err(VaultKeyStoreError::Unavailable(
                "unexpected synthetic store reply".to_string(),
            )),
        }
    }
}

fn seal_proof(key: &VaultKey, worker: u8) -> Vec<u8> {
    let nonce = [worker; 12];
    let cipher = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, key.expose()).unwrap());
    let mut body = PROOF.to_vec();
    cipher
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut body)
        .expect("synthetic proof sealing");
    let mut sealed = nonce.to_vec();
    sealed.extend(body);
    sealed
}

fn opens_proof(key: Option<&VaultKey>, ciphertext: &[u8]) -> bool {
    let Some(key) = key else { return false };
    let Some((nonce, body)) = ciphertext.split_at_checked(12) else {
        return false;
    };
    let Ok(cipher) = UnboundKey::new(&CHACHA20_POLY1305, key.expose()) else {
        return false;
    };
    let mut body = body.to_vec();
    LessSafeKey::new(cipher)
        .open_in_place(
            Nonce::assume_unique_for_key(nonce.try_into().expect("nonce length")),
            Aad::empty(),
            &mut body,
        )
        .is_ok_and(|plaintext| plaintext == PROOF)
}

pub(super) fn run_worker() {
    let Ok(address) = std::env::var(CREDENTIAL_ENV) else {
        return;
    };
    initialization_diagnostics::start();
    let mode = std::env::var(MODE_ENV).expect("worker mode");
    let client = Arc::new(CredentialClient {
        address: address.parse().expect("synthetic credential address"),
        worker: std::env::var(WORKER_ENV)
            .expect("worker id")
            .parse()
            .expect("worker number"),
        pause_before_store: mode == "pause_before_store",
    });
    let runtime = PathBuf::from(std::env::var_os(RUNTIME_ENV).expect("test runtime"));
    let adapter = Arc::new(adapter_with_store(runtime, client.clone()));
    let service = EvidenceVaultUnlockService::new(adapter.clone());
    let mut result = WorkerResult {
        worker: client.worker,
        key_id: None,
        ciphertext: None,
        failure: None,
        diagnostics: Vec::new(),
    };
    if mode == "read_only" {
        match adapter.load_key() {
            Ok(key) => result.key_id = key.map(|key| record_key(key).key_id),
            Err(_) => result.failure = Some("unexpected read failure".to_string()),
        }
    } else if mode != "construct_only" {
        match service.sealing_key() {
            Ok(key) => {
                result.ciphertext = Some(seal_proof(&key.key, client.worker));
                result.key_id = Some(key.key_id);
            }
            Err(reason) => {
                result.failure = Some(
                    match reason {
                        VaultSharedCopyReason::KeyStoreUnavailable => "KeyStoreUnavailable",
                        _ => "unexpected sealing hold",
                    }
                    .to_string(),
                )
            }
        }
    }
    result.diagnostics = initialization_diagnostics::snapshot();
    assert!(matches!(
        client.request(CredentialRequest::Finished(result)),
        Ok(CredentialReply::Received)
    ));
}

pub(super) fn run_server() {
    let Ok(address) = std::env::var(CONTROL_ENV) else {
        return;
    };
    let gate_count: usize = std::env::var(GATE_ENV)
        .expect("load gate")
        .parse()
        .expect("load count");
    let listener = TcpListener::bind("127.0.0.1:0").expect("credential listener");
    let address: SocketAddr = address.parse().expect("parent address");
    let mut control = TcpStream::connect_timeout(&address, LIFETIME).expect("parent connection");
    configure(&control).expect("control configuration");
    write_frame(
        &mut control,
        &Notice::Ready {
            address: listener.local_addr().expect("credential address"),
        },
    )
    .expect("server ready");

    let (events, received) = mpsc::channel();
    let commands = events.clone();
    let mut input = BufReader::new(control.try_clone().expect("control reader"));
    std::thread::spawn(move || {
        loop {
            let command = read_frame(&mut input).unwrap_or(Control::Stop);
            let stop = matches!(command, Control::Stop);
            if commands.send(ServerEvent::Control(command)).is_err() || stop {
                break;
            }
        }
    });
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let requests = events.clone();
            std::thread::spawn(move || {
                if configure(&stream).is_err() {
                    return;
                }
                let Ok(reader) = stream.try_clone() else {
                    return;
                };
                if let Ok(request) = read_frame(&mut BufReader::new(reader)) {
                    let _ = requests.send(ServerEvent::Credential(stream, request));
                }
            });
        }
    });

    let deadline = Instant::now() + LIFETIME;
    let mut key: Option<VaultKey> = None;
    let mut initial_loads = Vec::new();
    let mut loads_released = gate_count == 0;
    let mut pending_stores = VecDeque::<PendingStore>::new();
    let mut before_store = Vec::<TcpStream>::new();
    let (mut loads, mut store_requests, mut applied_stores) = (0, 0, 0);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "synthetic server deadline");
        let event = received
            .recv_timeout(remaining)
            .expect("synthetic server event");
        match event {
            ServerEvent::Credential(mut stream, CredentialRequest::Load { worker }) => {
                loads += 1;
                if !loads_released {
                    assert!(key.is_none(), "initial load barrier must observe absence");
                    initial_loads.push(stream);
                    write_frame(&mut control, &Notice::LoadWaiting { worker })
                        .expect("load notice");
                } else {
                    let _ = write_frame(
                        &mut stream,
                        &CredentialReply::Loaded(key.as_ref().map(|key| key.expose().to_vec())),
                    );
                }
            }
            ServerEvent::Credential(stream, CredentialRequest::BeforeStore { worker }) => {
                before_store.push(stream);
                write_frame(&mut control, &Notice::BeforeStoreWaiting { worker })
                    .expect("before store notice");
            }
            ServerEvent::Credential(stream, CredentialRequest::Store { worker, key }) => {
                store_requests += 1;
                let key = VaultKey::new(key);
                let key_id = record_key(key.clone()).key_id;
                pending_stores.push_back(PendingStore { stream, key });
                write_frame(&mut control, &Notice::StoreWaiting { worker, key_id })
                    .expect("store notice");
            }
            ServerEvent::Credential(mut stream, CredentialRequest::Finished(result)) => {
                write_frame(&mut control, &Notice::Finished(result)).expect("result notice");
                let _ = write_frame(&mut stream, &CredentialReply::Received);
            }
            ServerEvent::Control(Control::ReleaseInitialLoads) => {
                assert_eq!(
                    initial_loads.len(),
                    gate_count,
                    "all initial None observations precede release"
                );
                loads_released = true;
                for mut stream in initial_loads.drain(..) {
                    let _ = write_frame(&mut stream, &CredentialReply::Loaded(None));
                }
            }
            ServerEvent::Control(Control::CommitNextStore { acknowledge }) => {
                let mut pending = pending_stores
                    .pop_front()
                    .expect("pending credential store");
                key = Some(pending.key);
                applied_stores += 1;
                // 죽은 worker의 stream도 계속 보유한다. 부모의 apply 전에는 credential을 바꾸지 않는다.
                if acknowledge {
                    let _ = write_frame(&mut pending.stream, &CredentialReply::Stored);
                }
            }
            ServerEvent::Control(Control::Snapshot) => {
                write_frame(
                    &mut control,
                    &Notice::Snapshot(Snapshot {
                        key_id: key.as_ref().map(|key| record_key(key.clone()).key_id),
                        loads,
                        store_requests,
                        applied_stores,
                        pending_stores: pending_stores.len(),
                        before_store_waiters: before_store.len(),
                    }),
                )
                .expect("snapshot notice");
            }
            ServerEvent::Control(Control::Verify { ciphertext }) => {
                write_frame(
                    &mut control,
                    &Notice::Verified(opens_proof(key.as_ref(), &ciphertext)),
                )
                .expect("verification notice");
            }
            ServerEvent::Control(Control::Stop) => break,
        }
    }
}
