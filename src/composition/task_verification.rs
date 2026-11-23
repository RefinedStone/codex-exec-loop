//! Execution is wired only into a live repository kernel after its OS lock is acquired.
use std::sync::Arc;

use anyhow::Result;

use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::git::task_verification::GitTaskVerificationRunnerAdapter;
use crate::application::service::task_verification::{
    TaskVerificationDependencies, TaskVerificationService, VerificationWake,
};

pub(crate) fn build(
    workspace: &str,
    wake: VerificationWake,
) -> Result<Arc<TaskVerificationService>> {
    let store = Arc::new(SqlitePlanningAuthorityAdapter::new());
    Ok(Arc::new(TaskVerificationService::new(
        TaskVerificationDependencies {
            workspace: workspace.to_owned(),
            repository: store.clone(),
            artifacts: super::task_evidence::artifacts(),
            runtime: Arc::new(GitParallelModeRuntimeAdapter::new()),
            authority: store,
            runner: Arc::new(GitTaskVerificationRunnerAdapter::new()),
            spawn: Arc::new(|work, recover| {
                super::core_effect_worker::try_spawn_worker_with_panic_fallback(work, recover)
                    .map(|_| ())
            }),
            wake,
        },
    )?))
}
