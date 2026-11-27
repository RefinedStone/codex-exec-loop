use akra_verification::service::TaskVerificationCaptureService;

use crate::application::port::outbound::task_verification_repository_port::TaskVerificationBindCommit;
use crate::application::port::outbound::task_verification_runner_port::{
    TaskVerificationPrepareRequest, TaskVerificationRunOutcome,
};
use crate::application::service::parallel_mode::evidence::{
    snapshot_worker_tree, with_running_worker,
};
use crate::domain::planning::evidence::TaskEvidenceArtifact;
use crate::domain::planning::verification::TaskVerificationExecutionManifest;

use super::*;

pub(super) fn execute(
    dependencies: &TaskVerificationDependencies,
    stored: &StoredTaskVerification,
    execution: TaskVerificationExecutionKey,
    cancelled: &AtomicBool,
    closed: &AtomicBool,
    automatic: Option<&automatic::ExecutionGuard>,
    binding: Option<&binding::BindingVerification>,
) -> TaskVerificationFinishCommit {
    match execute_inner(
        dependencies,
        stored,
        &execution,
        cancelled,
        closed,
        automatic,
        binding,
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
    binding: Option<&binding::BindingVerification>,
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
            || binding.is_some_and(|binding| !binding.is_current())
    };
    if check_cancelled() {
        return Err(TaskVerificationNotRunReason::Cancelled);
    }
    // Fence the *claimed* recipe too: a stored automatic request must never
    // start a repository declaration or summon a human prompt.
    if automatic.is_some()
        && stored.attempt.recipe
            != crate::domain::planning::verification::TaskVerificationRecipe::GitDiffCheckV1
    {
        return Err(TaskVerificationNotRunReason::InvalidInput);
    }
    authority::require_current(dependencies, stored)
        .map_err(|error| authority::not_run_reason(&error))?;
    if let Some(binding) = binding {
        binding
            .validate(dependencies, stored)
            .map_err(|_| TaskVerificationNotRunReason::Unavailable)?;
    }
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
            if let Some(binding) = binding {
                binding.validate(dependencies, stored)?;
            }
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
    TaskVerificationCaptureService::new(&dependencies.workspace, dependencies.artifacts.as_ref())
        .capture(attempt_id, manifest, observation, stdout, stderr)
}
