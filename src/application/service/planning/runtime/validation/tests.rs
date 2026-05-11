use super::PlanningValidationService;
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    PlanningFileKind, PlanningWorkspaceFiles, QueueIdleConfig, QueueIdlePolicy,
};

/*
 * 이 테스트 묶음은 draft promotion, runtime projection, proposal promotion, doctor, reset이
 * 공유하는 application validation boundary를 고정한다. domain validator를 직접 호출하지 않고
 * `PlanningWorkspaceFiles`를 통해 `PlanningValidationService`를 통과시키는 이유는 adapter가
 * operator에게 보여 주는 report code와 동일한 표면을 검증하기 위해서다.
 */

// 공통 성공 fixture는 result-output 계약을 의도적으로 단조롭게 유지한다. task-authority 테스트가
// unrelated markdown noise가 아니라 자신이 만든 JSON/semantic 조건 때문에만 실패하게 한다.
fn valid_result_output_markdown() -> &'static str {
    /*
     * worker completion summary의 최소 계약만 포함한다. heading과 instruction line이 모두
     * 있어서 markdown validator를 통과하지만, placeholder나 path 정책과 엮이지 않는다.
     */
    r#"# Result Output Prompt

- Summarize the work you actually completed in this turn.
- Mention task-authority updates when they changed.
"#
}

/*
 * semantic validation fixture용 최소 direction catalog다. direction id를 호출부가 명시하게
 * 둔 이유는 각 task-authority JSON이 missing relation, worker provenance, graph invariant 중
 * 무엇을 검증하는지 direction 이름만 봐도 드러나게 하려는 것이다.
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

// bootstrap output은 golden baseline이다. detail-mode가 생성한 최초 planning authority도
// promotion/doctor/runtime과 같은 validator를 통과해야 이후 흐름의 기본 전제가 성립한다.
#[test]
fn bootstrap_artifacts_validate_successfully() {
    /*
     * 많은 workspace에서 bootstrap artifact가 첫 authority 문서가 된다. public service로
     * 직접 검증해 generated default와 나중에 쓰이는 엄격한 validation gate 사이의 drift를 잡는다.
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

// cross-document semantic: 모든 task는 catalog에 존재하는 direction에 붙어 있어야 한다.
#[test]
fn rejects_unknown_direction_references() {
    /*
     * JSON shape는 유효하지만 direction catalog가 workstream 소속을 설명하지 못하는
     * workspace-level 실패다. issue file_kind가 TaskAuthority로 남아야 operator가 어떤
     * authority 관계를 고쳐야 하는지 알 수 있다.
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

// worker-authored work는 나중에 operator가 direction 소속 이유를 감사할 수 있도록 relation note가 필요하다.
#[test]
fn rejects_worker_tasks_without_relation_notes() {
    /*
     * user-authored task는 다른 경로에서 relation note가 비어 있을 수 있지만, worker proposal은
     * provenance text가 필요하다. 이 fixture는 단순 non-empty 문자열 규칙이 아니라
     * actor/source semantics에 묶인 stricter policy임을 고정한다.
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
      "description": "Worker adds a new task.",
      "status": "proposed",
      "base_priority": 5,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "worker",
      "last_updated_by": "worker",
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

// queue graph semantic은 runtime queue projection이 executable work를 고르기 전에 dependency loop를 막는다.
#[test]
fn rejects_dependency_cycles() {
    /*
     * runtime queue builder는 validation이 cycle을 이미 제거했다고 가정한다. 두 노드 loop는
     * parsing 이후, queue projection 이전에 graph traversal이 실행된다는 점을 증명하는
     * 가장 작은 fixture다.
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

// version check는 JSON shape가 최소 형태여도 semantic validation의 독립 code로 보고된다.
#[test]
fn rejects_unsupported_task_authority_version_without_schema_validation() {
    /*
     * `{version: 2}`만 둔 payload는 task field noise를 제거한다. version compatibility가
     * 자체 semantic code로 보고되어야 repair tool이 migration 대상인지 수동 schema repair
     * 대상인지 구분할 수 있다.
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

// serde shape validation은 semantic check가 accepted authority로 취급하기 전에 unknown field를 거부한다.
#[test]
fn rejects_unknown_task_authority_fields() {
    /*
     * unknown field는 parse time에 막아 future key나 오탈자 authority key가 조용히 무시되지
     * 않게 한다. adapter에는 JSON decoding 실패라는 coarse code만 필요하므로 세부 serde
     * message를 별도 contract로 만들지 않는다.
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
 * 여러 domain invariant는 하나의 report에 누적되어야 한다. task state, dependency,
 * blocker, in-progress rule이 같은 authority 문서에서 동시에 깨졌을 때 editor/CLI가
 * 한 번에 하나씩만 고치는 반복 루프에 빠지지 않게 하는 계약이다.
 */
