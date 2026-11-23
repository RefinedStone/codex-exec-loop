//! One fixed host recipe. No shell, repository script, model turn, or remote command.
//! This is a bounded Git whitespace/conflict-marker check, not a general test runner.
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::application::port::outbound::task_verification_runner_port::{
    PreparedTaskVerification, TaskVerificationPrepareRequest, TaskVerificationRunOutcome,
    TaskVerificationRunnerPort,
};
use crate::domain::planning::verification::{
    MAX_TASK_VERIFICATION_STREAM_BYTES, TaskVerificationExecutionManifest,
    TaskVerificationExecutionOutcome, TaskVerificationNotRunReason as Reason,
    TaskVerificationObservation, TaskVerificationStream,
};
use crate::subprocess::controlled::{ControlledTermination, wait_with_output_controlled};
use crate::trusted_executable::PinnedNativeExecutable;

mod context;
mod environment;
mod input;
#[cfg(test)]
mod tests;

use context::PrivateGitContext;
use environment::{HostEnvironment, digest};

#[derive(Default)]
pub struct GitTaskVerificationRunnerAdapter;

impl GitTaskVerificationRunnerAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl TaskVerificationRunnerPort for GitTaskVerificationRunnerAdapter {
    fn prepare(
        &self,
        request: TaskVerificationPrepareRequest<'_>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn PreparedTaskVerification>, Reason> {
        if cancelled() {
            return Err(Reason::Cancelled);
        }
        let result = prepare(request, cancelled);
        if cancelled() {
            return Err(Reason::Cancelled);
        }
        result.map(|prepared| Box::new(prepared) as Box<dyn PreparedTaskVerification>)
    }

    fn environment_fingerprint(
        &self,
        manifest: &TaskVerificationExecutionManifest,
    ) -> Result<String, Reason> {
        manifest.validate().map_err(|_| Reason::InvalidInput)?;
        let context = std::env::current_dir().map_err(|_| Reason::Unavailable)?;
        let binary = environment::resolve_binary(&context).map_err(|_| Reason::Unavailable)?;
        let current = HostEnvironment::read().map_err(|_| Reason::Unavailable)?;
        // Historical receipt fields must not affect a shared current-environment
        // observation. The query compares each receipt's environment independently.
        current
            .fingerprint(&binary, manifest.source.tree_sha.len())
            .map_err(|_| Reason::Unavailable)
    }
}

struct PreparedVerification {
    context: PrivateGitContext,
    binary: PinnedNativeExecutable,
    manifest: TaskVerificationExecutionManifest,
    deadline: Instant,
}

