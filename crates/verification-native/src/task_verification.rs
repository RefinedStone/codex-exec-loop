//! Pinned Git verification. A repository declaration is data, never shell text.
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use akra_native::subprocess::controlled::{ControlledTermination, wait_with_output_controlled};
use akra_native::trusted_executable::PinnedNativeExecutable;
use akra_verification::ports::task_verification_runner_port::{
    PreparedTaskVerification, TaskVerificationPrepareRequest, TaskVerificationRunOutcome,
    TaskVerificationRunnerPort,
};
use akra_verification::verification::{
    MAX_TASK_VERIFICATION_STREAM_BYTES, TaskVerificationExecutionManifest,
    TaskVerificationExecutionOutcome, TaskVerificationNotRunReason as Reason,
    TaskVerificationObservation, TaskVerificationRecipe, TaskVerificationStream,
};

mod context;
mod declared;
#[cfg(test)]
mod diagnostics;
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
            .recipe_fingerprint(&binary, manifest)
            .map_err(|_| Reason::Unavailable)
    }
}

struct PreparedVerification {
    context: PrivateGitContext,
    binary: PinnedNativeExecutable,
    manifest: TaskVerificationExecutionManifest,
    deadline: Instant,
    declared: Option<declared::Grep>,
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
        .map_err(|error| preparation_unavailable("pin_native_binary", error))?;
    let current = HostEnvironment::read()
        .map_err(|error| preparation_unavailable("read_platform_environment", error))?;
    let mut context = PrivateGitContext::new(
        Path::new(request.workspace_directory),
        request.source.tree_sha.len(),
        &binary,
        current,
        deadline,
        cancelled,
    )
    .map_err(|error| preparation_unavailable("create_private_context", error))?;
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
    let declared = match request.recipe {
        TaskVerificationRecipe::GitDiffCheckV1 => None,
        TaskVerificationRecipe::RepoGitGrepV1 => Some(declared::Grep::read(
            &mut context,
            &binary,
            &request.source.tree_sha,
            deadline,
            cancelled,
        )?),
    };
    let command_sha256 = match &declared {
        Some(grep) => grep.fingerprint(&binary, &request.source.tree_sha)?,
        None => environment::command_fingerprint(&binary, &base_tree, &request.source.tree_sha)
            .map_err(|error| preparation_unavailable("command_fingerprint", error))?,
    };
    let mut manifest = TaskVerificationExecutionManifest {
        recipe: request.recipe,
        base_commit_sha: request.base_commit_sha.into(),
        base_tree_sha: base_tree.clone(),
        source: request.source.clone(),
        git_binary_sha256: binary.sha256().into(),
        command_sha256,
        environment_sha256: context
            .environment_fingerprint(&binary, request.source.tree_sha.len())
            .map_err(|error| preparation_unavailable("environment_fingerprint", error))?,
        configuration_sha256: environment::configuration_fingerprint(request.source.tree_sha.len())
            .map_err(|error| preparation_unavailable("configuration_fingerprint", error))?,
    };
    manifest.environment_sha256 =
        environment::recipe_environment(manifest.recipe, &manifest.environment_sha256);
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
        declared,
    })
}

fn preparation_unavailable(stage: &'static str, error: anyhow::Error) -> Reason {
    #[cfg(test)]
    diagnostics::report(stage, &error);
    #[cfg(not(test))]
    let _ = (stage, error);
    Reason::Unavailable
}

mod execution;
