//! Keep inventory failures distinct without copying paths, config values or credentials into logs.
use std::ffi::OsString;
use std::path::Path;

use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelCommandOutput, ParallelModeRuntimePort,
};
use crate::diagnostics::event_log;

use super::{GitWorktreeRecord, parse_worktree_records};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    ConfigurationRejected,
    ExecutionFailed,
    TimedOut,
    NonzeroExit,
    InvalidUtf8,
    EmptyOutput,
    MalformedOutput,
}

impl FailureKind {
    fn code(self) -> &'static str {
        match self {
            Self::ConfigurationRejected => "configuration_rejected",
            Self::ExecutionFailed => "execution_failed",
            Self::TimedOut => "timed_out",
            Self::NonzeroExit => "nonzero_exit",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::EmptyOutput => "empty_output",
            Self::MalformedOutput => "malformed_output",
        }
    }
}

#[derive(Debug)]
pub(super) struct InventoryFailure {
    kind: FailureKind,
    exit_code: Option<i32>,
    diagnostic: &'static str,
    diagnostic_bytes: usize,
}

impl InventoryFailure {
    fn new(kind: FailureKind, exit_code: Option<i32>, diagnostic: &[u8]) -> Self {
        Self {
            kind,
            exit_code,
            diagnostic: safe_diagnostic_category(diagnostic),
            diagnostic_bytes: diagnostic.len(),
        }
    }

    fn record(&self) {
        event_log::emit_lazy("parallel_worktree_inventory_failed", || {
            serde_json::json!({
                "stage": "worktree_list",
                "failure_kind": self.kind.code(),
                "exit_code": self.exit_code,
                "diagnostic_category": self.diagnostic,
                "diagnostic_bytes": self.diagnostic_bytes,
            })
        });
    }
}

impl std::fmt::Display for InventoryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stage=worktree_list kind={}", self.kind.code())?;
        if let Some(code) = self.exit_code {
            write!(f, " exit_code={code}")?;
        }
        write!(f, " diagnostic={}", self.diagnostic)
    }
}

// Only inspect a bounded prefix and emit fixed categories. Arbitrary stderr may contain
// paths, URLs, config values or tokens; neither it nor the stdout inventory is logged.
fn safe_diagnostic_category(bytes: &[u8]) -> &'static str {
    if bytes.is_empty() {
        return "none";
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    for (needle, category) in [
        ("not a git repository", "not_a_repository"),
        ("dubious ownership", "dubious_ownership"),
        ("permission denied", "permission_denied"),
        ("access is denied", "permission_denied"),
        ("cannot lock", "lock_failed"),
        ("unable to create", "create_failed"),
        ("no such file", "not_found"),
        ("cannot find", "not_found"),
        ("resource temporarily unavailable", "resource_unavailable"),
        ("timed out after", "deadline_exceeded"),
        ("exceeded", "output_or_resource_limit"),
    ] {
        if text.contains(needle) {
            return category;
        }
    }
    "redacted"
}

pub(super) fn load(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> Result<Vec<GitWorktreeRecord>, InventoryFailure> {
    let result = load_checked(runtime, repo_root);
    if let Err(failure) = &result {
        failure.record();
    }
    result
}

fn load_checked(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> Result<Vec<GitWorktreeRecord>, InventoryFailure> {
    // run_git_command preserves exit/stdout/stderr but deliberately leaves this guard to
    // its caller. Keep the same repository execution-config gate as the former run_command.
    runtime
        .ensure_git_execution_safe(Path::new(repo_root))
        .map_err(|error| {
            InventoryFailure::new(FailureKind::ConfigurationRejected, None, error.as_bytes())
        })?;
    let args = ["-C", repo_root, "worktree", "list", "--porcelain"]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    let output = runtime.run_git_command(&args, None).map_err(|error| {
        let kind = if error.contains(" timed out after ") {
            FailureKind::TimedOut
        } else {
            FailureKind::ExecutionFailed
        };
        InventoryFailure::new(kind, None, error.as_bytes())
    })?;
    decode(output)
}

fn decode(output: ParallelCommandOutput) -> Result<Vec<GitWorktreeRecord>, InventoryFailure> {
    let failure = |kind| InventoryFailure::new(kind, output.exit_code, &output.stderr);
    if !output.succeeded() {
        return Err(failure(FailureKind::NonzeroExit));
    }
    let stdout =
        std::str::from_utf8(&output.stdout).map_err(|_| failure(FailureKind::InvalidUtf8))?;
    if stdout.trim().is_empty() {
        return Err(failure(FailureKind::EmptyOutput));
    }
    let records = parse_worktree_records(stdout);
    if records.is_empty()
        || records
            .iter()
            .any(|record| record.path.as_os_str().is_empty())
    {
        return Err(failure(FailureKind::MalformedOutput));
    }
    Ok(records)
}

#[cfg(test)]
mod tests;
