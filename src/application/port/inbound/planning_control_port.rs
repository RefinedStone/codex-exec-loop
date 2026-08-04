use anyhow::Result;
use std::sync::Arc;

use crate::domain::planning::PlanningResetTarget;

pub const PLANNING_CONTROL_HELP_TEXT: &str = "지원 명령어\n\
/help\n\
/status\n\
/queue\n\
/reset queue\n\
/reset directions\n\
/reset all";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningControlCommand {
    Help,
    Status,
    Queue,
    Reset(PlanningResetTarget),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlReply {
    pub text: String,
}

impl PlanningControlReply {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlRequest {
    pub command: PlanningControlCommand,
}

impl PlanningControlRequest {
    pub fn new(command: PlanningControlCommand) -> Self {
        Self { command }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlResponse {
    pub workspace_dir: String,
    pub reply: PlanningControlReply,
}

impl PlanningControlResponse {
    pub(crate) fn new(workspace_dir: impl Into<String>, reply: PlanningControlReply) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            reply,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlQueueEntry {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlStatusSnapshot {
    pub workspace_dir: String,
    pub planning_state: String,
    pub task_authority_signature: Option<u64>,
    pub queue_head_task_signature: Option<u64>,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub health: Option<String>,
    pub issue: Option<String>,
    pub note: Option<String>,
    pub preview_status_label: String,
    pub preview_detail: Option<String>,
    pub queue_head: Option<PlanningControlQueueEntry>,
    pub visible_tasks: Vec<PlanningControlQueueEntry>,
    pub proposed_tasks: Vec<PlanningControlQueueEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlResetOutcome {
    pub target: String,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub planning_state: String,
    pub health: Option<String>,
    pub issue: Option<String>,
}

pub trait PlanningControlPort: Send + Sync {
    fn execute_request(&self, request: PlanningControlRequest) -> Result<PlanningControlResponse>;

    fn help_text(&self) -> &'static str {
        PLANNING_CONTROL_HELP_TEXT
    }
}

impl<T> PlanningControlPort for Arc<T>
where
    T: PlanningControlPort + ?Sized,
{
    fn execute_request(&self, request: PlanningControlRequest) -> Result<PlanningControlResponse> {
        self.as_ref().execute_request(request)
    }

    fn help_text(&self) -> &'static str {
        self.as_ref().help_text()
    }
}
