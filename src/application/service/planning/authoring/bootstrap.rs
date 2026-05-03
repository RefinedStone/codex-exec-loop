use crate::application::port::outbound::planning_workspace_port::PlanningDraftFileRecord;
use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    QueueIdleConfig, QueueIdlePolicy, TaskAuthorityDocument,
};

/*
 * Bootstrap artifacts are the planning subsystem's first durable contract.
 * Init, reset, validation tests, and default authority seeding all call through
 * this service so a fresh workspace receives the same direction catalog,
 * task-authority envelope, result-output prompt, and optional supporting files.
 */
const DEFAULT_RESULT_OUTPUT_MARKDOWN: &str = r#"# Result Output Prompt

- Summarize the work you actually completed in this turn.
- If you updated task authority, mention which tasks changed and why.
- Do not claim unrelated work was added when it was rejected by validation.
"#;

#[derive(Default, Clone)]
pub struct PlanningBootstrapService;

/*
 * Detail mode starts with an operator-editable taxonomy placeholder. Simple
 * mode starts with a catch-all direction and enables the queue-idle review
 * prompt so the app can derive follow-up tasks without a curated catalog yet.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningBootstrapMode {
    Detail,
    Simple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningBootstrapSupplementalFile {
    // Supplemental files are workspace files that must be created alongside the
    // core authority documents, but are not embedded in those JSON documents.
    pub active_path: String,
    pub body: String,
}

impl From<PlanningBootstrapSupplementalFile> for PlanningDraftFileRecord {
    fn from(value: PlanningBootstrapSupplementalFile) -> Self {
        // Workspace draft staging uses the same path/body shape, so conversion
        // keeps bootstrap extras reusable by init and draft-promotion flows.
        Self {
            active_path: value.active_path,
            body: value.body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningBootstrapArtifacts {
    // The two authority documents are intentionally created together: direction
    // authority gives tasks a routing taxonomy, while task authority starts
    // empty and later records accepted queue work.
    pub directions: DirectionCatalogDocument,
    pub task_authority: TaskAuthorityDocument,
    // Result-output remains a markdown file because the runtime prompt is
    // edited and validated through the workspace-file boundary, not the DB
    // authority repository.
    pub result_output_path: String,
    pub result_output_markdown: String,
    // Supporting files are mode-dependent. Simple mode needs the auto-follow
    // prompt immediately; detail mode leaves that choice to the operator.
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
        // Keep mode selection centralized here so every caller seeds the same
        // format version, default prompts, and queue-idle policy.
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
            // Task authority starts empty even in Simple mode. The queue-idle
            // evaluator may later derive tasks from completed turns, but seed
            // state should never pretend work has already been accepted.
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
                // The detail bootstrap is deliberately a visible placeholder:
                // it gives the manual editor a valid schema while making it
                // obvious that real project directions still need authoring.
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
            // Simple mode is optimized for native-first startup: the operator
            // can begin with one broad workstream and let the post-turn review
            // prompt derive explicit queue items as evidence accumulates.
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::ReviewAndEnqueue,
                prompt_path: DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            },
            directions: vec![DirectionDefinition {
                // This catch-all direction is a bridge, not a final taxonomy.
                // It keeps validation and queue projection usable until the
                // admin direction tools replace it with richer project slices.
                id: "general-workstream".to_string(),
                title: "General workstream".to_string(),
                summary: "No detailed direction taxonomy is defined yet. After each main result, evaluate the latest user request and accepted answer against this generic direction, capture the next queue-driven task in DB task authority, and work from the derived queue.".to_string(),
                success_criteria: vec![
                    "Actionable goals are represented in DB task authority as queue-driven execution slices."
                        .to_string(),
                    "When the latest request and main result leave a clear follow-up, gap, or verification need, that next task is derived into task authority instead of leaving the queue idle.".to_string(),
                    "Work advances by updating task authority instead of inventing unmanaged side tasks."
                        .to_string(),
                ],
                scope_hints: vec![
                    "Use this generic direction until the operator replaces it with a richer direction catalog."
                        .to_string(),
                    "Represent concrete next actions and proposals in accepted task authority."
                        .to_string(),
                    "If the user asked for a multi-step artifact, evaluate the latest main result and queue the next concrete slice only when the follow-up is clear.".to_string(),
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

        // These assertions pin the shared seed contract used by workspace init,
        // authority seeding, reset, and validation fixtures.
        assert!(artifacts.result_output_path.ends_with("result-output.md"));
        assert_eq!(artifacts.directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(artifacts.task_authority.version, PLANNING_FORMAT_VERSION);
    }

    #[test]
    fn bootstrap_direction_catalog_remains_readable() {
        let service = PlanningBootstrapService::new();
        let artifacts = service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let directions = artifacts.directions;

        // Detail mode should be immediately valid and inspectable in the manual
        // editor even though the placeholder must later be replaced.
        assert_eq!(directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(directions.directions.len(), 1);
        assert_eq!(directions.directions[0].state, DirectionState::Active);
    }

    #[test]
    fn simple_mode_artifacts_use_generic_catch_all_direction() {
        let service = PlanningBootstrapService::new();
        let artifacts = service.build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        let directions = artifacts.directions;

        // Simple mode must preserve the auto-follow lane: one active direction,
        // review-and-enqueue policy, and a prompt file that tells the evaluator
        // how to derive follow-up work after each completed turn.
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
                .contains("After each main result")
        );
        assert!(
            directions.directions[0]
                .success_criteria
                .iter()
                .any(|criterion| { criterion.contains("follow-up, gap, or verification need") })
        );
        assert!(
            directions.directions[0]
                .scope_hints
                .iter()
                .any(|hint| hint.contains("evaluate the latest main result"))
        );
        assert_eq!(artifacts.supplemental_files.len(), 1);
        assert_eq!(
            artifacts.supplemental_files[0].active_path,
            DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
        );
        assert!(
            artifacts.supplemental_files[0]
                .body
                .contains("post-turn planning evaluator")
        );
        assert!(
            artifacts.supplemental_files[0]
                .body
                .contains("완료 authority가 아닙니다")
        );
        assert!(
            artifacts.supplemental_files[0]
                .body
                .contains("명시 TODO가 없어도")
        );
    }
}
