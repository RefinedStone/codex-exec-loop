use crate::application::port::outbound::planning_workspace_port::PlanningDraftFileRecord;
use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    QueueIdleConfig, QueueIdlePolicy, TaskAuthorityDocument,
};

const DEFAULT_RESULT_OUTPUT_MARKDOWN: &str = r#"# Result Output Prompt

- Summarize the work you actually completed in this turn.
- If you updated task authority, mention which tasks changed and why.
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
pub struct PlanningBootstrapSupplementalFile {
    pub active_path: String,
    pub body: String,
}

impl From<PlanningBootstrapSupplementalFile> for PlanningDraftFileRecord {
    fn from(value: PlanningBootstrapSupplementalFile) -> Self {
        Self {
            active_path: value.active_path,
            body: value.body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningBootstrapArtifacts {
    pub directions: DirectionCatalogDocument,
    pub task_authority: TaskAuthorityDocument,
    pub result_output_path: String,
    pub result_output_markdown: String,
    pub supplemental_files: Vec<PlanningBootstrapSupplementalFile>,
}

impl PlanningBootstrapService {
    pub fn new() -> Self {
        Self
    }

    pub fn build_artifacts_for_mode(
        &self,
        mode: PlanningBootstrapMode,
    ) -> PlanningBootstrapArtifacts {
        let directions = directions_for_mode(mode);
        let supplemental_files = match mode {
            PlanningBootstrapMode::Detail => Vec::new(),
            PlanningBootstrapMode::Simple => vec![PlanningBootstrapSupplementalFile {
                active_path: DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
                body: DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string(),
            }],
        };

        PlanningBootstrapArtifacts {
            directions,
            task_authority: TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: Vec::new(),
            },
            result_output_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            result_output_markdown: DEFAULT_RESULT_OUTPUT_MARKDOWN.to_string(),
            supplemental_files,
        }
    }
}

fn directions_for_mode(mode: PlanningBootstrapMode) -> DirectionCatalogDocument {
    match mode {
        PlanningBootstrapMode::Detail => DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![DirectionDefinition {
                id: "example-direction".to_string(),
                title: "Example direction".to_string(),
                summary: "Replace this example with the real macro direction for the workspace."
                    .to_string(),
                success_criteria: vec![
                    "Replace the placeholder direction with a real operator-defined direction."
                        .to_string(),
                ],
                scope_hints: vec![
                    "Add loose hints that help relate future tasks to this direction.".to_string(),
                ],
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        },
        PlanningBootstrapMode::Simple => DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::ReviewAndEnqueue,
                prompt_path: DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            },
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General workstream".to_string(),
                summary: "No detailed direction taxonomy is defined yet. Derive the next actionable work from the latest user request and the latest accepted answer, capture it in DB task authority, and work from the derived queue.".to_string(),
                success_criteria: vec![
                    "Actionable goals are represented in DB task authority before execution."
                        .to_string(),
                    "When the latest answer clearly implies a next step, that follow-up is derived into task authority instead of leaving the queue idle.".to_string(),
                    "Work advances by updating task authority instead of inventing unmanaged side tasks."
                        .to_string(),
                ],
                scope_hints: vec![
                    "Use this generic direction until the operator replaces it with a richer direction catalog."
                        .to_string(),
                    "Represent concrete next actions and proposals in accepted task authority."
                        .to_string(),
                    "If the user asked for a multi-step artifact, convert the next obvious step from the latest answer into a queued task.".to_string(),
                ],
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{PlanningBootstrapMode, PlanningBootstrapService};
    use crate::application::service::planning::shared::contract::DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH;
    use crate::domain::planning::{DirectionState, PLANNING_FORMAT_VERSION, QueueIdlePolicy};

    #[test]
    fn bootstrap_artifacts_use_expected_paths_and_versioned_contracts() {
        let service = PlanningBootstrapService::new();
        let artifacts = service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);

        assert!(artifacts.result_output_path.ends_with("result-output.md"));
        assert_eq!(artifacts.directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(artifacts.task_authority.version, PLANNING_FORMAT_VERSION);
    }

    #[test]
    fn bootstrap_direction_catalog_remains_readable() {
        let service = PlanningBootstrapService::new();
        let artifacts = service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let directions = artifacts.directions;

        assert_eq!(directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(directions.directions.len(), 1);
        assert_eq!(directions.directions[0].state, DirectionState::Active);
    }

    #[test]
    fn simple_mode_artifacts_use_generic_catch_all_direction() {
        let service = PlanningBootstrapService::new();
        let artifacts = service.build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        let directions = artifacts.directions;

        assert_eq!(directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(directions.directions.len(), 1);
        assert_eq!(directions.directions[0].id, "general-workstream");
        assert_eq!(directions.directions[0].state, DirectionState::Active);
        assert_eq!(
            directions.queue_idle.policy,
            QueueIdlePolicy::ReviewAndEnqueue
        );
        assert_eq!(
            directions.queue_idle.prompt_path,
            DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
        );
        assert!(
            directions.directions[0]
                .summary
                .contains("DB task authority")
        );
        assert_eq!(artifacts.supplemental_files.len(), 1);
        assert_eq!(
            artifacts.supplemental_files[0].active_path,
            DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
        );
        assert!(!artifacts.supplemental_files[0].body.trim().is_empty());
    }
}
