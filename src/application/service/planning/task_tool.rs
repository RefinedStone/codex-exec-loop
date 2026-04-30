use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::domain::planning::{
    PriorityQueueService, PriorityQueueTask, TaskDefinition, TaskStatus,
};

use super::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskMutationCommand, PlanningTaskMutationRequest,
    PlanningTaskMutationService, PlanningTaskMutationSource, PlanningTaskUpdateInput,
};

const TASK_TOOL_CONTRACT_JSON: &str = concat!(
    r#"{"tool":"akra planning-tool","version":1,"#,
    r#""commands":["akra planning-tool contract","akra planning-tool run . < request.json"],"#,
    r#""request":{"version":1,"op":"list_tasks|create_task|update_task","apply":"boolean, required for create_task/update_task","source_turn_id":"optional string","task":"flat create/update fields"},"#,
    r#""rules":["Use this instead of editing planning files, writing SQL, or inventing final-only mutations.","#,
    r#""Run against `.` from the planning worker cwd; in parallel official completion do not use payload.worktree_path as the tool workspace.","#,
    r#""Read state with list_tasks before deciding create vs update.","#,
    r#""For create_task/update_task, set apply=true only after payload is specific and tied to accepted DB authority.","#,
    r#""Use one narrow task per call; avoid broad backlog generation.","#,
    r#""If a mutation succeeds, final planning_task_commands should be empty to avoid double apply."],"#,
    r#""create_task_fields":["title required","description optional","direction_id optional","direction_relation_note optional","status optional: ready|blocked|in_progress|done|cancelled|awaiting_user|proposed","base_priority optional","dynamic_priority_delta optional","priority_reason optional","depends_on optional array","blocked_by optional array"],"#,
    r#""update_task_fields":["task_id required","all other create fields optional"],"#,
    r#""response":{"ok":"boolean","error":"string when failed","tasks":"list_tasks result","committed_task_ids":"mutation result","queue_head":"queue head after mutation"}}"#
);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanningTaskToolRequest {
    ListTasks(PlanningTaskToolListRequest),
    CreateTask(PlanningTaskToolCreateRequest),
    UpdateTask(PlanningTaskToolUpdateRequest),
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
    pub source_turn_id: Option<String>,
    #[serde(flatten)]
    pub input: PlanningTaskCreatePayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTaskToolUpdateRequest {
    pub version: u32,
    pub apply: bool,
    pub source_turn_id: Option<String>,
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

#[derive(Clone)]
pub struct PlanningTaskToolService {
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    task_mutation_service: PlanningTaskMutationService,
}

impl PlanningTaskToolService {
    pub fn new(
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        let task_mutation_service = PlanningTaskMutationService::new(
            planning_task_repository_port.clone(),
            priority_queue_service,
        );
        Self {
            planning_task_repository_port,
            task_mutation_service,
        }
    }

    pub fn handle_request(
        &self,
        workspace_directory: &str,
        request: PlanningTaskToolRequest,
    ) -> Result<PlanningTaskToolResponse> {
        match request {
            PlanningTaskToolRequest::ListTasks(request) => {
                validate_version(request.version)?;
                self.list_tasks(workspace_directory, request)
            }
            PlanningTaskToolRequest::CreateTask(request) => {
                validate_version(request.version)?;
                self.create_task(workspace_directory, request)
            }
            PlanningTaskToolRequest::UpdateTask(request) => {
                validate_version(request.version)?;
                self.update_task(workspace_directory, request)
            }
        }
    }

    fn list_tasks(
        &self,
        workspace_directory: &str,
        request: PlanningTaskToolListRequest,
    ) -> Result<PlanningTaskToolResponse> {
        let snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_directory)?
            .ok_or_else(|| anyhow!("planning task authority is unavailable"))?;
        let mut tasks = snapshot.task_authority.tasks;
        if !request.status.is_empty() {
            tasks.retain(|task| request.status.contains(&task.status));
        }
        tasks.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        if let Some(limit) = request.limit {
            tasks.truncate(limit);
        }
        Ok(PlanningTaskToolResponse {
            ok: true,
            operation: "list_tasks".to_string(),
            task_authority_changed: false,
            applied_command_count: 0,
            committed_task_ids: Vec::new(),
            committed_planning_revision: Some(snapshot.planning_revision),
            queue_head: snapshot.queue_projection.next_task,
            tasks,
            guidance: vec![
                "Use create_task only when no existing task covers the intended work.".to_string(),
                "Use update_task when an existing task should be narrowed, promoted, blocked, or completed."
                    .to_string(),
            ],
        })
    }

    fn create_task(
        &self,
        workspace_directory: &str,
        request: PlanningTaskToolCreateRequest,
    ) -> Result<PlanningTaskToolResponse> {
        if !request.apply {
            return Err(anyhow!(
                "create_task requires apply=true; run list_tasks first if you need context"
            ));
        }
        let mutation = self
            .task_mutation_service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: workspace_directory.to_string(),
                source: PlanningTaskMutationSource::Llm,
                source_turn_id: request.source_turn_id,
                commands: vec![PlanningTaskMutationCommand::CreateTask(
                    request.input.into(),
                )],
            })?;
        Ok(response_from_mutation("create_task", mutation))
    }

    fn update_task(
        &self,
        workspace_directory: &str,
        request: PlanningTaskToolUpdateRequest,
    ) -> Result<PlanningTaskToolResponse> {
        if !request.apply {
            return Err(anyhow!(
                "update_task requires apply=true; run list_tasks first if you need context"
            ));
        }
        let mutation = self
            .task_mutation_service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: workspace_directory.to_string(),
                source: PlanningTaskMutationSource::Llm,
                source_turn_id: request.source_turn_id,
                commands: vec![PlanningTaskMutationCommand::UpdateTask(
                    request.input.into(),
                )],
            })?;
        Ok(response_from_mutation("update_task", mutation))
    }
}

