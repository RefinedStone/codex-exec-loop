use base64::Engine;
use sha2::{Digest, Sha256};

use crate::application::port::outbound::task_verification_repository_port::TaskVerificationBindCommit;
use crate::application::port::outbound::task_verification_runner_port::{
    TaskVerificationPrepareRequest, TaskVerificationRunOutcome,
};
use crate::application::service::parallel_mode::evidence::{
    snapshot_worker_tree, with_running_worker,
};
use crate::domain::planning::evidence::{MAX_TASK_EVIDENCE_ARTIFACT_BYTES, TaskEvidenceArtifact};
use crate::domain::planning::verification::{
    MAX_TASK_VERIFICATION_STREAM_BYTES, TaskVerificationExecutionManifest, TaskVerificationStream,
};

use super::*;

pub(super) fn execute(
    dependencies: &TaskVerificationDependencies,
    stored: &StoredTaskVerification,
    execution: TaskVerificationExecutionKey,
    cancelled: &AtomicBool,
    closed: &AtomicBool,
    automatic: Option<&automatic::ExecutionGuard>,
) -> TaskVerificationFinishCommit {
    match execute_inner(
        dependencies,
        stored,
        &execution,
        cancelled,
        closed,
        automatic,
    ) {
        Ok((result, artifact)) => TaskVerificationFinishCommit {
            execution,
            result,
            artifact,
            at: utc_now(),
        },
        Err(reason) => not_run(execution, reason),
    }
}

fn execute_inner(
    dependencies: &TaskVerificationDependencies,
    stored: &StoredTaskVerification,
    execution: &TaskVerificationExecutionKey,
    cancelled: &AtomicBool,
    closed: &AtomicBool,
    automatic: Option<&automatic::ExecutionGuard>,
) -> std::result::Result<
    (TaskVerificationResult, Option<TaskEvidenceArtifact>),
    TaskVerificationNotRunReason,
> {
    let check_cancelled = || {
        cancelled.load(Ordering::Acquire)
            || closed.load(Ordering::Acquire)
            || automatic.is_some_and(|guard| {
                !guard.continuation.is_current() || Instant::now() >= guard.deadline
            })
    };
    if check_cancelled() {
        return Err(TaskVerificationNotRunReason::Cancelled);
    }
    authority::require_current(dependencies, stored)
        .map_err(|error| authority::not_run_reason(&error))?;
    let source = with_running_worker(
        dependencies.authority.as_ref(),
        dependencies.runtime.as_ref(),
        &dependencies.workspace,
        &stored.lease,
        || {
            authority::require_current(dependencies, stored)?;
            snapshot_worker_tree(dependencies.runtime.as_ref(), &stored.lease)
        },
    )
    .map_err(|error| authority::not_run_reason(&error))?;
    if automatic.is_some_and(|guard| guard.source != source) {
        return Err(TaskVerificationNotRunReason::Stale);
    }
    let base = stored
        .lease
        .delivery_target
        .as_ref()
        .ok_or(TaskVerificationNotRunReason::LeaseChanged)?;
    // Preparing objects/configuration and waiting for Git never retain the pool mutation guard.
    let prepared = dependencies.runner.prepare(
        TaskVerificationPrepareRequest {
            workspace_directory: &stored.lease.worktree_path,
            recipe: stored.attempt.recipe,
            base_commit_sha: &base.integration_base_commit_sha,
            source: &source,
        },
        &check_cancelled,
    )?;
    let manifest = prepared.manifest().clone();
    manifest
        .validate()
        .map_err(|_| TaskVerificationNotRunReason::Unavailable)?;
    if manifest.recipe != stored.attempt.recipe
        || manifest.source != source
        || manifest.base_commit_sha != base.integration_base_commit_sha
        || dependencies
            .runtime
            .read_commit_tree(
                &stored.lease.worktree_path,
                &base.integration_base_commit_sha,
            )
            .map_err(|_| TaskVerificationNotRunReason::Unavailable)?
            != manifest.base_tree_sha
    {
        return Err(TaskVerificationNotRunReason::Stale);
    }
    if check_cancelled() {
        return Err(TaskVerificationNotRunReason::Cancelled);
    }
    with_running_worker(
        dependencies.authority.as_ref(),
        dependencies.runtime.as_ref(),
        &dependencies.workspace,
        &stored.lease,
        || {
            authority::require_current(dependencies, stored)?;
            if snapshot_worker_tree(dependencies.runtime.as_ref(), &stored.lease)? != source {
                return Err(TaskEvidenceError::Stale.into());
            }
            if !dependencies.repository.bind_host_verification(
                &dependencies.workspace,
                &TaskVerificationBindCommit {
                    execution: execution.clone(),
                    manifest: manifest.clone(),
                    at: utc_now(),
                },
            )? {
                return Err(TaskEvidenceError::Conflict.into());
            }
            Ok(())
        },
    )
    .map_err(|error| authority::not_run_reason(&error))?;
    if check_cancelled() {
        return Err(TaskVerificationNotRunReason::Cancelled);
    }
    match prepared.run(&check_cancelled) {
        TaskVerificationRunOutcome::NotRun { reason } => Err(reason),
        TaskVerificationRunOutcome::Executed {
            observation,
            stdout,
            stderr,
        } => Ok(capture_executed(
            dependencies,
            &stored.attempt.attempt_id,
            &manifest,
            observation,
            &stdout,
            &stderr,
        )),
    }
}

