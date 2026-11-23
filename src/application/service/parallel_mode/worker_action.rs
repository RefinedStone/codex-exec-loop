use super::turn::ParallelModeTurnService;
use crate::application::port::agent_action::AgentActionGuard;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerStartAuthorization;
use crate::application::port::outbound::planning_authority_port::PlanningWorkerContinuationClaim;
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;
use anyhow::{Context, Result, bail};

pub(super) struct WorkerStartAuthorization {
    pub turn: ParallelModeTurnService,
    pub workspace: String,
    pub epoch: u64,
    pub lease: ParallelModeSlotLeaseSnapshot,
    pub continuation: Option<Box<PlanningWorkerContinuationClaim>>,
}

impl ParallelAgentWorkerStartAuthorization for WorkerStartAuthorization {
    fn validate(&self) -> Result<()> {
        if !self
            .turn
            .automation_epoch_is_active(&self.workspace, self.epoch)
        {
            bail!("parallel automation closed before worker turn/start");
        }
        self.turn
            .validate_captured_worker_action(&self.workspace, &self.lease.task_id)?;
        if let Some(claim) = self.continuation.as_deref() {
            self.turn
                .validate_worker_continuation_start(&self.workspace, claim)
                .map_err(anyhow::Error::msg)?;
        } else {
            self.turn
                .validate_claimed_worker_launch(&self.workspace, &self.lease)?;
        }
        if !self
            .turn
            .automation_epoch_is_active(&self.workspace, self.epoch)
        {
            bail!("parallel automation closed before worker turn/start");
        }
        Ok(())
    }

    fn acquire(&self) -> Result<Box<dyn AgentActionGuard>> {
        let action = self
            .turn
            .captured_action
            .as_ref()
            .context("worker action was not captured")?;
        let guard = action.acquire()?;
        self.validate()?;
        guard.validate()?;
        Ok(guard)
    }
}