impl From<PlanningTaskCreatePayload> for PlanningTaskCreateInput {
    fn from(input: PlanningTaskCreatePayload) -> Self {
        Self {
            direction_id: input.direction_id,
            direction_relation_note: input.direction_relation_note,
            title: input.title,
            description: input.description,
            status: input.status,
            base_priority: input.base_priority,
            dynamic_priority_delta: input.dynamic_priority_delta,
            priority_reason: input.priority_reason,
            depends_on: input.depends_on,
            blocked_by: input.blocked_by,
        }
    }
}

impl From<PlanningTaskUpdatePayload> for PlanningTaskUpdateInput {
    fn from(input: PlanningTaskUpdatePayload) -> Self {
        Self {
            task_id: input.task_id,
            direction_id: input.direction_id,
            direction_relation_note: input.direction_relation_note,
            title: input.title,
            description: input.description,
            status: input.status,
            base_priority: input.base_priority,
            dynamic_priority_delta: input.dynamic_priority_delta,
            priority_reason: input.priority_reason,
            depends_on: input.depends_on,
            blocked_by: input.blocked_by,
        }
    }
}

pub fn planning_task_tool_contract_json() -> &'static str {
    TASK_TOOL_CONTRACT_JSON
}

fn validate_version(version: u32) -> Result<()> {
    if version == 1 {
        Ok(())
    } else {
        Err(anyhow!(
            "planning-tool request version {version} is not supported"
        ))
    }
}

fn response_from_mutation(
    operation: &str,
    mutation: super::task_mutation::PlanningTaskMutationCommitResult,
) -> PlanningTaskToolResponse {
    PlanningTaskToolResponse {
        ok: true,
        operation: operation.to_string(),
        task_authority_changed: mutation.task_authority_changed,
        applied_command_count: mutation.applied_command_count,
        committed_task_ids: mutation.committed_task_ids,
        committed_planning_revision: Some(mutation.committed_planning_revision),
        queue_head: mutation.queue_head,
        tasks: Vec::new(),
        guidance: vec![
            "Read queue_head and committed_task_ids before deciding on another call.".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::{PlanningTaskToolRequest, planning_task_tool_contract_json};

    #[test]
    fn contract_is_compact_and_names_run_command() {
        let contract = planning_task_tool_contract_json();

        assert!(contract.contains("akra planning-tool run ."));
        assert!(contract.contains("do not use payload.worktree_path"));
        assert!(contract.contains("list_tasks|create_task|update_task"));
        assert!(contract.len() < 1550);
    }

    #[test]
    fn create_task_request_is_flat_for_llm_use() {
        let request = serde_json::from_str::<PlanningTaskToolRequest>(
            r#"{"version":1,"op":"create_task","apply":true,"title":"Review queue idle tool","status":"ready","depends_on":[],"blocked_by":[]}"#,
        )
        .expect("flat create request should parse");

        assert!(matches!(request, PlanningTaskToolRequest::CreateTask(_)));
    }
}
