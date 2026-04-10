use serde_json::json;

use crate::domain::planning::{
    DIRECTIONS_FILE_PATH, PLANNING_FORMAT_VERSION, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH, TaskLedgerDocument,
};

const DEFAULT_DIRECTIONS_TOML: &str = r#"version = 1

[[directions]]
id = "example-direction"
title = "Example direction"
summary = "Replace this example with the real macro direction for the workspace."
success_criteria = [
    "Replace the placeholder direction with a real operator-defined direction.",
]
scope_hints = [
    "Add loose hints that help relate future tasks to this direction.",
]
state = "active"
"#;

const SIMPLE_MODE_DIRECTIONS_TOML: &str = r#"version = 1

[[directions]]
id = "general-workstream"
title = "General workstream"
summary = "No detailed direction taxonomy is defined yet. Put every actionable goal or accepted proposal into task-ledger.json and work from that queue."
success_criteria = [
    "Actionable goals are represented in task-ledger.json before execution.",
    "Work advances by updating the task ledger instead of inventing unmanaged side tasks.",
]
scope_hints = [
    "Use this generic direction until the operator replaces it with a richer direction catalog.",
    "Treat task-ledger.json as the source of truth for concrete next actions and proposals.",
]
state = "active"
"#;

const DEFAULT_RESULT_OUTPUT_MARKDOWN: &str = r#"# Result Output Prompt

- Summarize the work you actually completed in this turn.
- If you updated `task-ledger.json`, mention which tasks changed and why.
- Do not claim unrelated work was added when it was rejected by validation.
"#;

#[derive(Default, Clone)]
pub struct PlanningBootstrapService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningBootstrapMode {
    Detail,
    Simple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningBootstrapArtifacts {
    pub directions_path: String,
    pub directions_toml: String,
    pub task_ledger_path: String,
    pub task_ledger_json: String,
    pub task_ledger_schema_path: String,
    pub task_ledger_schema_json: String,
    pub result_output_path: String,
    pub result_output_markdown: String,
}

impl PlanningBootstrapService {
    pub fn new() -> Self {
        Self
    }

    pub fn build_artifacts(&self) -> PlanningBootstrapArtifacts {
        self.build_artifacts_for_mode(PlanningBootstrapMode::Detail)
    }

    pub fn build_artifacts_for_mode(
        &self,
        mode: PlanningBootstrapMode,
    ) -> PlanningBootstrapArtifacts {
        let directions_toml = match mode {
            PlanningBootstrapMode::Detail => DEFAULT_DIRECTIONS_TOML,
            PlanningBootstrapMode::Simple => SIMPLE_MODE_DIRECTIONS_TOML,
        };

        PlanningBootstrapArtifacts {
            directions_path: DIRECTIONS_FILE_PATH.to_string(),
            directions_toml: directions_toml.to_string(),
            task_ledger_path: TASK_LEDGER_FILE_PATH.to_string(),
            task_ledger_json: serde_json::to_string_pretty(&TaskLedgerDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: Vec::new(),
            })
            .expect("bootstrap task ledger should serialize"),
            task_ledger_schema_path: TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
            task_ledger_schema_json: serde_json::to_string_pretty(&json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "title": "Task Ledger",
                "type": "object",
                "required": ["version", "tasks"],
                "properties": {
                    "version": {
                        "type": "integer",
                        "const": PLANNING_FORMAT_VERSION,
                    },
                    "tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": [
                                "id",
                                "direction_id",
                                "title",
                                "description",
                                "status",
                                "base_priority",
                                "created_by",
                                "last_updated_by",
                                "updated_at"
                            ],
                            "properties": {
                                "id": { "type": "string" },
                                "direction_id": { "type": "string" },
                                "direction_relation_note": { "type": "string" },
                                "title": { "type": "string" },
                                "description": { "type": "string" },
                                "status": {
                                    "type": "string",
                                    "enum": [
                                        "ready",
                                        "blocked",
                                        "in_progress",
                                        "done",
                                        "cancelled",
                                        "awaiting_user",
                                        "proposed"
                                    ]
                                },
                                "base_priority": { "type": "integer" },
                                "dynamic_priority_delta": { "type": "integer" },
                                "priority_reason": { "type": "string" },
                                "depends_on": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "blocked_by": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "created_by": {
                                    "type": "string",
                                    "enum": ["user", "llm", "system"]
                                },
                                "last_updated_by": {
                                    "type": "string",
                                    "enum": ["user", "llm", "system"]
                                },
                                "source_turn_id": { "type": ["string", "null"] },
                                "updated_at": { "type": "string" }
                            }
                        }
                    }
                }
            }))
            .expect("bootstrap task-ledger schema should serialize"),
            result_output_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            result_output_markdown: DEFAULT_RESULT_OUTPUT_MARKDOWN.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PlanningBootstrapMode, PlanningBootstrapService};
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionState, PLANNING_FORMAT_VERSION,
    };

    #[test]
    fn bootstrap_artifacts_use_expected_paths_and_versioned_contracts() {
        let service = PlanningBootstrapService::new();
        let artifacts = service.build_artifacts();

        assert!(artifacts.directions_path.ends_with("directions.toml"));
        assert!(artifacts.task_ledger_path.ends_with("task-ledger.json"));
        assert!(
            artifacts
                .task_ledger_schema_path
                .ends_with("task-ledger.schema.json")
        );
        assert!(artifacts.result_output_path.ends_with("result-output.md"));
        assert!(
            artifacts
                .task_ledger_json
                .contains(&format!("\"version\": {PLANNING_FORMAT_VERSION}"))
        );
    }

    #[test]
    fn bootstrap_direction_catalog_remains_readable() {
        let service = PlanningBootstrapService::new();
        let directions: DirectionCatalogDocument =
            toml::from_str(service.build_artifacts().directions_toml.as_str())
                .expect("bootstrap directions should parse");

        assert_eq!(directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(directions.directions.len(), 1);
        assert_eq!(directions.directions[0].state, DirectionState::Active);
    }

    #[test]
    fn simple_mode_artifacts_use_generic_catch_all_direction() {
        let service = PlanningBootstrapService::new();
        let directions: DirectionCatalogDocument = toml::from_str(
            service
                .build_artifacts_for_mode(PlanningBootstrapMode::Simple)
                .directions_toml
                .as_str(),
        )
        .expect("simple mode directions should parse");

        assert_eq!(directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(directions.directions.len(), 1);
        assert_eq!(directions.directions[0].id, "general-workstream");
        assert_eq!(directions.directions[0].state, DirectionState::Active);
        assert!(
            directions.directions[0]
                .summary
                .contains("task-ledger.json")
        );
    }
}