#[test]
fn rejects_conflicting_done_relationships_and_multiple_in_progress_tasks() {
    /*
     * 이 문서는 여러 독립 invariant를 의도적으로 동시에 깨뜨린다. validator는 첫 실패에서
     * 멈추지 않고 report entry를 계속 쌓아 TUI/CLI repair 화면이 전체 손상 지도를 보여 주게 한다.
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

// result-output.md는 prompt assembly와 admin preview가 section으로 읽으므로 heading으로 시작해야 한다.
#[test]
fn rejects_result_output_without_heading() {
    /*
     * 빈 문서가 아니어도 heading 없는 paragraph는 invalid다. prompt assembly가 이 파일을
     * named markdown section으로 다루기 때문에 heading은 표시 장식이 아니라 runtime contract다.
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

// placeholder marker는 warning이다. 문서는 사용할 수 있지만 operator에게 편집 잔여물을 보여 줘야 한다.
#[test]
fn warns_on_result_output_placeholders() {
    /*
     * 나머지 workspace가 정상이라면 placeholder text만으로 runtime startup을 막지 않는다.
     * warning으로 유지해 doctor/admin surface가 cleanup advice를 보여 주면서도 generated
     * workspace를 unusable 상태로 만들지 않게 한다.
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

// 빈 result-output contract는 runtime completion copy가 따를 instruction이 없으므로 hard error다.
#[test]
fn rejects_blank_result_output_prompt() {
    /*
     * blank content는 placeholder warning보다 엄격하다. worker가 따를 completion-output
     * instruction 자체가 없기 때문이다. space-only fixture로 hard-error 판단 전에 trim이
     * 적용됨을 고정한다.
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
 * supporting-file validation은 `validate_workspace_files`와 분리되어 있다. path 테스트들은 먼저
 * authority document가 parse/semantic validation을 통과함을 보인 뒤 filesystem-aware check를
 * 추가로 실행한다. 그래야 sandbox failure가 JSON semantics가 아니라 direction supporting-file
 * contract의 문제로 attribution된다.
 */
#[test]
fn rejects_detail_doc_paths_that_only_match_prefix_textually() {
    /*
     * sandbox validation은 문자열 prefix만 보지 않는다. `directions_backup` 같은 sibling directory가
     * 승인된 `.codex-exec-loop/planning/directions` 문자로 시작한다는 이유만으로 통과하면 안 된다.
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
     * parent traversal은 정규화 후 예상 tree 근처로 돌아올 수 있어도 거부된다. authority 문서는
     * filesystem resolution에 기대지 않는 깨끗하고 review 가능한 relative path를 담아야 한다.
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

// queue-idle prompt path는 detail-doc direction file과 다른 prompts sandbox를 사용한다.
#[test]
fn rejects_queue_idle_prompt_paths_that_only_match_prefix_textually() {
    /*
     * queue-idle prompt는 direction-detail sandbox가 아니라 prompt sandbox 아래에 산다.
     * detail-doc prefix 테스트와 같은 형태로 두 path family가 모두 starts_with 방식이 아닌
     * component-aware 검사를 쓴다는 점을 고정한다.
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
     * queue-idle automation은 사람이 task를 직접 고르지 않아도 실행될 수 있다. 따라서 prompt
     * path도 direction detail doc과 같은 traversal guard를 받으며, 이 테스트는 그 정책이
     * supporting-file validation 경계에 머문다는 점을 보여 준다.
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
