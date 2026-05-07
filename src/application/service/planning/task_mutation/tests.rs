use super::{
    PlanningTaskCommandExtraction, PlanningTaskCreateInput, PlanningTaskCreatePreviewRequest,
    PlanningTaskMutationCommand, PlanningTaskMutationRequest, PlanningTaskMutationService,
    PlanningTaskMutationSource, PlanningTaskUpdateInput, extract_planning_task_commands,
};
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit,
    PlanningTaskRepositoryPort,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    PLANNING_FORMAT_VERSION, PriorityQueueProjection, QueueIdleConfig, TaskActor,
    TaskAuthorityDocument, TaskDefinition, TaskMutationProvenance, TaskStatus,
};
use std::sync::Arc;

/*
 * 이 테스트들은 helper를 직접 때리지 않고 repository port를 통해 mutation service를 검증한다.
 * fixture를 실제 app-server 경로에 가깝게 유지하려는 의도다. authority snapshot을 load하고,
 * preview 또는 command batch를 적용한 뒤 queue projection을 재계산하고, optimistic planning
 * revision으로 commit하는 흐름 전체를 고정한다.
 */
fn workspace(label: &str) -> String {
    // in-memory/noop repository는 workspace string을 key로 쓴다. process id를 붙여 병렬 test
    // process가 authority state를 공유하지 않게 한다.
    format!(
        "/tmp/akra-planning-task-mutation-test-{label}-{}",
        std::process::id()
    )
}
fn repo() -> Arc<NoopPlanningTaskRepositoryPort> {
    Arc::new(NoopPlanningTaskRepositoryPort)
}
fn provenance() -> TaskMutationProvenance {
    TaskMutationProvenance::default()
}
fn directions() -> DirectionCatalogDocument {
    // active direction 하나면 mutation layer의 default direction 선택과 실제 direction validation을
    // 동시에 검증하기 충분하다.
    DirectionCatalogDocument {
        version: PLANNING_FORMAT_VERSION,
        queue_idle: QueueIdleConfig::default(),
        directions: vec![DirectionDefinition {
            id: "general-workstream".to_string(),
            title: "General".to_string(),
            summary: "Handle general planning work.".to_string(),
            success_criteria: vec!["done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    }
}
fn task(id: &str, status: TaskStatus) -> TaskDefinition {
    // baseline task는 의도적으로 user audit field와 stable timestamp를 가진다. update 테스트가
    // mutation이 건드릴 수 있는 field와 보존해야 하는 field를 정확히 구분할 수 있게 한다.
    TaskDefinition {
        id: id.to_string(),
        direction_id: "general-workstream".to_string(),
        direction_relation_note: "supports direction".to_string(),
        title: "Existing task".to_string(),
        description: "Existing task".to_string(),
        status,
        base_priority: 10,
        dynamic_priority_delta: 0,
        priority_reason: String::new(),
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        created_by: TaskActor::User,
        last_updated_by: TaskActor::User,
        source_turn_id: None,
        provenance: provenance(),
        updated_at: "2026-04-29T00:00:00Z".to_string(),
    }
}
fn seed(
    repo: &NoopPlanningTaskRepositoryPort,
    workspace: &str,
    task_authority: TaskAuthorityDocument,
) {
    /*
     * seed도 production code가 쓰는 repository commit API를 통과한다. 먼저 clear하는 이유는
     * 재사용된 workspace key가 test 사이 revision/snapshot leakage를 숨기지 못하게 하기 위해서다.
     */
    repo.clear_direction_authority_snapshot(workspace).unwrap();
    repo.clear_task_authority_snapshot(workspace).unwrap();
    repo.commit_direction_authority_snapshot(
        workspace,
        PlanningDirectionAuthorityCommit {
            observed_planning_revision: None,
            directions: &directions(),
        },
    )
    .unwrap();
    repo.commit_task_authority_snapshot(
        workspace,
        PlanningTaskAuthorityCommit {
            observed_planning_revision: None,
            task_authority: &task_authority,
            queue_projection: &PriorityQueueProjection {
                next_task: None,
                active_tasks: Vec::new(),
                proposed_tasks: Vec::new(),
                skipped_tasks: Vec::new(),
            },
        },
    )
    .unwrap();
}
#[test]
fn user_preview_and_llm_create_share_defaults_and_audit() {
    /*
     * user preview와 LLM command commit은 서로 다른 public method로 들어오지만 task default,
     * direction fallback, audit attribution은 같아야 한다. TUI preview path와 worker-response
     * command path 사이의 drift를 잡는 회귀다.
     */
    let repo = repo();
    let workspace = workspace("shared-defaults");
    seed(
        repo.as_ref(),
        &workspace,
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: Vec::new(),
        },
    );
    let service = PlanningTaskMutationService::new(
        repo.clone(),
        crate::domain::planning::PriorityQueueService::new(),
    );
    let preview = service
        .preview_create_task(PlanningTaskCreatePreviewRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::User,
            source_turn_id: Some("turn-user".to_string()),
            provenance: provenance(),
            input: PlanningTaskCreateInput {
                direction_id: None,
                direction_relation_note: None,
                title: "Create from task command".to_string(),
                description: Some("Create from task command".to_string()),
                status: None,
                base_priority: None,
                dynamic_priority_delta: None,
                priority_reason: None,
                depends_on: Vec::new(),
                blocked_by: Vec::new(),
            },
        })
        .unwrap();
    assert_eq!(preview.task.status, TaskStatus::Ready);
    assert_eq!(preview.task.base_priority, 80);
    assert_eq!(preview.task.created_by, TaskActor::User);
    assert_eq!(preview.task.last_updated_by, TaskActor::User);

    // LLM path는 apply_commands를 통해 실제 persistence까지 간다. returned commit summary만
    // 믿지 않고 repository snapshot을 읽어 audit field가 저장됐는지 확인한다.
    let result = service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::Llm,
            source_turn_id: Some("turn-llm".to_string()),
            provenance: provenance(),
            commands: vec![PlanningTaskMutationCommand::CreateTask(
                PlanningTaskCreateInput {
                    direction_id: None,
                    direction_relation_note: None,
                    title: "Create from worker command".to_string(),
                    description: None,
                    status: None,
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                },
            )],
        })
        .unwrap();
    assert!(result.task_authority_changed);
    let snapshot = repo
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    let llm_task = snapshot
        .task_authority
        .tasks
        .iter()
        .find(|task| task.title == "Create from worker command")
        .unwrap();
    assert_eq!(llm_task.status, TaskStatus::Ready);
    assert_eq!(llm_task.base_priority, 80);
    assert_eq!(llm_task.created_by, TaskActor::Llm);
    assert_eq!(llm_task.last_updated_by, TaskActor::Llm);
    assert_eq!(llm_task.source_turn_id.as_deref(), Some("turn-llm"));
}

