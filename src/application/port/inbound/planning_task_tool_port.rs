use crate::domain::planning::{OriginSessionKind, PriorityQueueTask, TaskDefinition, TaskStatus};
use serde::{Deserialize, Serialize};

const TASK_TOOL_CONTRACT_JSON: &str = concat!(
    r#"{"tool":"akra planning-tool","version":1,"#,
    r#""commands":["akra planning-tool contract","akra planning-tool run . < request.json"],"#,
    r#""request":{"version":1,"op":"list_tasks|create_task|update_task","apply":"true for create/update","provenance":"application-controlled","fields":"flat"},"#,
    r#""examples":{"list_tasks":{"version":1,"op":"list_tasks","status":["ready","blocked"],"limit":20},"#,
    r#""create_task":{"version":1,"op":"create_task","apply":true,"title":"Review queue handoff","status":"ready","depends_on":[],"blocked_by":[]},"#,
    r#""update_task":{"version":1,"op":"update_task","apply":true,"task_id":"task-123","status":"blocked","priority_reason":"waiting for operator"}},"#,
    r#""rules":["Use before final planning_task_commands.","#,
    r#""Do not edit files, SQL, or JSON authority.","#,
    r#""Run against `.`; in completion prompts do not use payload.worktree_path.","#,
    r#""list_tasks before create/update.","#,
    r#""One narrow task per call; no broad backlog.","#,
    r#""If mutation succeeds, final commands must be empty."],"#,
    r#""create_task_fields":["title required","description optional","direction_id optional","direction_relation_note optional","status optional","base_priority optional","dynamic_priority_delta optional","priority_reason optional","depends_on optional array","blocked_by optional array"],"#,
    r#""update_task_fields":["task_id required","existing descriptions are preserved","other fields optional"],"#,
    r#""response":{"ok":"boolean","error":"string","tasks":"list result","committed_task_ids":"mutation result","queue_head":"after mutation"}}"#
);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanningTaskToolRequest {
    ListTasks(PlanningTaskToolListRequest),
    CreateTask(PlanningTaskToolCreateRequest),
    UpdateTask(PlanningTaskToolUpdateRequest),
}

impl PlanningTaskToolRequest {
    pub(crate) fn apply_cli_host_context(
        &mut self,
        parent_thread_id: Option<String>,
        parent_turn_id: Option<String>,
    ) {
        let parent_thread_id = normalized_host_identifier(parent_thread_id);
        let parent_turn_id = normalized_host_identifier(parent_turn_id);
        match self {
            Self::ListTasks(_) => {}
            Self::CreateTask(request) => {
                request.legacy_source_turn_id = None;
                request.origin_session_kind = Some(OriginSessionKind::Planner);
                request.thread_id = None;
                request.turn_id = None;
                request.parent_thread_id = parent_thread_id;
                request.parent_turn_id = parent_turn_id;
            }
            Self::UpdateTask(request) => {
                request.legacy_source_turn_id = None;
                request.origin_session_kind = Some(OriginSessionKind::Planner);
                request.thread_id = None;
                request.turn_id = None;
                request.parent_thread_id = parent_thread_id;
                request.parent_turn_id = parent_turn_id;
            }
        }
    }
}

fn normalized_host_identifier(identifier: Option<String>) -> Option<String> {
    identifier.and_then(|identifier| {
        let identifier = identifier.trim();
        (!identifier.is_empty()).then(|| identifier.to_string())
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTaskToolListRequest {
    pub version: u32,
    #[serde(default)]
    pub status: Vec<TaskStatus>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTaskToolCreateRequest {
    pub version: u32,
    pub apply: bool,
    #[serde(rename = "source_turn_id")]
    pub legacy_source_turn_id: Option<String>,
    pub origin_session_kind: Option<OriginSessionKind>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub parent_turn_id: Option<String>,
    #[serde(flatten)]
    pub input: PlanningTaskCreatePayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTaskToolUpdateRequest {
    pub version: u32,
    pub apply: bool,
    #[serde(rename = "source_turn_id")]
    pub legacy_source_turn_id: Option<String>,
    pub origin_session_kind: Option<OriginSessionKind>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub parent_turn_id: Option<String>,
    #[serde(flatten)]
    pub input: PlanningTaskUpdatePayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTaskCreatePayload {
    pub direction_id: Option<String>,
    pub direction_relation_note: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTaskUpdatePayload {
    pub task_id: String,
    pub direction_id: Option<String>,
    pub direction_relation_note: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanningTaskToolResponse {
    pub ok: bool,
    pub operation: String,
    pub task_authority_changed: bool,
    pub applied_command_count: usize,
    pub committed_task_ids: Vec<String>,
    pub committed_planning_revision: Option<i64>,
    pub queue_head: Option<PriorityQueueTask>,
    pub tasks: Vec<TaskDefinition>,
    pub guidance: Vec<String>,
}

pub trait PlanningTaskToolPort: Send + Sync {
    fn contract_json(&self) -> &'static str {
        TASK_TOOL_CONTRACT_JSON
    }

    fn run(
        &self,
        workspace_dir: &str,
        request: PlanningTaskToolRequest,
    ) -> anyhow::Result<PlanningTaskToolResponse>;
}

pub fn planning_task_tool_contract_json() -> &'static str {
    TASK_TOOL_CONTRACT_JSON
}
