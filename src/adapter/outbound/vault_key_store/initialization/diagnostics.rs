//! Test-only, bounded diagnostics. Never retain paths, SQL, credentials or error messages.
use std::cell::RefCell;
use std::error::Error;

use serde::{Deserialize, Serialize};

pub(crate) const PREFIX: &str = "AKRA_VAULT_INITIALIZATION_DIAGNOSTIC ";
const LIMIT: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Diagnostic {
    pub(crate) stage: String,
    pub(crate) outcome: String,
    pub(crate) sqlite_code: Option<i32>,
    pub(crate) io_kind: Option<String>,
    pub(crate) io_code: Option<i32>,
}

thread_local! {
    static CAPTURE: RefCell<Option<Vec<Diagnostic>>> = const { RefCell::new(None) };
}

pub(crate) fn start() {
    CAPTURE.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
}

pub(crate) fn snapshot() -> Vec<Diagnostic> {
    CAPTURE.with(|capture| capture.borrow().clone().unwrap_or_default())
}

pub(crate) fn step(stage: &'static str) {
    record(Diagnostic {
        stage: stage.into(),
        outcome: "entered".into(),
        sqlite_code: None,
        io_kind: None,
        io_code: None,
    });
}

pub(crate) fn failure(stage: &'static str, error: &(dyn Error + 'static)) {
    let mut diagnostic = Diagnostic {
        stage: stage.into(),
        outcome: "failed".into(),
        sqlite_code: None,
        io_kind: None,
        io_code: None,
    };
    let mut source = Some(error);
    while let Some(error) = source {
        if let Some(rusqlite::Error::SqliteFailure(code, _)) =
            error.downcast_ref::<rusqlite::Error>()
        {
            diagnostic.sqlite_code = Some(code.extended_code);
        }
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            diagnostic.io_kind = Some(format!("{:?}", error.kind()));
            diagnostic.io_code = error.raw_os_error();
        }
        source = error.source();
    }
    record(diagnostic);
}

fn record(diagnostic: Diagnostic) {
    let enabled = CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        let Some(records) = capture.as_mut() else {
            return false;
        };
        if records.len() == LIMIT {
            records.remove(0);
        }
        records.push(diagnostic.clone());
        true
    });
    if enabled {
        // Only the explicitly opted-in synthetic worker emits this allowlisted DTO.
        eprintln!(
            "{PREFIX}{}",
            serde_json::to_string(&diagnostic).expect("safe diagnostic")
        );
    }
}

#[test]
fn error_codes_survive_context_without_retaining_secret_error_text() {
    start();
    let error = anyhow::Error::new(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
        Some("synthetic-secret-key-and-sql".into()),
    ))
    .context("synthetic-secret-path");
    failure("journal_schema_begin", error.as_ref());
    let records = snapshot();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sqlite_code, Some(rusqlite::ffi::SQLITE_BUSY));
    let output = serde_json::to_string(&records).unwrap();
    assert!(output.contains("journal_schema_begin"));
    assert!(!output.contains("synthetic-secret"));
}

#[test]
fn capture_keeps_a_bounded_tail_and_native_io_code() {
    start();
    for _ in 0..LIMIT + 4 {
        step("journal_snapshot");
    }
    failure(
        "credential_rpc_connect",
        &std::io::Error::from_raw_os_error(13),
    );
    let records = snapshot();
    assert_eq!(records.len(), LIMIT);
    assert_eq!(records.last().unwrap().io_code, Some(13));
    assert_eq!(records.last().unwrap().outcome, "failed");
}
