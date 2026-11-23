//! Stored evidence composition. Constructing this graph never starts a model or scheduler.
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::adapter::inbound::kernel_protocol::{PlanningState, TaskEvidenceState};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemTaskEvidenceArtifactAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::inbound::task_evidence_query_port::{
    TaskEvidencePlanningSnapshot, TaskEvidenceQueryPort, TaskEvidenceSnapshotQuery,
};
use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;
use crate::application::service::task_evidence::TaskEvidenceService;
use crate::domain::planning::evidence::TaskEvidenceError;

pub(crate) fn artifacts() -> Arc<dyn TaskEvidenceArtifactPort> {
    Arc::new(FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(
        |workspace| {
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)
                .map(|location| PathBuf::from(location.runtime_dir))
        },
    )))
}

pub(crate) fn build(workspace: &str) -> Arc<TaskEvidenceService> {
    let store = Arc::new(SqlitePlanningAuthorityAdapter::new());
    Arc::new(
        TaskEvidenceService::new(
            workspace.to_owned(),
            store.clone(),
            artifacts(),
            Arc::new(GitParallelModeRuntimeAdapter::new()),
            store,
        )
        .with_verification_runner(Arc::new(
            crate::adapter::outbound::git::task_verification::GitTaskVerificationRunnerAdapter::new(
            ),
        )),
    )
}

pub(crate) fn query(
    workspace: &str,
    query: &TaskEvidenceSnapshotQuery,
) -> Result<TaskEvidenceState> {
    build(workspace).planning_snapshot(query).map(Into::into)
}

pub(crate) fn enrich_planning(workspace: &str, planning: PlanningState) -> PlanningState {
    let snapshot = build(workspace).planning_snapshot(&TaskEvidenceSnapshotQuery::default());
    let snapshot = match snapshot {
        Ok(value) if value.planning_revision == planning.revision => value,
        Ok(_) => TaskEvidencePlanningSnapshot::unavailable(TaskEvidenceError::Stale),
        Err(_) => TaskEvidencePlanningSnapshot::unavailable(TaskEvidenceError::Unavailable),
    };
    planning.with_task_evidence(snapshot.into())
}