#[test]
fn create_records_generic_thread_turn_provenance() {
    /*
     * source_turn_id는 legacy 조회 키로 남기되, 새 감사 정보는 provider-neutral
     * thread_id/turn_id/parent_* 필드에 저장한다. source_turn_id가 없으면 실제 worker
     * turn_id를 fallback으로 써서 synthetic orchestration id가 accepted task에 남지 않게 한다.
     */
    let repo = repo();
    let workspace = workspace("generic-provenance");
    seed(
        repo.as_ref(),
        &workspace,
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: Vec::new(),
        },
    );
    let service = PlanningTaskMutationService::new(
        repo.clone(),
        crate::domain::planning::PriorityQueueService::new(),
    );
    let provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
        .with_thread_turn(
            Some("worker-thread-1".to_string()),
            Some("worker-turn-1".to_string()),
        )
        .with_parent(
            Some("visible-thread-1".to_string()),
            Some("visible-turn-1".to_string()),
        );

    service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::Llm,
            source_turn_id: None,
            provenance: provenance.clone(),
            commands: vec![PlanningTaskMutationCommand::CreateTask(
                PlanningTaskCreateInput {
                    direction_id: None,
                    direction_relation_note: None,
                    title: "Persist generic provenance".to_string(),
                    description: None,
                    status: None,
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                },
            )],
        })
        .unwrap();

    let snapshot = repo
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    let task = snapshot.task_authority.tasks.first().unwrap();
    assert_eq!(task.source_turn_id.as_deref(), Some("worker-turn-1"));
    assert_eq!(task.provenance, provenance);
}

