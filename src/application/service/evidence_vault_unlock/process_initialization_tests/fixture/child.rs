//! Only safe structured diagnostics are rendered from a bounded stderr tail.
use super::{
    CONTROL_ENV, CREDENTIAL_ENV, GATE_ENV, MODE_ENV, RUNTIME_ENV, WORKER_ENV,
    initialization_diagnostics,
};
use std::ffi::OsString;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const STDERR_LIMIT: usize = 8 * 1024;

#[derive(Default)]
struct StderrTail {
    bytes_seen: usize,
    bytes: Vec<u8>,
}
impl StderrTail {
    fn push(&mut self, bytes: &[u8]) {
        self.bytes_seen = self.bytes_seen.saturating_add(bytes.len());
        self.bytes.extend_from_slice(bytes);
        if self.bytes.len() > STDERR_LIMIT {
            self.bytes.drain(..self.bytes.len() - STDERR_LIMIT);
        }
    }

    fn summary(&self) -> String {
        let records: Vec<initialization_diagnostics::Diagnostic> =
            String::from_utf8_lossy(&self.bytes)
                .lines()
                .filter_map(|line| line.strip_prefix(initialization_diagnostics::PREFIX))
                .filter_map(|json| serde_json::from_str(json).ok())
                .collect();
        format!(
            "stderr_bytes={} stderr_truncated={} diagnostic_tail={records:?}",
            self.bytes_seen,
            self.bytes_seen > self.bytes.len()
        )
    }
}

pub(super) struct ManagedChild {
    pub(super) process: Child,
    role: &'static str,
    started: Instant,
    stderr: Arc<Mutex<StderrTail>>,
    reader: Option<JoinHandle<()>>,
}
impl ManagedChild {
    pub(super) fn spawn(test: &'static str, environment: &[(&str, OsString)]) -> Self {
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for name in [
            CONTROL_ENV,
            CREDENTIAL_ENV,
            WORKER_ENV,
            RUNTIME_ENV,
            MODE_ENV,
            GATE_ENV,
        ] {
            command.env_remove(name);
        }
        for (name, value) in environment {
            command.env(name, value);
        }
        let mut process = command.spawn().expect("synthetic child process");
        let mut input = process.stderr.take().expect("child stderr pipe");
        let stderr = Arc::new(Mutex::new(StderrTail::default()));
        let output = stderr.clone();
        let reader = std::thread::spawn(move || {
            let mut buffer = [0; 1024];
            loop {
                match input.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => output.lock().unwrap().push(&buffer[..count]),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });
        Self {
            process,
            role: test.rsplit("::").next().unwrap_or("synthetic_child"),
            started: Instant::now(),
            stderr,
            reader: Some(reader),
        }
    }

    pub(super) fn summary(&mut self) -> String {
        let status = match self.process.try_wait() {
            Ok(Some(status)) => format!("{status}"),
            Ok(None) => "running".into(),
            Err(error) => format!("status_error={:?}", error.kind()),
        };
        format!(
            "role={} pid={} elapsed_ms={} status={} {}",
            self.role,
            self.process.id(),
            self.started.elapsed().as_millis(),
            status,
            self.stderr.lock().unwrap().summary(),
        )
    }

    fn join_reader(&mut self) {
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }

    pub(super) fn wait_success(&mut self, deadline: Instant) {
        loop {
            if let Some(status) = self.process.try_wait().expect("child status") {
                self.join_reader();
                assert!(
                    status.success(),
                    "synthetic child failed: {}",
                    self.summary()
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "synthetic child deadline: {}",
                self.summary()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(super) fn kill_and_wait(&mut self) {
        self.process.kill().expect("kill paused synthetic worker");
        let status = self.process.wait().expect("reap paused synthetic worker");
        self.join_reader();
        assert!(!status.success(), "the paused worker must be terminated");
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        self.join_reader();
    }
}

#[test]
fn stderr_capture_is_bounded_and_does_not_render_unstructured_secrets() {
    let mut capture = StderrTail::default();
    capture.push(&vec![b'x'; STDERR_LIMIT * 2]);
    capture.push(b"synthetic-secret-key-and-ciphertext\n");
    capture.push(
        format!(
            "{}{}\n",
            initialization_diagnostics::PREFIX,
            serde_json::json!({
                "stage": "journal_open", "outcome": "failed",
                "sqlite_code": 5, "io_kind": null, "io_code": null,
            })
        )
        .as_bytes(),
    );
    assert_eq!(capture.bytes.len(), STDERR_LIMIT);
    let summary = capture.summary();
    assert!(summary.contains("journal_open"));
    assert!(summary.contains("stderr_truncated=true"));
    assert!(!summary.contains("synthetic-secret"));
}
