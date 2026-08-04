use std::sync::Arc;

use crate::domain::parallel_mode::ParallelModeOrchestratorState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeOrchestratorTrigger {
    MainTurnCompleted,
    PlanningRefreshCompleted,
    ManualDispatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeOrchestratorTickResult {
    pub trigger: ParallelModeOrchestratorTrigger,
    pub state: ParallelModeOrchestratorState,
    pub blocked: bool,
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelModeControlStatusSnapshot {
    pub readiness_label: String,
    pub reconcile_status: String,
    pub active_agent_count: usize,
    pub queue_depth: usize,
    pub visible_event_count: usize,
}

pub trait ParallelModeControlPort: Send + Sync {
    fn load_status(
        &self,
        workspace_dir: &str,
        recent_event_limit: usize,
    ) -> Result<ParallelModeControlStatusSnapshot, String>;

    fn run_manual_orchestrator_tick(
        &self,
        workspace_dir: &str,
    ) -> Result<ParallelModeOrchestratorTickResult, String>;
}

impl<T> ParallelModeControlPort for Arc<T>
where
    T: ParallelModeControlPort + ?Sized,
{
    fn load_status(
        &self,
        workspace_dir: &str,
        recent_event_limit: usize,
    ) -> Result<ParallelModeControlStatusSnapshot, String> {
        self.as_ref().load_status(workspace_dir, recent_event_limit)
    }

    fn run_manual_orchestrator_tick(
        &self,
        workspace_dir: &str,
    ) -> Result<ParallelModeOrchestratorTickResult, String> {
        self.as_ref().run_manual_orchestrator_tick(workspace_dir)
    }
}
