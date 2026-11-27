//! Headless AMD64 PE32+ Windows-subsystem validators in a private LPAC.
//! Unsupported creation/token/access/Job/mitigation capabilities fail before ResumeThread.
//! Each run owns a fresh disposable profile; no existing profile or host ACL is repaired.
use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorCancellation, IssueCodeValidatorNotRun as NotRun,
    IssueCodeValidatorRunOutcome as Outcome, IssueCodeValidatorRunRequest,
};

#[path = "windows/handles.rs"]
mod handles;
#[path = "windows/process.rs"]
mod process;
#[path = "windows/profile.rs"]
mod profile;
#[path = "windows/security.rs"]
mod security;
#[cfg(test)]
#[path = "windows/service_tests.rs"]
mod service_tests;
#[path = "windows/storage.rs"]
mod storage;
#[cfg(test)]
#[path = "windows/tests.rs"]
mod tests;

pub(super) fn run(
    request: &IssueCodeValidatorRunRequest,
    cancellation: &IssueCodeValidatorCancellation,
) -> Outcome {
    if cancellation.is_cancelled() {
        return Outcome::NotRun(NotRun::Cancelled);
    }
    if !cfg!(target_arch = "x86_64") {
        return Outcome::NotRun(NotRun::IsolationUnavailable);
    }
    if request.timeout.is_zero() {
        return Outcome::NotRun(NotRun::TimedOut);
    }
    if request.input_json.len() > storage::MAX_INPUT {
        return Outcome::NotRun(NotRun::IsolationUnavailable);
    }
    let content = match storage::approved_content(request) {
        Ok(content) => content,
        Err(reason) => return Outcome::NotRun(reason),
    };
    let outcome = (|| {
        let identity = security::Identity::create()?;
        execute(&identity, &content, request, cancellation)
    })();
    match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            // Do not expose paths, tokens or untrusted process output in user-visible errors.
            #[cfg(test)]
            eprintln!("Windows validator isolation failed: {error}");
            #[cfg(not(test))]
            let _ = error;
            Outcome::NotRun(NotRun::IsolationUnavailable)
        }
    }
}

fn execute(
    identity: &security::Identity,
    content: &[u8],
    request: &IssueCodeValidatorRunRequest,
    cancellation: &IssueCodeValidatorCancellation,
) -> std::io::Result<Outcome> {
    let mut profile = profile::Profile::create(identity)?;
    let execution = (|| {
        let mut workspace = storage::Workspace::create(identity, content, &request.input_json)?;
        let result = process::run(identity, &workspace, request.timeout, cancellation);
        workspace.cleanup()?;
        result
    })();
    profile.cleanup()?;
    execution
}
