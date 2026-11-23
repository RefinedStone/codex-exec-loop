//! Delivery reads the ledger without creating executions or changing its evidence revision.
use super::*;
use crate::application::port::outbound::task_delivery_evidence_repository_port::*;
use crate::domain::parallel_mode::ParallelModeQueueItemState;
use crate::domain::planning::delivery_evidence::*;

use super::super::{DISTRIBUTOR_QUEUE_CLAIM_KIND, continuation, runtime_projection, work_orders};

mod binding;
mod commit;
mod proof;
mod read;

pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter) use binding::protect_queue_evidence;

fn permit_stage(stage: TaskEvidenceDeliveryStage) -> PlanningWorkPermitStage {
    match stage {
        TaskEvidenceDeliveryStage::Push => PlanningWorkPermitStage::Push,
        TaskEvidenceDeliveryStage::PullRequest => PlanningWorkPermitStage::PullRequest,
        TaskEvidenceDeliveryStage::Integrate => PlanningWorkPermitStage::Integrate,
    }
}

fn read_queue(
    connection: &Connection,
    queue_id: &str,
) -> Result<Option<PlanningAuthorityDistributorQueueRecord>> {
    let row: Option<(String, String, String)> = connection
        .query_row(
            "SELECT session_key,queue_state,content FROM runtime_distributor_queue WHERE queue_item_id=?1",
            [queue_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(session, state, json)| {
        let queue: PlanningAuthorityDistributorQueueRecord = serde_json::from_str(&json)?;
        if queue.queue_item_id != queue_id
            || queue.session_key != session
            || queue.queue_state.label() != state
            || queue.evidence_observations.len() > 3
        {
            return Err(TaskDeliveryEvidenceError::Unavailable.into());
        }
        if let Some(binding) = &queue.evidence_binding {
            binding.validate()?;
            if binding.scope.task_id != queue.task_id
                || queue.session_key != format!("{}@{}", queue.slot_id, binding.lease_generation)
            {
                return Err(TaskDeliveryEvidenceError::Unavailable.into());
            }
        }
        for (index, observation) in queue.evidence_observations.iter().enumerate() {
            observation.validate()?;
            if queue.evidence_observations[..index]
                .iter()
                .any(|previous| previous.stage == observation.stage)
            {
                return Err(TaskDeliveryEvidenceError::Unavailable.into());
            }
        }
        Ok(queue)
    })
    .transpose()
}

impl TaskDeliveryEvidenceRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn load_delivery_evidence(
        &self,
        workspace: &str,
        expected_queue: &PlanningAuthorityDistributorQueueRecord,
        stage: TaskEvidenceDeliveryStage,
    ) -> Result<StoredTaskDeliveryEvidence> {
        let connection = connection(workspace)?;
        let transaction = connection.unchecked_transaction()?;
        let snapshot = read::snapshot(&transaction, expected_queue, stage)?;
        transaction.commit()?;
        Ok(snapshot)
    }

    fn commit_delivery_evidence_observation(
        &self,
        workspace: &str,
        value: &TaskDeliveryEvidenceObservationCommit,
    ) -> Result<TaskDeliveryEvidenceObservationCommitResult> {
        commit::commit(workspace, value)
    }
}
