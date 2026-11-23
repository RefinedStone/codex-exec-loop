//! A remote request outlives a killed Windows publisher. New workers only reconcile it.
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::{ApiCalls, World, isolation, public_text, publish_request, run_isolated};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::composition::production::build_issue_publish_journal;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueProjectScope, IssueProvider, IssueReference, IssueSharedCopy,
};

const CASE: &str = "delayed_workers::a_killed_worker_cannot_cancel_or_resend_the_remote_request";
const ROLE: &str = "AKRA_SHARED_COPY_DELAYED_WORKER_ROLE";
const ADDRESS: &str = "AKRA_SHARED_COPY_DELAYED_PROVIDER_ADDRESS";
const EXPECTED: &str = "AKRA_SHARED_COPY_DELAYED_EXPECTED_STATE";

struct Remote(SocketAddr);

impl Remote {
    fn request(&self, request: Value) -> Result<Value> {
        let mut stream = TcpStream::connect_timeout(&self.0, Duration::from_secs(5))?;
        stream.set_read_timeout(Some(Duration::from_secs(40)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        serde_json::to_writer(&mut stream, &request)?;
        stream.write_all(b"\n")?;
        let mut body = String::new();
        BufReader::new(stream).read_line(&mut body)?;
        Ok(serde_json::from_str(&body)?)
    }

    fn snapshot(&self, issue: IssueReference) -> Result<IssueSnapshotResponse> {
        let copy: IssueSharedCopy =
            serde_json::from_value(self.request(json!({"method": "read"}))?)?;
        let fields = IssueRawFields::new(copy.title, copy.body);
        Ok(IssueSnapshotResponse {
            issue,
            comparison_basis: IssueComparisonBasis {
                field_hash: fields.field_hash(),
                provider_updated_at: "2026-11-23T00:00:00Z".to_string(),
            },
            raw_fields: fields,
        })
    }
}

impl IssueSnapshotPort for Remote {
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        self.snapshot(reference(request.project_scope))
    }
    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        self.snapshot(request.issue)
    }
}

impl IssueUpdatePort for Remote {
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        request.ensure_authorized()?;
        let copy: IssueSharedCopy = serde_json::from_value(self.request(json!({
            "method": "write", "copy": request.shared_copy,
        }))?)?;
        Ok(IssueUpdateResponse {
            issue: request.issue,
            published_copy: copy,
        })
    }
}

fn reference(project_scope: IssueProjectScope) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_synthetic".to_string(),
        issue_id: "I_7".to_string(),
        project_scope,
    }
}

struct State {
    copy: IssueSharedCopy,
    pending: Option<IssueSharedCopy>,
    writes: usize,
    applied: bool,
    stopped: bool,
}
type Shared = Arc<(Mutex<State>, Condvar)>;

struct Server {
    address: SocketAddr,
    shared: Shared,
    driver: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let shared = Arc::new((
            Mutex::new(State {
                copy: IssueSharedCopy {
                    title: "Old synthetic title".into(),
                    body: "Old synthetic body".into(),
                },
                pending: None,
                writes: 0,
                applied: false,
                stopped: false,
            }),
            Condvar::new(),
        ));
        let state = shared.clone();
        let driver = thread::spawn(move || {
            let mut requests = Vec::new();
            while !state.0.lock().unwrap().stopped {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        requests.push(thread::spawn(move || {
                            let _ = serve(stream, state);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("synthetic provider accept failed: {error}"),
                }
            }
            for request in requests {
                request.join().unwrap();
            }
        });
        Self {
            address,
            shared,
            driver: Some(driver),
        }
    }

    fn writes(&self) -> usize {
        self.shared.0.lock().unwrap().writes
    }

    fn apply_delayed_request(&self) {
        let mut state = self.shared.0.lock().unwrap();
        assert_eq!(state.writes, 1);
        state.copy = state
            .pending
            .take()
            .expect("the server retained the dead worker's request");
        state.applied = true;
        self.shared.1.notify_all();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.0.lock().unwrap().stopped = true;
        self.shared.1.notify_all();
        if let Some(driver) = self.driver.take() {
            driver.join().unwrap();
        }
    }
}

fn serve(mut stream: TcpStream, shared: Shared) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut line)?;
    let request: Value = serde_json::from_str(&line)?;
    let mut state = shared.0.lock().unwrap();
    if request["method"] == "write" {
        state.writes += 1;
        state.pending = Some(serde_json::from_value(request["copy"].clone())?);
        shared.1.notify_all();
        // Do not couple server execution to the lifetime of the client's TCP connection.
        while !state.applied && !state.stopped {
            state = shared.1.wait(state).unwrap();
        }
        if state.stopped {
            bail!("synthetic server stopped");
        }
    }
    let copy = state.copy.clone();
    drop(state);
    serde_json::to_writer(&mut stream, &copy)?;
    stream.write_all(b"\n")?;
    Ok(())
}

