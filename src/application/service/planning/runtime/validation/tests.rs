use super::PlanningValidationService;
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    PlanningFileKind, PlanningWorkspaceFiles, QueueIdleConfig, QueueIdlePolicy,
};

/*
 * These tests pin the application validation boundary used by draft promotion, runtime snapshots,
 * proposal promotion, doctor, and reset flows.
 * They intentionally exercise PlanningValidationService through PlanningWorkspaceFiles so each case proves
 * the same report codes that adapters show to operators, instead of testing domain validators in isolation.
 */

// Shared successful result-output contract; individual tests replace it only when they target markdown rules.
fn valid_result_output_markdown() -> &'static str {
    /*
     * Keep the shared prompt valid and intentionally boring. Task-authority
     * tests should fail only on the JSON or cross-document condition they set
     * up, not on result-output markdown noise from an unrelated fixture.
     */
    r#"# Result Output Prompt

- Summarize the work you actually completed in this turn.
- Mention task-authority updates when they changed.
"#
}

/*
 * Minimal direction catalog for semantic validation cases.
 * The helper keeps direction ids explicit so each task-authority fixture can show whether it is testing a
 * missing relation, an LLM-authored relation note, or graph invariants against a known direction.
 */
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

// Bootstrap output is the golden baseline: generated detail-mode artifacts must pass the same validator.
#[test]
fn bootstrap_artifacts_validate_successfully() {
    /*
     * Bootstrap artifacts are the first planning files many workspaces ever see.
     * Running them through the public service catches drift between generated
     * defaults and the stricter validator used later by promotion and doctor.
     */
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

// Cross-document semantics: every task must stay attached to a known direction catalog entry.
#[test]
fn rejects_unknown_direction_references() {
    /*
     * This is a workspace-level check, not a JSON-shape check: the task document
     * is parseable, but the direction catalog cannot explain where the work
     * belongs. Operators need the TaskAuthority issue code so they know which
     * authority file to edit.
     */
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

// LLM-authored work needs an explicit relation note so later operators can audit why it belongs to a direction.
#[test]
fn rejects_llm_tasks_without_relation_notes() {
    /*
     * User-authored fixtures elsewhere may leave relation notes empty, but LLM
     * proposals need provenance text. This test keeps that stricter policy tied
     * to actor/source semantics rather than a blanket non-empty string rule.
     */
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

// Queue graph semantics reject dependency loops before runtime queue projection chooses executable work.
#[test]
fn rejects_dependency_cycles() {
    /*
     * The runtime queue builder assumes validation has already ruled out cycles.
     * A two-node loop is the smallest fixture that proves graph traversal runs
     * after parsing and before queue projection can mark either task executable.
     */
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

// Version checks happen through semantic validation even when the JSON shape itself is minimal.
#[test]
fn rejects_unsupported_task_authority_version_without_schema_validation() {
    /*
     * A minimal `{version: 2}` payload avoids task-field noise and proves version
     * compatibility is reported as its own semantic code. That distinction lets
     * repair tools decide whether to migrate the document or ask for manual
     * schema repair.
     */
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

// Serde shape validation rejects unknown task fields before semantic checks can treat them as accepted authority.
#[test]
fn rejects_unknown_task_authority_fields() {
    /*
     * Unknown fields are rejected at parse time so future or misspelled authority
     * keys cannot be silently ignored. The expected code stays coarse because
     * adapters only need to tell the operator that JSON decoding failed.
     */
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

/*
 * Multiple domain invariants should accumulate in one report.
 * This protects editor and CLI callers from a fix-one-error-at-a-time loop when task state, dependency,
 * blocker, and in-progress rules are all broken in the same authority document.
 */
#[test]
fn rejects_conflicting_done_relationships_and_multiple_in_progress_tasks() {
    /*
     * This document intentionally violates several independent invariants in a
     * single pass. The validator must keep accumulating report entries so a TUI
     * or CLI repair screen can show the whole damage map at once.
     */
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

// result-output.md must start with a heading because prompt assembly and admin previews read it as a section.
#[test]
fn rejects_result_output_without_heading() {
    /*
     * A non-blank paragraph is still invalid here: prompt assembly treats the
     * file as a named markdown section, so the heading is part of the runtime
     * contract rather than presentation polish.
     */
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

// Placeholder markers are warnings: the document may still be usable, but operators should see edit residue.
#[test]
fn warns_on_result_output_placeholders() {
    /*
     * Placeholder text should not block runtime startup if the rest of the
     * workspace is sound. Keeping this as a warning lets doctor/admin surfaces
     * surface cleanup advice without making generated workspaces unusable.
     */
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

// A blank result-output contract is a hard error because runtime completion copy would have no instruction.
#[test]
fn rejects_blank_result_output_prompt() {
    /*
     * Blank content is stricter than a placeholder warning because the worker
     * would have no completion-output instructions to follow. The fixture uses
     * spaces to prove trimming happens before the hard-error decision.
     */
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

/*
 * Supporting-file validation is separate from validate_workspace_files.
 * These path tests first prove the authority documents parse, then run the extra filesystem-aware check so
 * sandbox failures are attributed to the direction supporting-file contract rather than JSON semantics.
 */
#[test]
fn rejects_detail_doc_paths_that_only_match_prefix_textually() {
    /*
     * Prefix text is not enough for sandbox validation. A sibling directory such
     * as `directions_backup` must not pass just because the string starts with
     * the approved `.codex-exec-loop/planning/directions` characters.
     */
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
    /*
     * Parent traversal is rejected even when the normalized path might land back
     * near the expected tree. The authority document should contain clean,
     * reviewable relative paths rather than relying on filesystem resolution.
     */
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

// Queue-idle prompt paths use the prompts sandbox, distinct from detail-doc direction files.
#[test]
fn rejects_queue_idle_prompt_paths_that_only_match_prefix_textually() {
    /*
     * Queue-idle prompts live under the prompt sandbox, not the direction-detail
     * sandbox. This mirrors the detail-doc prefix test so both path families use
     * component-aware checks instead of starts_with-style filtering.
     */
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
    /*
     * Queue-idle automation can run without a human selecting a task, so its
     * prompt path gets the same traversal guard as direction detail docs. The
     * test keeps the policy local to supporting-file validation.
     */
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
