use super::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskMutationCommand, PlanningTaskMutationRequest,
    PlanningTaskMutationService, PlanningTaskMutationSource, PlanningTaskUpdateInput,
};
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::domain::planning::{
    PriorityQueueService, PriorityQueueTask, TaskDefinition, TaskStatus,
};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/*
 * planning task tool은 LLM-facing task authority write API다. full planning admin surface보다
 * 의도적으로 좁게 설계되어 worker는 task를 읽고, `PlanningTaskMutationService`를 통해 한 번에
 * 하나의 create/update만 적용할 수 있다. 파일 편집, SQL rewrite, 광범위한 backlog batch는 이
 * tool boundary 밖에 둬서 model output이 accepted DB authority를 우회하지 못하게 한다.
 */
const TASK_TOOL_CONTRACT_JSON: &str = concat!(
    r#"{"tool":"akra planning-tool","version":1,"#,
    r#""commands":["akra planning-tool contract","akra planning-tool run . < request.json"],"#,
    r#""request":{"version":1,"op":"list_tasks|create_task|update_task","apply":"boolean, required for create_task/update_task","source_turn_id":"optional string","task":"flat create/update fields"},"#,
    r#""rules":["Use this tool first; final planning_task_commands is fallback only.","#,
    r#""Use instead of editing planning files, SQL, or final-only mutations.","#,
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
    // tagged enum은 JSON `op` field와 직접 대응한다. 잘못된 operation 이름은 mutation code에
    // 닿기 전에 serde 단계에서 실패해 tool command surface를 작게 유지한다.
    ListTasks(PlanningTaskToolListRequest),
    CreateTask(PlanningTaskToolCreateRequest),
    UpdateTask(PlanningTaskToolUpdateRequest),
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTaskToolListRequest {
    pub version: u32,
    // 빈 status는 "모든 task 표시"다. 명시 status는 model이 자체 filter 언어를 만들지 않고
    // ready/proposed/blocked 같은 authority 상태로만 목록을 좁히게 한다.
    #[serde(default)]
    pub status: Vec<TaskStatus>,
    pub limit: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTaskToolCreateRequest {
    pub version: u32,
    // mutation에는 명시적인 apply flag가 필요하다. prompt가 authority 변경을 허용하기 전
    // dry planning/list 단계로 model을 유도할 수 있게 하는 안전장치다.
    pub apply: bool,
    pub source_turn_id: Option<String>,
    // flatten은 model이 생성할 JSON을 단순하게 만든다. nested task object 대신
    // {"op":"create_task","title":"..."} 형태를 유지해 prompt 예시와 실제 schema가 가까워진다.
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
    // payload는 JSON ergonomics를 제외하면 `PlanningTaskCreateInput`과 의도적으로 같은 의미를
    // 갖는다. 변환을 기계적으로 유지하고, default/validation은 mutation service 한 곳에 남긴다.
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
    // update field는 optional patch다. 누락된 값은 현재 task를 보존하고,
    // Some(Vec::new())은 dependency/blocker 목록을 명시적으로 비우는 의미를 갖는다.
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
    // mutation metadata를 직접 노출해 worker가 성공한 tool call 뒤 final planning_task_commands를
    // 다시 적용하는 double-apply를 피하게 한다.
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
    // write는 app runtime이 쓰는 같은 mutation service를 통과한다. tool 전용 writer를 만들지
    // 않아 revision compare-and-swap, validation, audit attribution 경로가 하나로 유지된다.
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
        // version check는 dispatch 전에 실행한다. list/create/update가 같은 compatibility gate를
        // 공유해야 command schema 변경 시 operation별 drift가 생기지 않는다.
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
        // list_tasks는 persisted authority state와 최신 queue projection을 읽지만 mutation service를
        // 호출하거나 revision을 올리지 않는다. model이 먼저 현재 상태를 확인하는 read-only 단계다.
        if !request.status.is_empty() {
            tasks.retain(|task| request.status.contains(&task.status));
        }
        // newest update를 먼저 보여 주면 worker가 의도한 follow-up이 최근 task에 이미 반영됐는지
        // 판단하기 쉽다. 동률은 id로 정렬해 응답 순서를 안정화한다.
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
            // prompt는 worker에게 먼저 list_tasks를 호출하라고 요구한다. apply=false create를
            // 조용히 preview로 처리하면 commit될 수 없는 중간 상태가 생기므로 명시적으로 거부한다.
            return Err(anyhow!(
                "create_task requires apply=true; run list_tasks first if you need context"
            ));
        }
        // 이 API는 planning worker tool이 호출하므로 source는 항상 Llm이다.
        // source_turn_id는 실제 model turn과 mutation audit record를 이어 주는 provenance다.
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
            // update도 authority write이므로 create와 같은 explicit-apply guard를 적용한다.
            // partial patch라고 해서 model의 dry-run 의도를 실제 commit으로 해석하지 않는다.
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
        // 변환은 lossless여야 한다. status/priority 같은 default는 `PlanningTaskMutationService`가
        // 소유해야 tool call과 fallback command가 같은 authority 결과를 만든다.
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
        // Option<Vec<_>>를 그대로 보존해 mutation service가 field omission과 명시적 empty list를
        // 구분하게 한다. 이 차이가 dependency/blocker clear semantics를 만든다.
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
    // contract는 prompt와 CLI output에 삽입된다. runtime에 Rust type에서 재생성하지 않고
    // compact/stable 문자열로 고정해 model-facing schema drift를 리뷰 가능한 diff로 남긴다.
    TASK_TOOL_CONTRACT_JSON
}
fn validate_version(version: u32) -> Result<()> {
    // tool JSON version은 planning authority document version과 독립이다. command schema가 바뀔 때
    // request compatibility를 authority migration과 섞지 않고 명시적으로 차단한다.
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
    // mutation response는 의도적으로 task list를 비워 둔다. full post-commit view가 필요하면
    // caller가 fresh list_tasks를 호출해야 하며, 그 과정에서 새 revision/queue projection을 다시 관찰한다.
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
        // contract는 prompt에 들어갈 만큼 작아야 하지만, worker가 file edit나 payload.worktree_path
        // 오용으로 빠지지 않게 충분한 guardrail도 포함해야 한다.
        let contract = planning_task_tool_contract_json();

        assert!(contract.contains("akra planning-tool run ."));
        assert!(contract.contains("do not use payload.worktree_path"));
        assert!(contract.contains("list_tasks|create_task|update_task"));
        assert!(contract.len() < 1550);
    }
    #[test]
    fn create_task_request_is_flat_for_llm_use() {
        // flat JSON은 model-facing ergonomics의 일부다. nested task object를 요구하기 시작하면
        // prompt 예시와 실제 요청 작성 난도가 같이 올라간다.
        let request = serde_json::from_str::<PlanningTaskToolRequest>(
            r#"{"version":1,"op":"create_task","apply":true,"title":"Review queue idle tool","status":"ready","depends_on":[],"blocked_by":[]}"#,
        )
        .expect("flat create request should parse");

        assert!(matches!(request, PlanningTaskToolRequest::CreateTask(_)));
    }
}
