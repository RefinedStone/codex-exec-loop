use super::PlanningValidationService;
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    PlanningFileKind, PlanningWorkspaceFiles, QueueIdleConfig, QueueIdlePolicy,
};

fn valid_result_output_markdown() -> &'static str {
    r#"# Result Output Prompt

- Summarize the work you actually completed in this turn.
- Mention task-authority updates when they changed.
"#
}

fn test_directions(direction_id: &str) -> DirectionCatalogDocument {
    DirectionCatalogDocument {
        version: PLANNING_FORMAT_VERSION,
        queue_idle: QueueIdleConfig::default(),
        directions: vec![DirectionDefinition {
            id: direction_id.to_string(),
            title: "Direction A".to_string(),
            summary: "Keep task updates aligned.".to_string(),
            success_criteria: vec!["Only aligned tasks enter the authority.".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    }
}

#[test]
fn bootstrap_artifacts_validate_successfully() {
    let bootstrap_service = PlanningBootstrapService::new();
    let validation_service = PlanningValidationService::new();
    let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

    let task_authority_json = serde_json::to_string(&artifacts.task_authority)
        .expect("bootstrap task authority should serialize");
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &artifacts.directions,
        task_authority_json: &task_authority_json,
        result_output_markdown: &artifacts.result_output_markdown,
    });

    assert!(result.is_valid(), "{:?}", result.report.issues);
    assert!(result.directions.is_some());
    assert!(result.task_authority.is_some());
}

#[test]
fn rejects_unknown_direction_references() {
    let validation_service = PlanningValidationService::new();
    let directions = test_directions("product-direction");
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &directions,
        task_authority_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "missing-direction",
      "direction_relation_note": "Loose relation",
      "title": "Draft follow-up work",
      "description": "Write one next task.",
      "status": "ready",
      "base_priority": 10,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:00:00Z"
    }
  ]
}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(!result.is_valid());
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "missing_direction_reference"
    }));
}

#[test]
fn rejects_llm_tasks_without_relation_notes() {
    let validation_service = PlanningValidationService::new();
    let directions = test_directions("direction-a");
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &directions,
        task_authority_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "direction-a",
      "direction_relation_note": "",
      "title": "Add a follow-up",
      "description": "LLM adds a new task.",
      "status": "proposed",
      "base_priority": 5,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-1",
      "updated_at": "2026-04-09T10:00:00Z"
    }
  ]
}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(!result.is_valid());
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "missing_direction_relation_note"
    }));
}

#[test]
fn rejects_dependency_cycles() {
    let validation_service = PlanningValidationService::new();
    let directions = test_directions("direction-a");
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &directions,
        task_authority_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "direction-a",
      "direction_relation_note": "Still under direction A",
      "title": "Task 1",
      "description": "First task.",
      "status": "ready",
      "base_priority": 10,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": ["task-2"],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:00:00Z"
    },
    {
      "id": "task-2",
      "direction_id": "direction-a",
      "direction_relation_note": "Still under direction A",
      "title": "Task 2",
      "description": "Second task.",
      "status": "ready",
      "base_priority": 9,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": ["task-1"],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:01:00Z"
    }
  ]
}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(!result.is_valid());
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "dependency_cycle_detected"
    }));
}

#[test]
fn rejects_unsupported_task_authority_version_without_schema_validation() {
    let bootstrap_service = PlanningBootstrapService::new();
    let validation_service = PlanningValidationService::new();
    let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &artifacts.directions,
        task_authority_json: r#"{
  "version": 2
}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(!result.is_valid());
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "unsupported_task_authority_version"
    }));
}

#[test]
fn rejects_unknown_task_authority_fields() {
    let bootstrap_service = PlanningBootstrapService::new();
    let validation_service = PlanningValidationService::new();
    let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &artifacts.directions,
        task_authority_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "direction_relation_note": "",
      "title": "Task 1",
      "description": "Keep schema and serde aligned.",
      "status": "ready",
      "base_priority": 10,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:00:00Z",
      "unexpected_field": true
    }
  ]
}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(!result.is_valid());
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "task_authority_parse_failed"
    }));
}

#[test]
fn rejects_conflicting_done_relationships_and_multiple_in_progress_tasks() {
    let bootstrap_service = PlanningBootstrapService::new();
    let validation_service = PlanningValidationService::new();
    let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &artifacts.directions,
        task_authority_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "direction_relation_note": "",
      "title": "Task 1",
      "description": "Still running.",
      "status": "in_progress",
      "base_priority": 10,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:00:00Z"
    },
    {
      "id": "task-2",
      "direction_id": "example-direction",
      "direction_relation_note": "",
      "title": "Task 2",
      "description": "Also marked active.",
      "status": "in_progress",
      "base_priority": 9,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:01:00Z"
    },
    {
      "id": "task-3",
      "direction_id": "example-direction",
      "direction_relation_note": "",
      "title": "Task 3",
      "description": "Claims to be done too early.",
      "status": "done",
      "base_priority": 8,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": ["task-1"],
      "blocked_by": ["task-1"],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T10:02:00Z"
    }
  ]
}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(!result.is_valid());
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "dependency_blocker_conflict"
    }));
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "done_task_unresolved_dependency"
    }));
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "done_task_unresolved_blocker"
    }));
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::TaskAuthority
            && issue.code == "multiple_in_progress_tasks"
    }));
}

