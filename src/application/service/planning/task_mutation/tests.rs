use std::sync::Arc;

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
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    PriorityQueueProjection, QueueIdleConfig, TaskActor, TaskAuthorityDocument, TaskDefinition,
    TaskStatus,
};

fn workspace(label: &str) -> String {
    format!(
        "/tmp/akra-planning-task-mutation-test-{label}-{}",
        std::process::id()
    )
}

fn repo() -> Arc<NoopPlanningTaskRepositoryPort> {
    Arc::new(NoopPlanningTaskRepositoryPort)
}

fn directions() -> DirectionCatalogDocument {
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
        updated_at: "2026-04-29T00:00:00Z".to_string(),
    }
}

fn seed(
    repo: &NoopPlanningTaskRepositoryPort,
    workspace: &str,
    task_authority: TaskAuthorityDocument,
) {
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

    let result = service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::Llm,
            source_turn_id: Some("turn-llm".to_string()),
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
fn update_preserves_unspecified_fields() {
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
    assert_eq!(updated.title, "Updated title");
    assert_eq!(updated.description, "Existing task");
    assert_eq!(updated.status, TaskStatus::Blocked);
    assert_eq!(updated.created_by, TaskActor::User);
    assert_eq!(updated.last_updated_by, TaskActor::Llm);
    assert_eq!(updated.source_turn_id.as_deref(), Some("turn-2"));
}

#[test]
fn no_op_update_does_not_bump_revision_or_touch_audit_fields() {
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
            commands,
        })
        .unwrap_err();

    assert!(error.to_string().contains("at most 16 command"));
}

#[test]
fn legacy_task_authority_is_rejected_by_extractor() {
    let message = r#"```json
{"task_authority":{"version":1,"tasks":[]}}
```"#;

    assert!(matches!(
        extract_planning_task_commands(message),
        PlanningTaskCommandExtraction::LegacyTaskAuthorityRejected(_)
    ));
}

#[test]
fn unknown_command_fields_and_delete_ops_are_invalid() {
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