#[test]
fn update_preserves_unspecified_fields() {
    /*
     * update command는 partial patch다. 누락 field를 빈 replacement로 오해하지 않으면서,
     * 실제 변경된 field가 있을 때 audit identity만 갱신하는지 검증한다.
     */
    let repo = repo();
    let workspace = workspace("preserve-update");
    seed(
        repo.as_ref(),
        &workspace,
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready)],
        },
    );
    let service = PlanningTaskMutationService::new(
        repo.clone(),
        crate::domain::planning::PriorityQueueService::new(),
    );

    service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::Llm,
            source_turn_id: Some("turn-2".to_string()),
            provenance: provenance(),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(
                PlanningTaskUpdateInput {
                    task_id: "task-1".to_string(),
                    direction_id: None,
                    direction_relation_note: None,
                    title: Some("Updated title".to_string()),
                    description: None,
                    status: Some(TaskStatus::Blocked),
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: None,
                    blocked_by: None,
                },
            )],
        })
        .unwrap();
    let snapshot = repo
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    let updated = &snapshot.task_authority.tasks[0];
    // 제공된 patch field와 audit field만 움직여야 한다.
    assert_eq!(updated.title, "Updated title");
    assert_eq!(updated.description, "Existing task");
    assert_eq!(updated.status, TaskStatus::Blocked);
    assert_eq!(updated.created_by, TaskActor::User);
    assert_eq!(updated.last_updated_by, TaskActor::Llm);
    assert_eq!(updated.source_turn_id.as_deref(), Some("turn-2"));
}
#[test]
fn llm_update_preserves_existing_description_even_when_supplied() {
    /*
     * worker-authored updates can refine scheduling fields, but an existing description is user-facing
     * task context. Supplying description in an LLM patch must not rewrite it.
     */
    let repo = repo();
    let workspace = workspace("llm-preserve-description");
    seed(
        repo.as_ref(),
        &workspace,
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready)],
        },
    );
    let service = PlanningTaskMutationService::new(
        repo.clone(),
        crate::domain::planning::PriorityQueueService::new(),
    );

    service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::Llm,
            source_turn_id: Some("turn-llm-description".to_string()),
            provenance: provenance(),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(
                PlanningTaskUpdateInput {
                    task_id: "task-1".to_string(),
                    direction_id: None,
                    direction_relation_note: None,
                    title: None,
                    description: Some("LLM-generated rewrite".to_string()),
                    status: Some(TaskStatus::Blocked),
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: None,
                    blocked_by: None,
                },
            )],
        })
        .unwrap();
    let snapshot = repo
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    let updated = &snapshot.task_authority.tasks[0];
    assert_eq!(updated.description, "Existing task");
    assert_eq!(updated.status, TaskStatus::Blocked);
    assert_eq!(updated.last_updated_by, TaskActor::Llm);
    assert_eq!(
        updated.source_turn_id.as_deref(),
        Some("turn-llm-description")
    );
}
#[test]
fn user_update_can_replace_existing_description() {
    /*
     * Operator-facing edits still own description changes. The LLM guard must not remove the
     * admin/runtime-user ability to correct task text intentionally.
     */
    let repo = repo();
    let workspace = workspace("user-update-description");
    seed(
        repo.as_ref(),
        &workspace,
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready)],
        },
    );
    let service = PlanningTaskMutationService::new(
        repo.clone(),
        crate::domain::planning::PriorityQueueService::new(),
    );

    service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::User,
            source_turn_id: Some("turn-user-description".to_string()),
            provenance: provenance(),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(
                PlanningTaskUpdateInput {
                    task_id: "task-1".to_string(),
                    direction_id: None,
                    direction_relation_note: None,
                    title: None,
                    description: Some("Operator-authored description".to_string()),
                    status: None,
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: None,
                    blocked_by: None,
                },
            )],
        })
        .unwrap();
    let snapshot = repo
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    let updated = &snapshot.task_authority.tasks[0];
    assert_eq!(updated.description, "Operator-authored description");
    assert_eq!(updated.last_updated_by, TaskActor::User);
    assert_eq!(
        updated.source_turn_id.as_deref(),
        Some("turn-user-description")
    );
}
#[test]
fn no_op_update_does_not_bump_revision_or_touch_audit_fields() {
    /*
     * LLM이 이미 적용된 intent를 반복하면 no-op update가 흔히 나온다. service는 correlation을
     * 위해 addressed task id를 보고하되, 새 planning revision을 만들거나 audit timestamp를
     * 다시 쓰면 안 된다.
     */
    let repo = repo();
    let workspace = workspace("noop-update");
    seed(
        repo.as_ref(),
        &workspace,
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Ready)],
        },
    );
    let before = repo
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    let service = PlanningTaskMutationService::new(
        repo.clone(),
        crate::domain::planning::PriorityQueueService::new(),
    );
    let result = service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::Llm,
            source_turn_id: Some("turn-noop".to_string()),
            provenance: provenance(),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(
                PlanningTaskUpdateInput {
                    task_id: "task-1".to_string(),
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
            )],
        })
        .unwrap();
    let after = repo
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    assert!(!result.task_authority_changed);
    assert_eq!(result.applied_command_count, 0);
    assert_eq!(result.committed_task_ids, vec!["task-1"]);
    assert_eq!(after.planning_revision, before.planning_revision);
    assert_eq!(after.task_authority, before.task_authority);
}
#[test]
fn oversized_worker_command_batch_is_rejected_before_mutation() {
    /*
     * command budget은 큰 worker response가 authority document를 과도하게 바꾸지 못하게 막는다.
     * 이 case는 per-command builder나 repository commit이 실행되기 전에 limit이 적용됨을 검증한다.
     */
    let repo = repo();
    let workspace = workspace("command-budget");
    seed(
        repo.as_ref(),
        &workspace,
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: Vec::new(),
        },
    );
    let service = PlanningTaskMutationService::new(
        repo,
        crate::domain::planning::PriorityQueueService::new(),
    );
    let commands = (0..17)
        .map(|index| {
            PlanningTaskMutationCommand::CreateTask(PlanningTaskCreateInput {
                direction_id: None,
                direction_relation_note: None,
                title: format!("Generated follow-up {index}"),
                description: None,
                status: None,
                base_priority: None,
                dynamic_priority_delta: None,
                priority_reason: None,
                depends_on: Vec::new(),
                blocked_by: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    let error = service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace,
            source: PlanningTaskMutationSource::Llm,
            source_turn_id: Some("turn-many".to_string()),
            provenance: provenance(),
            commands,
        })
        .unwrap_err();

    assert!(error.to_string().contains("at most 16 command"));
}
#[test]
fn unknown_command_fields_and_delete_ops_are_invalid() {
    /*
     * command schema는 의도적으로 엄격하다. caller가 create command에 id를 몰래 넣을 수 없고,
     * destructive task deletion은 이 mutation boundary 밖에 있다.
     */
    let unknown_field = r#"{"planning_task_commands":{"version":1,"commands":[{"op":"create_task","title":"x","id":"forbidden"}]}}"#;
    let delete_op = r#"{"planning_task_commands":{"version":1,"commands":[{"op":"delete_task","task_id":"task-1"}]}}"#;

    assert!(matches!(
        extract_planning_task_commands(unknown_field),
        PlanningTaskCommandExtraction::InvalidCommands { .. }
    ));
    assert!(matches!(
        extract_planning_task_commands(delete_op),
        PlanningTaskCommandExtraction::InvalidCommands { .. }
    ));
}
#[test]
fn invalid_command_extraction_preserves_rejected_payload_for_repair() {
    // invalid payload는 rejected JSON을 보존한다. 주변 prompt/repair 흐름이 worker에게 schema
    // validation 실패 대상을 정확히 보여 줄 수 있게 하기 위해서다.
    let wrapped_command = r#"{"planning_task_commands":{"version":1,"commands":[{"create_task":{"title":"Queue follow-up"}}]}}"#;
    let extraction = extract_planning_task_commands(wrapped_command);

    assert!(matches!(
        extraction,
        PlanningTaskCommandExtraction::InvalidCommands {
            error,
            rejected_json: Some(rejected_json),
        } if error.contains("missing field `op`")
            && rejected_json.contains("\"create_task\"")
    ));
}
#[test]
fn extractor_accepts_balanced_json_embedded_in_markdown_text() {
    /*
     * worker response는 JSON 주변에 prose를 포함하는 경우가 많다. extractor는 fenced block을
     * 요구하지 않고 balanced command object를 분리해야 ordinary model output의 유효 command를
     * 놓치지 않는다.
     */
    let message = r#"Updated planning commands:
{"planning_task_commands":{"version":1,"commands":[{"op":"create_task","title":"Write review response"}]}}

Summary: one task added."#;
    let extraction = extract_planning_task_commands(message);

    assert!(matches!(
        extraction,
        PlanningTaskCommandExtraction::Commands(commands) if matches!(
            commands.as_slice(),
            [PlanningTaskMutationCommand::CreateTask(input)]
                if input.title == "Write review response"
        )
    ));
}
#[test]
fn terminal_status_change_is_rejected() {
    /*
     * terminal task는 historical record다. task가 Done/Cancelled가 된 뒤에는 이후 worker
     * output이 generic update path로 reopen하거나 다른 terminal 상태로 재분류할 수 없다.
     */
    let repo = repo();
    let workspace = workspace("terminal-regression");
    seed(
        repo.as_ref(),
        &workspace,
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task("task-1", TaskStatus::Done)],
        },
    );
    let service = PlanningTaskMutationService::new(
        repo,
        crate::domain::planning::PriorityQueueService::new(),
    );
    let error = service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace,
            source: PlanningTaskMutationSource::Llm,
            source_turn_id: None,
            provenance: provenance(),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(
                PlanningTaskUpdateInput {
                    task_id: "task-1".to_string(),
                    direction_id: None,
                    direction_relation_note: None,
                    title: None,
                    description: None,
                    status: Some(TaskStatus::Cancelled),
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: None,
                    blocked_by: None,
                },
            )],
        })
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("cannot change from terminal status")
    );
}
