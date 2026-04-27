use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkerOperation {
    RefreshQueue,
    RepairTaskLedger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkerRequest {
    pub operation: PlanningWorkerOperation,
    pub workspace_directory: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkerResponse {
    pub operation: PlanningWorkerOperation,
    pub final_agent_message: Option<String>,
    pub changed_planning_file_paths: Vec<String>,
}

pub trait PlanningWorkerPort: Send + Sync {
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse>;
}

pub struct NoopPlanningWorkerPort;

impl PlanningWorkerPort for NoopPlanningWorkerPort {
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse> {
        Ok(PlanningWorkerResponse {
            operation: request.operation,
            final_agent_message: Some("planner worker disabled".to_string()),
            changed_planning_file_paths: Vec::new(),
        })
    }
}
