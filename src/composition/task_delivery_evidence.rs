//! Fixed effective policy and real evidence readers for one canonical repository kernel.
use std::sync::Arc;

use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::git::task_verification::GitTaskVerificationRunnerAdapter;
use crate::application::port::inbound::task_delivery_evidence_port::TaskDeliveryEvidenceGatePort;
use crate::application::service::task_delivery_evidence::{
    TaskDeliveryEvidenceDependencies, TaskDeliveryEvidenceService,
};
use crate::domain::planning::delivery_evidence::TaskEvidenceDeliveryMode;

pub(crate) fn build(
    workspace: &str,
    repository_identity: &str,
    mode: Result<TaskEvidenceDeliveryMode, String>,
) -> Arc<dyn TaskDeliveryEvidenceGatePort> {
    let store = Arc::new(SqlitePlanningAuthorityAdapter::new());
    Arc::new(TaskDeliveryEvidenceService::new(
        TaskDeliveryEvidenceDependencies {
            workspace: workspace.to_owned(),
            repository_identity: repository_identity.to_owned(),
            repository: store.clone(),
            artifacts: super::task_evidence::artifacts(),
            runtime: Arc::new(GitParallelModeRuntimeAdapter::new()),
            authority: store,
            runner: Arc::new(GitTaskVerificationRunnerAdapter::new()),
        },
        mode,
    ))
}