#[test]
fn rejects_result_output_without_heading() {
    let bootstrap_service = PlanningBootstrapService::new();
    let validation_service = PlanningValidationService::new();
    let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
    let task_authority_json = serde_json::to_string(&artifacts.task_authority)
        .expect("bootstrap task authority should serialize");
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &artifacts.directions,
        task_authority_json: &task_authority_json,
        result_output_markdown: "Summarize the completed work.",
    });

    assert!(!result.is_valid());
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::ResultOutput
            && issue.code == "missing_result_output_heading"
    }));
}

#[test]
fn warns_on_result_output_placeholders() {
    let bootstrap_service = PlanningBootstrapService::new();
    let validation_service = PlanningValidationService::new();
    let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
    let task_authority_json = serde_json::to_string(&artifacts.task_authority)
        .expect("bootstrap task authority should serialize");
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &artifacts.directions,
        task_authority_json: &task_authority_json,
        result_output_markdown: r#"# Result Output Prompt

- TODO: replace this guidance before relying on it.
"#,
    });

    assert!(result.is_valid(), "{:?}", result.report.issues);
    assert!(result.report.issues.iter().any(|issue| {
        issue.file_kind == PlanningFileKind::ResultOutput
            && issue.code == "result_output_contains_placeholder"
    }));
}

#[test]
fn rejects_blank_result_output_prompt() {
    let bootstrap_service = PlanningBootstrapService::new();
    let validation_service = PlanningValidationService::new();
    let artifacts = bootstrap_service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
    let task_authority_json = serde_json::to_string(&artifacts.task_authority)
        .expect("bootstrap task authority should serialize");
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &artifacts.directions,
        task_authority_json: &task_authority_json,
        result_output_markdown: "   ",
    });

    assert!(!result.is_valid());
    assert!(result.report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::ResultOutput && issue.code == "blank_result_output"
    }));
}

#[test]
fn rejects_detail_doc_paths_that_only_match_prefix_textually() {
    let validation_service = PlanningValidationService::new();
    let mut directions = test_directions("direction-a");
    directions.directions[0].detail_doc_path =
        ".codex-exec-loop/planning/directions_backup/direction-a.md".to_string();
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &directions,
        task_authority_json: r#"{"version":1,"tasks":[]}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(result.is_valid(), "{:?}", result.report.issues);
    let mut report = result.report;
    let directions = result
        .directions
        .expect("directions should parse for supporting file validation");
    validation_service.validate_direction_supporting_files(&directions, |_| true, &mut report);
    assert!(report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::Directions && issue.code == "invalid_detail_doc_path"
    }));
}

#[test]
fn rejects_detail_doc_paths_with_parent_dir_components() {
    let validation_service = PlanningValidationService::new();
    let mut directions = test_directions("direction-a");
    directions.directions[0].detail_doc_path =
        ".codex-exec-loop/planning/directions/../direction-a.md".to_string();
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &directions,
        task_authority_json: r#"{"version":1,"tasks":[]}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(result.is_valid(), "{:?}", result.report.issues);
    let mut report = result.report;
    let directions = result
        .directions
        .expect("directions should parse for supporting file validation");
    validation_service.validate_direction_supporting_files(&directions, |_| true, &mut report);
    assert!(report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::Directions && issue.code == "invalid_detail_doc_path"
    }));
}

#[test]
fn rejects_queue_idle_prompt_paths_that_only_match_prefix_textually() {
    let validation_service = PlanningValidationService::new();
    let mut directions = test_directions("direction-a");
    directions.queue_idle = QueueIdleConfig {
        policy: QueueIdlePolicy::ReviewAndEnqueue,
        prompt_path: ".codex-exec-loop/planning/prompts_backup/queue-idle-review.md".to_string(),
    };
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &directions,
        task_authority_json: r#"{"version":1,"tasks":[]}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(result.is_valid(), "{:?}", result.report.issues);
    let mut report = result.report;
    let directions = result
        .directions
        .expect("directions should parse for supporting file validation");
    validation_service.validate_direction_supporting_files(&directions, |_| true, &mut report);
    assert!(report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::Directions
            && issue.code == "invalid_queue_idle_prompt_path"
    }));
}

#[test]
fn rejects_queue_idle_prompt_paths_with_parent_dir_components() {
    let validation_service = PlanningValidationService::new();
    let mut directions = test_directions("direction-a");
    directions.queue_idle = QueueIdleConfig {
        policy: QueueIdlePolicy::ReviewAndEnqueue,
        prompt_path: ".codex-exec-loop/planning/prompts/../queue-idle-review.md".to_string(),
    };
    let result = validation_service.validate_workspace_files(PlanningWorkspaceFiles {
        directions: &directions,
        task_authority_json: r#"{"version":1,"tasks":[]}"#,
        result_output_markdown: valid_result_output_markdown(),
    });

    assert!(result.is_valid(), "{:?}", result.report.issues);
    let mut report = result.report;
    let directions = result
        .directions
        .expect("directions should parse for supporting file validation");
    validation_service.validate_direction_supporting_files(&directions, |_| true, &mut report);
    assert!(report.errors().iter().any(|issue| {
        issue.file_kind == PlanningFileKind::Directions
            && issue.code == "invalid_queue_idle_prompt_path"
    }));
}
