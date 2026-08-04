use crate::domain::planning::{QueueIdlePolicy, RuntimeWorkspaceStatus, TaskStatus};

/*
 * Shared application projection contracts belong to the inbound boundary, not
 * to a concrete planning service module. Driving adapters may render these
 * immutable values without gaining access to the service graph that produced
 * them.
 */
pub trait PlanningProjectionPort: Send + Sync {
    fn load_application_projection(
        &self,
        workspace_directory: &str,
    ) -> anyhow::Result<PlanningApplicationProjection>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningApplicationProjection {
    pub workspace_present: bool,
    pub workspace_status: RuntimeWorkspaceStatus,
    pub planning_revision: Option<i64>,
    pub task_authority_signature: Option<u64>,
    pub queue_head_task_signature: Option<u64>,
    pub auto_follow_paused: bool,
    pub status_label: String,
    pub status_detail: Option<String>,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub queue_idle_policy: QueueIdlePolicy,
    pub queue_idle_prompt_path: Option<String>,
    pub has_structured_queue_projection: bool,
    pub queue_head: Option<PlanningApplicationQueueTask>,
    pub visible_tasks: Vec<PlanningApplicationQueueTask>,
    pub proposed_tasks: Vec<PlanningApplicationQueueTask>,
    pub skipped_tasks: Vec<PlanningApplicationSkippedTask>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningApplicationQueueTask {
    pub rank: usize,
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub direction_title: String,
    pub status: TaskStatus,
    pub status_label: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub rank_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningApplicationSkippedTask {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: TaskStatus,
    pub status_label: String,
    pub reason: String,
}
