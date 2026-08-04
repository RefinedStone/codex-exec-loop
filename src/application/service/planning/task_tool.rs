use super::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskMutationCommand, PlanningTaskMutationRequest,
    PlanningTaskMutationService, PlanningTaskMutationSource, PlanningTaskUpdateInput,
};
pub use crate::application::port::inbound::planning_task_tool_port::{
    PlanningTaskCreatePayload, PlanningTaskToolCreateRequest, PlanningTaskToolListRequest,
    PlanningTaskToolRequest, PlanningTaskToolResponse, PlanningTaskToolUpdateRequest,
    PlanningTaskUpdatePayload, planning_task_tool_contract_json,
};
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::domain::planning::{OriginSessionKind, PriorityQueueService, TaskMutationProvenance};
use anyhow::{Result, anyhow};
use std::sync::Arc;

/*
 * planning task tool은 worker-facing task authority write API다. full planning admin surface보다
 * 의도적으로 좁게 설계되어 worker는 task를 읽고, `PlanningTaskMutationService`를 통해 한 번에
 * 하나의 create/update만 적용할 수 있다. 파일 편집, SQL rewrite, 광범위한 backlog batch는 이
 * tool boundary 밖에 둬서 model output이 accepted DB authority를 우회하지 못하게 한다.
 */
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
        // 이 API는 planning worker tool이 호출하므로 source는 항상 Worker이다.
        // generic provenance가 없으면 worker/tool 경계라는 origin만 남긴다.
        let provenance = task_tool_provenance(
            request.origin_session_kind,
            request.thread_id,
            request.turn_id,
            request.parent_thread_id,
            request.parent_turn_id,
        );
        let mutation = self
            .task_mutation_service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: workspace_directory.to_string(),
                source: PlanningTaskMutationSource::Worker,
                legacy_source_turn_id: request.legacy_source_turn_id,
                provenance,
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
        let provenance = task_tool_provenance(
            request.origin_session_kind,
            request.thread_id,
            request.turn_id,
            request.parent_thread_id,
            request.parent_turn_id,
        );
        let mutation = self
            .task_mutation_service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: workspace_directory.to_string(),
                source: PlanningTaskMutationSource::Worker,
                legacy_source_turn_id: request.legacy_source_turn_id,
                provenance,
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
fn task_tool_provenance(
    origin_session_kind: Option<OriginSessionKind>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    parent_thread_id: Option<String>,
    parent_turn_id: Option<String>,
) -> TaskMutationProvenance {
    TaskMutationProvenance::new(origin_session_kind.unwrap_or(OriginSessionKind::Planner))
        .with_thread_turn(thread_id, turn_id)
        .with_parent(parent_thread_id, parent_turn_id)
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
    use super::{
        PlanningTaskCreatePayload, PlanningTaskToolCreateRequest, PlanningTaskToolListRequest,
        PlanningTaskToolRequest, PlanningTaskToolService, PlanningTaskToolUpdateRequest,
        PlanningTaskUpdatePayload, planning_task_tool_contract_json,
    };
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
        PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
    };
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
        PLANNING_FORMAT_VERSION, PriorityQueueProjection, PriorityQueueService, PriorityQueueTask,
        QueueIdleConfig, TaskActor, TaskAuthorityDocument, TaskDefinition, TaskMutationProvenance,
        TaskStatus,
    };
    use std::sync::Arc;

    fn workspace(label: &str) -> String {
        format!(
            "/tmp/akra-planning-task-tool-test-{label}-{}",
            std::process::id()
        )
    }

    fn repo() -> Arc<NoopPlanningTaskRepositoryPort> {
        Arc::new(NoopPlanningTaskRepositoryPort)
    }

    fn service(repo: Arc<NoopPlanningTaskRepositoryPort>) -> PlanningTaskToolService {
        PlanningTaskToolService::new(repo, PriorityQueueService::new())
    }

    fn directions() -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![
                DirectionDefinition {
                    id: "general-workstream".to_string(),
                    title: "General".to_string(),
                    summary: "General planning work.".to_string(),
                    success_criteria: vec!["done".to_string()],
                    scope_hints: Vec::new(),
                    detail_doc_path: String::new(),
                    state: DirectionState::Active,
                },
                DirectionDefinition {
                    id: "support-workstream".to_string(),
                    title: "Support".to_string(),
                    summary: "Supporting planning work.".to_string(),
                    success_criteria: vec!["supported".to_string()],
                    scope_hints: Vec::new(),
                    detail_doc_path: String::new(),
                    state: DirectionState::Active,
                },
            ],
        }
    }

    fn provenance() -> TaskMutationProvenance {
        TaskMutationProvenance::new(OriginSessionKind::Planner)
    }

    fn task(id: &str, title: &str, status: TaskStatus, updated_at: &str) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: "general-workstream".to_string(),
            direction_relation_note: "supports direction".to_string(),
            title: title.to_string(),
            description: title.to_string(),
            status,
            base_priority: 50,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: provenance(),
            updated_at: updated_at.to_string(),
        }
    }

    fn queue_task(task: &TaskDefinition, rank: usize) -> PriorityQueueTask {
        PriorityQueueTask {
            rank,
            task_id: task.id.clone(),
            direction_id: task.direction_id.clone(),
            direction_title: "General".to_string(),
            task_title: task.title.clone(),
            status: task.status,
            combined_priority: task.base_priority + task.dynamic_priority_delta,
            updated_at: task.updated_at.clone(),
            rank_reasons: vec!["fixture".to_string()],
        }
    }

    fn seed(
        repo: &NoopPlanningTaskRepositoryPort,
        workspace: &str,
        tasks: Vec<TaskDefinition>,
        queue_head: Option<PriorityQueueTask>,
    ) {
        repo.clear_direction_authority_snapshot(workspace).unwrap();
        repo.clear_task_authority_snapshot(workspace).unwrap();
        repo.commit_direction_authority_snapshot(
            workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions(),
                authority_mutation_owner_token: None,
            },
        )
        .unwrap();
        repo.commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &TaskAuthorityDocument {
                    version: PLANNING_FORMAT_VERSION,
                    tasks,
                },
                queue_projection: &PriorityQueueProjection {
                    next_task: queue_head,
                    active_tasks: Vec::new(),
                    proposed_tasks: Vec::new(),
                    skipped_tasks: Vec::new(),
                },
            },
        )
        .unwrap();
    }
    #[test]
    fn contract_is_compact_and_names_run_command() {
        // contract는 prompt에 들어갈 만큼 작아야 하지만, worker가 file edit나 payload.worktree_path
        // 오용으로 빠지지 않게 충분한 guardrail도 포함해야 한다.
        let contract = planning_task_tool_contract_json();

        assert!(contract.contains("akra planning-tool run ."));
        assert!(contract.contains("do not use payload.worktree_path"));
        assert!(contract.contains("list_tasks|create_task|update_task"));
        assert!(contract.contains("application-controlled"));
        assert!(!contract.contains("optional provenance"));
        assert!(!contract.contains("source_turn_id/thread_id"));
        assert!(contract.contains("existing descriptions are preserved"));
        assert!(contract.contains(r#""status":["ready","blocked"]"#));
        assert!(!contract.contains("statuses"));
        assert!(contract.len() < 2100);
    }

    #[test]
    fn contract_examples_parse_as_tool_requests() {
        // contract에 넣은 예제가 schema drift를 만들면 worker가 그대로 따라 하다 실패한다.
        let contract =
            serde_json::from_str::<serde_json::Value>(planning_task_tool_contract_json())
                .expect("contract should be valid json");
        let examples = contract
            .get("examples")
            .and_then(|value| value.as_object())
            .expect("contract should include request examples");

        for (op_name, example_value) in examples {
            let request = serde_json::from_value::<PlanningTaskToolRequest>(example_value.clone())
                .unwrap_or_else(|error| {
                    panic!(
                        "example for `{op_name}` should parse as PlanningTaskToolRequest: {error}"
                    )
                });
            match (op_name.as_str(), request) {
                ("list_tasks", PlanningTaskToolRequest::ListTasks(_)) => {}
                ("create_task", PlanningTaskToolRequest::CreateTask(_)) => {}
                ("update_task", PlanningTaskToolRequest::UpdateTask(_)) => {}
                (name, request) => {
                    panic!("example `{name}` parsed into an unexpected variant: {request:?}")
                }
            }
        }
    }

    #[test]
    fn cli_host_context_replaces_untrusted_json_provenance_and_clears_it_when_absent() {
        let parsed = serde_json::from_str::<PlanningTaskToolRequest>(
            r#"{
                "version":1,
                "op":"create_task",
                "apply":true,
                "title":"Follow up",
                "source_turn_id":"spoof-source",
                "origin_session_kind":"main",
                "thread_id":"spoof-thread",
                "turn_id":"spoof-turn",
                "parent_thread_id":"spoof-parent-thread",
                "parent_turn_id":"spoof-parent-turn"
            }"#,
        )
        .expect("request should parse");
        let mut with_host = parsed.clone();
        with_host.apply_cli_host_context(
            Some(" host-parent-thread ".to_string()),
            Some("host-parent-turn".to_string()),
        );
        let PlanningTaskToolRequest::CreateTask(with_host) = with_host else {
            panic!("create request should remain create");
        };
        assert_eq!(with_host.legacy_source_turn_id, None);
        assert_eq!(
            with_host.origin_session_kind,
            Some(OriginSessionKind::Planner)
        );
        assert_eq!(with_host.thread_id, None);
        assert_eq!(with_host.turn_id, None);
        assert_eq!(
            with_host.parent_thread_id.as_deref(),
            Some("host-parent-thread")
        );
        assert_eq!(
            with_host.parent_turn_id.as_deref(),
            Some("host-parent-turn")
        );

        let mut without_host = parsed;
        without_host.apply_cli_host_context(None, None);
        let PlanningTaskToolRequest::CreateTask(without_host) = without_host else {
            panic!("create request should remain create");
        };
        assert_eq!(without_host.legacy_source_turn_id, None);
        assert_eq!(without_host.thread_id, None);
        assert_eq!(without_host.turn_id, None);
        assert_eq!(without_host.parent_thread_id, None);
        assert_eq!(without_host.parent_turn_id, None);
    }

    #[test]
    fn create_task_request_is_flat_for_worker_use() {
        // flat JSON은 model-facing ergonomics의 일부다. nested task object를 요구하기 시작하면
        // prompt 예시와 실제 요청 작성 난도가 같이 올라간다.
        let request = serde_json::from_str::<PlanningTaskToolRequest>(
            r#"{"version":1,"op":"create_task","apply":true,"title":"Review queue idle tool","status":"ready","depends_on":[],"blocked_by":[]}"#,
        )
        .expect("flat create request should parse");

        assert!(matches!(request, PlanningTaskToolRequest::CreateTask(_)));
    }

    #[test]
    fn list_tasks_filters_sorts_limits_and_returns_queue_head() {
        let repo = repo();
        let workspace = workspace("list");
        let older_ready = task(
            "ready-old",
            "Older ready",
            TaskStatus::Ready,
            "2026-05-01T00:00:00Z",
        );
        let newer_ready = task(
            "ready-new",
            "Newer ready",
            TaskStatus::Ready,
            "2026-05-02T00:00:00Z",
        );
        let proposed = task(
            "proposed",
            "Proposal",
            TaskStatus::Proposed,
            "2026-05-03T00:00:00Z",
        );
        seed(
            repo.as_ref(),
            &workspace,
            vec![older_ready.clone(), proposed, newer_ready.clone()],
            Some(queue_task(&older_ready, 1)),
        );
        let response = service(repo)
            .handle_request(
                &workspace,
                PlanningTaskToolRequest::ListTasks(PlanningTaskToolListRequest {
                    version: 1,
                    status: vec![TaskStatus::Ready],
                    limit: Some(1),
                }),
            )
            .unwrap();

        assert!(response.ok);
        assert_eq!(response.operation, "list_tasks");
        assert!(!response.task_authority_changed);
        assert_eq!(response.applied_command_count, 0);
        assert_eq!(response.committed_planning_revision, Some(2));
        assert_eq!(
            response
                .queue_head
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("ready-old")
        );
        assert_eq!(
            response
                .tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ready-new"]
        );
        assert!(
            response
                .guidance
                .iter()
                .any(|line| line.contains("Use update_task"))
        );
    }

    #[test]
    fn list_tasks_reports_missing_authority_and_rejects_unknown_version() {
        let repo = repo();
        let missing = service(repo.clone())
            .handle_request(
                &workspace("missing"),
                PlanningTaskToolRequest::ListTasks(PlanningTaskToolListRequest {
                    version: 1,
                    status: Vec::new(),
                    limit: None,
                }),
            )
            .unwrap_err();
        assert!(
            missing
                .to_string()
                .contains("planning task authority is unavailable")
        );

        let unsupported = service(repo)
            .handle_request(
                &workspace("unsupported-version"),
                PlanningTaskToolRequest::ListTasks(PlanningTaskToolListRequest {
                    version: 2,
                    status: Vec::new(),
                    limit: None,
                }),
            )
            .unwrap_err();
        assert!(
            unsupported
                .to_string()
                .contains("planning-tool request version 2 is not supported")
        );
    }

    #[test]
    fn create_task_requires_apply_then_commits_worker_provenance() {
        let repo = repo();
        let workspace = workspace("create");
        seed(repo.as_ref(), &workspace, Vec::new(), None);
        let tool = service(repo.clone());

        let dry_run_error = tool
            .handle_request(
                &workspace,
                PlanningTaskToolRequest::CreateTask(PlanningTaskToolCreateRequest {
                    version: 1,
                    apply: false,
                    legacy_source_turn_id: None,
                    origin_session_kind: None,
                    thread_id: None,
                    turn_id: None,
                    parent_thread_id: None,
                    parent_turn_id: None,
                    input: PlanningTaskCreatePayload {
                        direction_id: None,
                        direction_relation_note: None,
                        title: "Dry run should fail".to_string(),
                        description: None,
                        status: None,
                        base_priority: None,
                        dynamic_priority_delta: None,
                        priority_reason: None,
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                    },
                }),
            )
            .unwrap_err();
        assert!(
            dry_run_error
                .to_string()
                .contains("create_task requires apply=true")
        );

        let response = tool
            .handle_request(
                &workspace,
                PlanningTaskToolRequest::CreateTask(PlanningTaskToolCreateRequest {
                    version: 1,
                    apply: true,
                    legacy_source_turn_id: Some("legacy-turn".to_string()),
                    origin_session_kind: Some(OriginSessionKind::Parallel),
                    thread_id: Some("worker-thread".to_string()),
                    turn_id: Some("worker-turn".to_string()),
                    parent_thread_id: Some("parent-thread".to_string()),
                    parent_turn_id: Some("parent-turn".to_string()),
                    input: PlanningTaskCreatePayload {
                        direction_id: Some("support-workstream".to_string()),
                        direction_relation_note: Some("covers support".to_string()),
                        title: "Create through tool".to_string(),
                        description: Some("Worker-created tool task".to_string()),
                        status: Some(TaskStatus::Ready),
                        base_priority: Some(70),
                        dynamic_priority_delta: Some(3),
                        priority_reason: Some("operator priority".to_string()),
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                    },
                }),
            )
            .unwrap();

        assert_eq!(response.operation, "create_task");
        assert!(response.task_authority_changed);
        assert_eq!(response.applied_command_count, 1);
        assert_eq!(response.committed_planning_revision, Some(3));
        assert_eq!(response.tasks, Vec::new());
        assert_eq!(
            response
                .queue_head
                .as_ref()
                .map(|task| task.task_title.as_str()),
            Some("Create through tool")
        );
        assert!(
            response
                .guidance
                .iter()
                .any(|line| line.contains("committed_task_ids"))
        );

        let snapshot = repo
            .load_task_authority_snapshot(&workspace)
            .unwrap()
            .unwrap();
        let created = snapshot.task_authority.tasks.first().unwrap();
        assert_eq!(created.direction_id, "support-workstream");
        assert_eq!(created.direction_relation_note, "covers support");
        assert_eq!(created.description, "Worker-created tool task");
        assert_eq!(created.base_priority, 70);
        assert_eq!(created.dynamic_priority_delta, 3);
        assert_eq!(created.priority_reason, "operator priority");
        assert_eq!(created.created_by, TaskActor::Worker);
        assert_eq!(created.source_turn_id.as_deref(), Some("legacy-turn"));
        assert_eq!(
            created.provenance,
            TaskMutationProvenance::new(OriginSessionKind::Parallel)
                .with_thread_turn(
                    Some("worker-thread".to_string()),
                    Some("worker-turn".to_string())
                )
                .with_parent(
                    Some("parent-thread".to_string()),
                    Some("parent-turn".to_string())
                )
        );
    }

    #[test]
    fn update_task_requires_apply_and_preserves_patch_semantics() {
        let repo = repo();
        let workspace = workspace("update");
        let dependency = task(
            "dependency",
            "Dependency",
            TaskStatus::Done,
            "2026-05-01T00:00:00Z",
        );
        let blocker = task(
            "blocker",
            "Blocker",
            TaskStatus::Ready,
            "2026-05-02T00:00:00Z",
        );
        let target = task(
            "target",
            "Target",
            TaskStatus::Ready,
            "2026-05-03T00:00:00Z",
        );
        seed(
            repo.as_ref(),
            &workspace,
            vec![dependency, blocker, target],
            None,
        );
        let tool = service(repo.clone());

        let dry_run_error = tool
            .handle_request(
                &workspace,
                PlanningTaskToolRequest::UpdateTask(PlanningTaskToolUpdateRequest {
                    version: 1,
                    apply: false,
                    legacy_source_turn_id: None,
                    origin_session_kind: None,
                    thread_id: None,
                    turn_id: None,
                    parent_thread_id: None,
                    parent_turn_id: None,
                    input: PlanningTaskUpdatePayload {
                        task_id: "target".to_string(),
                        direction_id: None,
                        direction_relation_note: None,
                        title: None,
                        description: None,
                        status: None,
                        base_priority: None,
                        dynamic_priority_delta: None,
                        priority_reason: None,
                        depends_on: None,
                        blocked_by: None,
                    },
                }),
            )
            .unwrap_err();
        assert!(
            dry_run_error
                .to_string()
                .contains("update_task requires apply=true")
        );

        let response = tool
            .handle_request(
                &workspace,
                PlanningTaskToolRequest::UpdateTask(PlanningTaskToolUpdateRequest {
                    version: 1,
                    apply: true,
                    legacy_source_turn_id: None,
                    origin_session_kind: Some(OriginSessionKind::Main),
                    thread_id: Some("main-thread".to_string()),
                    turn_id: Some("main-turn".to_string()),
                    parent_thread_id: None,
                    parent_turn_id: None,
                    input: PlanningTaskUpdatePayload {
                        task_id: "target".to_string(),
                        direction_id: Some("support-workstream".to_string()),
                        direction_relation_note: Some("moved to support".to_string()),
                        title: Some("Updated through tool".to_string()),
                        description: Some(
                            "Worker description must not replace existing".to_string(),
                        ),
                        status: Some(TaskStatus::Blocked),
                        base_priority: Some(65),
                        dynamic_priority_delta: Some(-5),
                        priority_reason: Some("waiting on blocker".to_string()),
                        depends_on: Some(vec!["dependency".to_string()]),
                        blocked_by: Some(vec!["blocker".to_string()]),
                    },
                }),
            )
            .unwrap();

        assert_eq!(response.operation, "update_task");
        assert!(response.task_authority_changed);
        assert_eq!(response.applied_command_count, 1);
        assert_eq!(response.committed_task_ids, vec!["target"]);
        assert_eq!(response.committed_planning_revision, Some(3));
        assert_eq!(
            response
                .queue_head
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("blocker")
        );

        let snapshot = repo
            .load_task_authority_snapshot(&workspace)
            .unwrap()
            .unwrap();
        let updated = snapshot
            .task_authority
            .tasks
            .iter()
            .find(|task| task.id == "target")
            .unwrap();
        assert_eq!(updated.direction_id, "support-workstream");
        assert_eq!(updated.direction_relation_note, "moved to support");
        assert_eq!(updated.title, "Updated through tool");
        assert_eq!(updated.description, "Target");
        assert_eq!(updated.status, TaskStatus::Blocked);
        assert_eq!(updated.base_priority, 65);
        assert_eq!(updated.dynamic_priority_delta, -5);
        assert_eq!(updated.priority_reason, "waiting on blocker");
        assert_eq!(updated.depends_on, vec!["dependency"]);
        assert_eq!(updated.blocked_by, vec!["blocker"]);
        assert_eq!(updated.last_updated_by, TaskActor::Worker);
        assert_eq!(
            updated.provenance,
            TaskMutationProvenance::new(OriginSessionKind::Main).with_thread_turn(
                Some("main-thread".to_string()),
                Some("main-turn".to_string())
            )
        );
    }
}
