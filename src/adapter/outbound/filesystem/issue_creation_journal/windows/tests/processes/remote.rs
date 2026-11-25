//! Loopback provider state belongs to the parent test, never to a worker.
//! Like the existing delayed publication worker fixture, it can retain an
//! applied mutation while withholding its response across process termination.
use super::*;
use crate::domain::issue_creation::IssueCreatedLocation;
use crate::domain::issue_work::IssueLocator;
use anyhow::ensure;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Condvar;
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;

#[derive(Default)]
struct State {
    copy: Option<IssueSharedCopy>,
    creates: usize,
    updates: usize,
    release_create: bool,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    stopped: AtomicBool,
}
pub(super) struct Server {
    address: String,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl Server {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let shared = Arc::new(Shared::default());
        let state = shared.clone();
        let thread = thread::spawn(move || {
            let mut clients = Vec::new();
            while !state.stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        clients.push(thread::spawn(move || {
                            // The killed creation worker intentionally loses its response.
                            let _ = serve(stream, &state);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("synthetic provider accept failed: {error}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            address,
            shared,
            thread: Some(thread),
        }
    }

    pub(super) fn address(&self) -> &str {
        &self.address
    }

    pub(super) fn wait_for_creation(&self) {
        let state = self.shared.state.lock().unwrap();
        let (state, timeout) = self
            .shared
            .changed
            .wait_timeout_while(state, WAIT, |state| state.creates == 0)
            .unwrap();
        assert!(
            !timeout.timed_out() && state.creates == 1,
            "creation did not reach the provider"
        );
        assert_eq!(state.copy.as_ref(), Some(&copy()));
    }

    pub(super) fn release_creation_response(&self) {
        self.shared.state.lock().unwrap().release_create = true;
        self.shared.changed.notify_all();
    }

    pub(super) fn mutations(&self) -> (usize, usize) {
        let state = self.shared.state.lock().unwrap();
        (state.creates, state.updates)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        self.release_creation_response();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn serve(mut stream: TcpStream, shared: &Shared) -> Result<()> {
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(WAIT))?;
    let mut line = String::new();
    if BufReader::new(stream.try_clone()?).read_line(&mut line)? == 0 {
        return Ok(());
    }
    let request: Value = serde_json::from_str(&line)?;
    let result = match request["operation"].as_str() {
        Some("create") => {
            let copy: IssueSharedCopy = serde_json::from_value(request["copy"].clone())?;
            let mut state = shared.state.lock().unwrap();
            state.copy = Some(copy.clone());
            state.creates += 1;
            shared.changed.notify_all();
            let (state, timeout) = shared
                .changed
                .wait_timeout_while(state, WAIT, |state| !state.release_create)
                .unwrap();
            ensure!(
                !timeout.timed_out(),
                "the test did not release the creation response"
            );
            drop(state);
            json!({ "copy": copy })
        }
        Some("read") => {
            let state = shared.state.lock().unwrap();
            json!({ "copy": state.copy.as_ref().context("not created")? })
        }
        Some("update") => {
            let copy: IssueSharedCopy = serde_json::from_value(request["copy"].clone())?;
            let mut state = shared.state.lock().unwrap();
            ensure!(state.copy.is_some(), "update before creation");
            state.copy = Some(copy.clone());
            state.updates += 1;
            json!({ "copy": copy })
        }
        _ => bail!("unexpected synthetic provider operation"),
    };
    serde_json::to_writer(&mut stream, &result)?;
    stream.write_all(b"\n")?;
    Ok(())
}

pub(super) struct Client {
    address: String,
    journal: IssueCreationJournalAdapter,
    source: IssueSourceRepository,
}
impl Client {
    pub(super) fn new(address: &str, fixture: &Fixture) -> Self {
        Self {
            address: address.into(),
            journal: fixture.adapter(),
            source: fixture.source(),
        }
    }

    fn request(&self, request: Value) -> Result<IssueSharedCopy> {
        let mut stream = TcpStream::connect(&self.address)?;
        stream.set_read_timeout(Some(WAIT))?;
        stream.set_write_timeout(Some(WAIT))?;
        serde_json::to_writer(&mut stream, &request)?;
        stream.write_all(b"\n")?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line)?;
        let response: Value = serde_json::from_str(&line)?;
        Ok(serde_json::from_value(response["copy"].clone())?)
    }

    fn reference(&self, scope: IssueProjectScope) -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_WIDGET".into(),
            issue_id: "I_CREATED".into(),
            project_scope: scope,
        }
    }

    fn snapshot(&self, issue: IssueReference) -> Result<IssueSnapshotResponse> {
        let copy = self.request(json!({ "operation": "read" }))?;
        let raw_fields = IssueRawFields::new(copy.title, copy.body);
        Ok(IssueSnapshotResponse {
            issue,
            comparison_basis: IssueComparisonBasis {
                field_hash: raw_fields.field_hash(),
                provider_updated_at: "2026-11-25T00:00:00Z".into(),
            },
            raw_fields,
        })
    }
}
impl IssueCreatePort for Client {
    fn resolve_repository(
        &self,
        request: IssueRepositoryResolveRequest,
    ) -> Result<IssueRepositoryReference> {
        Ok(IssueRepositoryReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_WIDGET".into(),
            project_scope: request.project_scope,
        })
    }

    fn create_issue(&self, request: IssueCreateRequest) -> Result<IssueCreateResponse> {
        request.ensure_authorized()?;
        ensure!(
            self.journal.load(&self.source)?.unresolved,
            "Sent must be durable before POST"
        );
        let published_copy =
            self.request(json!({ "operation": "create", "copy": request.shared_copy }))?;
        Ok(IssueCreateResponse {
            created: IssueCreatedLocation {
                issue: self.reference(request.repository.project_scope),
                locator: IssueLocator {
                    owner: "acme".into(),
                    repository: "widget".into(),
                    number: 7,
                },
                url: CREATED_URL.into(),
            },
            published_copy,
        })
    }
}
impl IssueSnapshotPort for Client {
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        ensure!(
            request.locator.owner == "acme"
                && request.locator.repository == "widget"
                && request.locator.number == 7,
            "unexpected confirmation locator"
        );
        self.snapshot(self.reference(request.project_scope))
    }
    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        self.snapshot(request.issue)
    }
}
impl IssueUpdatePort for Client {
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        request.ensure_authorized()?;
        let published_copy =
            self.request(json!({ "operation": "update", "copy": request.shared_copy }))?;
        Ok(IssueUpdateResponse {
            issue: request.issue,
            published_copy,
        })
    }
}