pub(super) fn capture_executed(
    dependencies: &TaskVerificationDependencies,
    attempt_id: &str,
    manifest: &TaskVerificationExecutionManifest,
    observation: Box<crate::domain::planning::verification::TaskVerificationObservation>,
    stdout: &[u8],
    stderr: &[u8],
) -> (TaskVerificationResult, Option<TaskEvidenceArtifact>) {
    if observation.validate().is_err() {
        // The process ran, but there is no valid observation to retain. Never turn
        // this uncertainty into either NotRun or a successful execution.
        return (TaskVerificationResult::Interrupted {}, None);
    }
    let artifact = if stream_matches(&observation.stdout, stdout)
        && stream_matches(&observation.stderr, stderr)
    {
        seal_capture(
            dependencies,
            attempt_id,
            manifest,
            &observation,
            stdout,
            stderr,
        )
        .ok()
    } else {
        None
    };
    // A failed capture retains the execution fact, without proof of a current pass.
    (TaskVerificationResult::Executed { observation }, artifact)
}

fn stream_matches(expected: &TaskVerificationStream, actual: &[u8]) -> bool {
    actual.len() as u64 <= MAX_TASK_VERIFICATION_STREAM_BYTES
        && expected.size_bytes == actual.len() as u64
        && expected.sha256 == format!("{:x}", Sha256::digest(actual))
}

fn seal_capture(
    dependencies: &TaskVerificationDependencies,
    attempt_id: &str,
    manifest: &TaskVerificationExecutionManifest,
    observation: &crate::domain::planning::verification::TaskVerificationObservation,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<TaskEvidenceArtifact> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "attempt_id": attempt_id,
        "manifest": manifest,
        "observation": observation,
        "stdout_base64": base64::engine::general_purpose::STANDARD.encode(stdout),
        "stderr_base64": base64::engine::general_purpose::STANDARD.encode(stderr),
    }))?;
    if bytes.len() as u64 > MAX_TASK_EVIDENCE_ARTIFACT_BYTES {
        return Err(TaskEvidenceError::Limit.into());
    }
    let artifact =
        dependencies
            .artifacts
            .seal_host_capture(&dependencies.workspace, attempt_id, &bytes)?;
    artifact.validate()?;
    if artifact.size_bytes != bytes.len() as u64
        || artifact.sha256 != format!("{:x}", Sha256::digest(&bytes))
    {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(artifact)
}
