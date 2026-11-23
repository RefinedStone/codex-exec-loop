use super::{
    SqlitePlanningAuthorityAdapter, authority_connection, authority_task,
    empty_test_queue_projection, temp_workspace, test_direction_catalog,
};
use crate::application::port::inbound::planning_task_tool_port::{
    PlanningTaskToolListRequest, PlanningTaskToolRequest,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::service::planning::task_mutation::{
    PlanningTaskMutationCommand, PlanningTaskMutationRequest, PlanningTaskMutationService,
    PlanningTaskMutationSource, PlanningTaskUpdateInput,
};
use crate::application::service::planning::task_tool::PlanningTaskToolService;
use crate::domain::planning::{
    OriginSessionKind, PriorityQueueService, TASK_AUTHORITY_FORMAT_VERSION, TaskAcceptanceChannel,
    TaskAcceptanceCriterion, TaskAuthorityDocument, TaskMutationProvenance,
};
use std::sync::Arc;

fn update_input(task_id: &str) -> PlanningTaskUpdateInput {
    PlanningTaskUpdateInput {
        acceptance: None,
        task_id: task_id.to_string(),
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
        expected_planning_revision: None,
    }
}

#[test]
fn legacy_db_acceptance_round_trips_through_user_mutation_worker_update_and_list() {
    let workspace = temp_workspace("task-acceptance-legacy-round-trip");
    let adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    adapter
        .commit_direction_authority_snapshot(
            &workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &test_direction_catalog(&["direction-1"]),
                authority_mutation_owner_token: None,
            },
        )
        .unwrap();
    let task = authority_task("legacy-task", "direction-1");
    adapter
        .commit_task_authority_snapshot(
            &workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &TaskAuthorityDocument {
                    version: 1,
                    tasks: vec![task.clone()],
                },
                queue_projection: &empty_test_queue_projection(),
            },
        )
        .unwrap();
    // Materialize the old on-disk JSON shape, not a current struct with empty fields.
    let mut legacy_json = serde_json::to_value(&task).unwrap();
    let object = legacy_json.as_object_mut().unwrap();
    object.remove("acceptance");
    object.remove("acceptance_revision");
    let connection = authority_connection(&workspace);
    connection
        .execute(
            "UPDATE planning_tasks SET content_json = ?1 WHERE task_id = ?2",
            rusqlite::params![serde_json::to_string(&legacy_json).unwrap(), task.id],
        )
        .unwrap();
    drop(connection);
    let legacy = adapter
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    assert_eq!(legacy.task_authority.version, 1);
    assert!(legacy.task_authority.tasks[0].acceptance.is_empty());
    assert_eq!(legacy.task_authority.tasks[0].acceptance_revision, 0);

    let criterion = TaskAcceptanceCriterion {
        id: "C1".to_string(),
        scenario: "The fixture command exits successfully.".to_string(),
        channel: TaskAcceptanceChannel::Command,
        essential: true,
        verification_recipe: None,
    };
    let mut update = update_input(&task.id);
    update.acceptance = Some(vec![criterion.clone()]);
    update.expected_planning_revision = Some(legacy.planning_revision);
    let service = PlanningTaskMutationService::new(adapter.clone(), PriorityQueueService::new());
    let created = service
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::User,
            legacy_source_turn_id: None,
            provenance: TaskMutationProvenance::new(OriginSessionKind::Main),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(update)],
        })
        .unwrap();
    assert_eq!(
        created.committed_planning_revision,
        legacy.planning_revision + 1
    );
    drop(service);
    drop(adapter);

    let restarted = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let saved = restarted
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    assert_eq!(saved.task_authority.version, TASK_AUTHORITY_FORMAT_VERSION);
    assert_eq!(
        saved.task_authority.tasks[0].acceptance,
        vec![criterion.clone()]
    );
    assert_eq!(saved.task_authority.tasks[0].acceptance_revision, 1);

    let mut worker_update = update_input(&task.id);
    worker_update.title = Some("Worker clarification".to_string());
    let worker = PlanningTaskMutationService::new(restarted.clone(), PriorityQueueService::new())
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: workspace.clone(),
            source: PlanningTaskMutationSource::Worker,
            legacy_source_turn_id: None,
            provenance: TaskMutationProvenance::new(OriginSessionKind::Planner),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(worker_update)],
        })
        .unwrap();
    let response = PlanningTaskToolService::new(restarted.clone(), PriorityQueueService::new())
        .handle_request(
            &workspace,
            PlanningTaskToolRequest::ListTasks(PlanningTaskToolListRequest {
                version: 1,
                status: Vec::new(),
                limit: None,
            }),
        )
        .unwrap();
    assert_eq!(
        response.committed_planning_revision,
        Some(worker.committed_planning_revision)
    );
    assert_eq!(response.tasks.len(), 1);
    assert_eq!(response.tasks[0].title, "Worker clarification");
    assert_eq!(response.tasks[0].acceptance, vec![criterion.clone()]);
    assert_eq!(response.tasks[0].acceptance_revision, 1);
    assert!(!response.task_authority_changed);
    assert_eq!(
        restarted
            .load_direction_authority_snapshot(&workspace)
            .unwrap()
            .unwrap()
            .directions
            .version,
        1
    );

    let records = restarted
        .load_task_authority_mutations(
            &workspace,
            legacy.planning_revision,
            worker.committed_planning_revision,
        )
        .unwrap();
    assert_eq!(records.len(), 2);
    for record in records {
        let recorded = record.after_task.unwrap();
        assert_eq!(recorded.acceptance, vec![criterion.clone()]);
        assert_eq!(recorded.acceptance_revision, 1);
    }
}