fn prepare(
    request: TaskVerificationPrepareRequest<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedVerification, Reason> {
    let deadline = Instant::now() + Duration::from_secs(environment::DEADLINE_SECONDS);
    request
        .source
        .validate()
        .map_err(|_| Reason::InvalidInput)?;
    if !environment::valid_oid(request.base_commit_sha)
        || request.base_commit_sha.len() != request.source.tree_sha.len()
        || !Path::new(request.workspace_directory).is_absolute()
    {
        return Err(Reason::InvalidInput);
    }
    let binary = environment::resolve_binary(Path::new(request.workspace_directory))
        .map_err(|_| Reason::Unavailable)?;
    let current = HostEnvironment::read().map_err(|_| Reason::Unavailable)?;
    let mut context = PrivateGitContext::new(
        Path::new(request.workspace_directory),
        request.source.tree_sha.len(),
        &binary,
        current,
        deadline,
        cancelled,
    )
    .map_err(|_| Reason::Unavailable)?;
    if context.source_head != request.source.head_commit_sha {
        return Err(Reason::Stale);
    }
    let base_tree = input::commit_tree(
        &mut context,
        &binary,
        request.base_commit_sha,
        deadline,
        cancelled,
    )?;
    let head_tree = input::commit_tree(
        &mut context,
        &binary,
        &request.source.head_commit_sha,
        deadline,
        cancelled,
    )?;
    if request.source.worktree_clean && head_tree != request.source.tree_sha {
        return Err(Reason::Stale);
    }
    input::inspect_changed_inputs(
        &mut context,
        &binary,
        &base_tree,
        &request.source.tree_sha,
        deadline,
        cancelled,
    )?;
    let manifest = TaskVerificationExecutionManifest {
        recipe: request.recipe,
        base_commit_sha: request.base_commit_sha.into(),
        base_tree_sha: base_tree.clone(),
        source: request.source.clone(),
        git_binary_sha256: binary.sha256().into(),
        command_sha256: environment::command_fingerprint(
            &binary,
            &base_tree,
            &request.source.tree_sha,
        )
        .map_err(|_| Reason::Unavailable)?,
        environment_sha256: context
            .environment_fingerprint(&binary, request.source.tree_sha.len())
            .map_err(|_| Reason::Unavailable)?,
        configuration_sha256: environment::configuration_fingerprint(request.source.tree_sha.len())
            .map_err(|_| Reason::Unavailable)?,
    };
    manifest.validate().map_err(|_| Reason::InvalidInput)?;
    binary.verify().map_err(|_| Reason::ConfigurationChanged)?;
    context.verify().map_err(|_| Reason::ConfigurationChanged)?;
    if Instant::now() >= deadline {
        return Err(Reason::Unavailable);
    }
    Ok(PreparedVerification {
        context,
        binary,
        manifest,
        deadline,
    })
}

impl PreparedTaskVerification for PreparedVerification {
    fn manifest(&self) -> &TaskVerificationExecutionManifest {
        &self.manifest
    }

    fn run(mut self: Box<Self>, cancelled: &dyn Fn() -> bool) -> TaskVerificationRunOutcome {
        if cancelled() {
            return not_run(Reason::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return not_run(Reason::Unavailable);
        }
        if !self.environment_matches()
            || self.context.verify().is_err()
            || self.binary.verify().is_err()
        {
            return not_run(Reason::ConfigurationChanged);
        }
        let Ok(mut command) = self.context.command(&self.binary) else {
            return not_run(Reason::ConfigurationChanged);
        };
        environment::append_diff_arguments(
            &mut command,
            &self.manifest.base_tree_sha,
            &self.manifest.source.tree_sha,
        );
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if cancelled() {
            return not_run(Reason::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return not_run(Reason::Unavailable);
        }
        let started_at = chrono::Utc::now();
        let started = Instant::now();
        let child = match crate::subprocess::spawn(&mut command) {
            Ok(child) => child,
            Err(_) => return not_run(Reason::SpawnFailed),
        };
        let output = wait_with_output_controlled(
            child,
            self.deadline,
            cancelled,
            MAX_TASK_VERIFICATION_STREAM_BYTES as usize,
            MAX_TASK_VERIFICATION_STREAM_BYTES as usize,
        );
        let mut outcome = match output.termination {
            ControlledTermination::Exited(status) => match status.code() {
                Some(exit_code) => TaskVerificationExecutionOutcome::Exited { exit_code },
                None => TaskVerificationExecutionOutcome::Signalled {},
            },
            ControlledTermination::TimedOut => TaskVerificationExecutionOutcome::TimedOut {},
            ControlledTermination::Cancelled => TaskVerificationExecutionOutcome::Cancelled {},
            ControlledTermination::OutputLimit => TaskVerificationExecutionOutcome::OutputLimit {},
            ControlledTermination::IoFailure => TaskVerificationExecutionOutcome::IoFailure {},
        };
        if self.context.verify().is_err()
            || self.binary.verify().is_err()
            || !self.environment_matches()
        {
            outcome = TaskVerificationExecutionOutcome::IoFailure {};
        }
        let finished_at = chrono::Utc::now();
        let observation = TaskVerificationObservation {
            outcome,
            started_at: started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            finished_at: finished_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            stdout: stream(&output.stdout, output.stdout_complete),
            stderr: stream(&output.stderr, output.stderr_complete),
        };
        TaskVerificationRunOutcome::Executed {
            observation: Box::new(observation),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

impl PreparedVerification {
    fn environment_matches(&self) -> bool {
        HostEnvironment::read()
            .and_then(|current| {
                current.fingerprint(&self.binary, self.manifest.source.tree_sha.len())
            })
            .is_ok_and(|value| value == self.manifest.environment_sha256)
    }
}

fn stream(bytes: &[u8], complete: bool) -> TaskVerificationStream {
    TaskVerificationStream {
        sha256: digest(bytes),
        size_bytes: bytes.len() as u64,
        complete,
    }
}

fn not_run(reason: Reason) -> TaskVerificationRunOutcome {
    TaskVerificationRunOutcome::NotRun { reason }
}