struct Worker {
    child: Child,
    log: std::path::PathBuf,
}

impl Worker {
    fn spawn(server: &Server, role: &str, expected: &str) -> Self {
        let log_path = isolation::root().join(format!("worker-{role}-{expected}.log"));
        let log = File::create(&log_path).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .arg(format!(
                "adapter::inbound::gateway::tests::issue_shared_copy_flow::{CASE}"
            ))
            .args(["--exact", "--nocapture", "--test-threads=1"])
            .env(ROLE, role)
            .env(ADDRESS, server.address.to_string())
            .env(EXPECTED, expected)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        Self {
            child,
            log: log_path,
        }
    }

    fn finish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "worker failed: {}",
                    fs::read_to_string(&self.log).unwrap()
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "worker timed out: {}",
                fs::read_to_string(&self.log).unwrap()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_killed_worker_cannot_cancel_or_resend_the_remote_request() {
    run_isolated(CASE, async {
        let world = World::new();
        if let Ok(role) = std::env::var(ROLE) {
            let remote = Arc::new(Remote(std::env::var(ADDRESS).unwrap().parse().unwrap()));
            let mut session = world
                .start_with_boundaries(remote.clone(), remote, build_issue_publish_journal())
                .await;
            if role == "publish" {
                let request = publish_request(&world, &mut session, public_text(), false).await;
                let result = session.call("akra/issue/publish", request).await;
                panic!(
                    "publisher should be terminated after durable Sent, before its remote answer: {result}"
                );
            }
            let expected = std::env::var(EXPECTED).unwrap();
            let opened = session.call("akra/issue/open", world.target()).await;
            assert_eq!(opened["result"]["intent"]["state"], expected, "{opened}");
            if expected == "needs_confirmation" {
                let request = publish_request(&world, &mut session, public_text(), false).await;
                let repeated = session.call("akra/issue/publish", request).await;
                assert_eq!(
                    repeated["result"]["intent"]["state"], expected,
                    "{repeated}"
                );
            }
            let rechecked = session.call("akra/issue/recheck", world.target()).await;
            assert_eq!(
                rechecked["result"]["intent"]["state"], expected,
                "{rechecked}"
            );
            assert_eq!(world.counts(), ApiCalls::default());
            session.close().await;
            return;
        }

        let server = Server::start();
        let mut first = Worker::spawn(&server, "publish", "sent");
        let deadline = Instant::now() + Duration::from_secs(25);
        while server.writes() == 0 {
            assert!(
                first.child.try_wait().unwrap().is_none(),
                "publisher exited: {}",
                fs::read_to_string(&first.log).unwrap()
            );
            assert!(
                Instant::now() < deadline,
                "publisher never sent: {}",
                fs::read_to_string(&first.log).unwrap()
            );
            thread::sleep(Duration::from_millis(10));
        }
        first.child.kill().unwrap();
        first.child.wait().unwrap();
        assert!(server.shared.0.lock().unwrap().pending.is_some());

        Worker::spawn(&server, "observe", "needs_confirmation").finish();
        assert_eq!(server.writes(), 1);
        server.apply_delayed_request();
        Worker::spawn(&server, "observe", "confirmed").finish();
        assert_eq!(
            server.writes(),
            1,
            "neither replacement worker may resend the request"
        );
    });
}
