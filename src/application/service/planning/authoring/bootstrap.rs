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
 * bootstrap artifact는 planning subsystem의 첫 durable contract다. init, reset, validation fixture, default
 * authority seed가 모두 이 service를 통과하므로 새 workspace와 복구된 workspace가 같은 direction catalog,
 * task-authority envelope, result-output prompt, supporting file baseline을 받는다. 이 파일을 바꾸면 단순
 * 기본값 수정이 아니라 "planning이 처음 무엇을 권위 상태로 인정하는가"를 바꾸는 일이 된다.
 */
// result-output prompt는 worker가 turn 결과를 설명하는 최소 보고 계약이다. 이 문구가 active workspace 파일로
// seed되기 때문에 validation, runtime prompt, reset all이 모두 같은 "무엇을 완료로 보고할 것인가" 기준을 공유한다.
const DEFAULT_RESULT_OUTPUT_MARKDOWN: &str = r#"# Result Output Prompt

- Summarize the work you actually completed in this turn.
- If you updated task authority, mention which tasks changed and why.
- Do not claim unrelated work was added when it was rejected by validation.
"#;

#[derive(Default, Clone)]
// service 자체는 상태를 갖지 않는다. 상태 없음은 init/reset/test fixture가 같은 생성 규칙을 값 복사 없이 재사용하게 하는
// 의도이며, bootstrap 결과의 차이는 오직 `PlanningBootstrapMode` 입력으로만 갈라진다.
pub struct PlanningBootstrapService;

/*
 * Detail mode는 operator가 직접 taxonomy를 작성하도록 placeholder catalog에서 시작한다. Simple mode는 broad
 * catch-all direction과 queue-idle review prompt를 켜서, curated catalog가 없더라도 turn 결과에서 follow-up
 * task를 도출할 수 있게 한다. 두 mode는 같은 schema를 쓰지만 운영 철학이 다르다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningBootstrapMode {
    // Detail mode는 schema-valid placeholder만 제공한다. operator가 project taxonomy를 직접 작성하는 흐름을 우선한다.
    Detail,
    // Simple mode는 native-first 자동 후속 흐름을 즉시 사용할 수 있게 generic direction과 queue-idle prompt를 함께 만든다.
    Simple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningBootstrapSupplementalFile {
    // supplemental file은 core authority document와 함께 생성되어야 하는 workspace file이다. JSON authority 안에
    // body를 embed하지 않는 이유는 prompt/detail 문서가 operator-editable markdown boundary에 남아야 하기 때문이다.
    pub active_path: String,
    pub body: String,
}

impl From<PlanningBootstrapSupplementalFile> for PlanningDraftFileRecord {
    fn from(value: PlanningBootstrapSupplementalFile) -> Self {
        // workspace draft staging도 active_path/body shape를 사용한다. 변환을 제공하면 bootstrap supplemental file을
        // init, draft promotion, admin editor staging에서 같은 자료형 계열로 재사용할 수 있다.
        Self {
            active_path: value.active_path,
            body: value.body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningBootstrapArtifacts {
    // 두 authority document는 함께 생성된다. direction authority는 task가 속할 routing taxonomy를 제공하고,
    // task authority는 빈 envelope에서 시작해 이후 accepted queue work를 기록한다.
    pub directions: DirectionCatalogDocument,
    pub task_authority: TaskAuthorityDocument,
    // result-output은 DB authority가 아니라 workspace markdown file로 남는다. runtime prompt는 operator가 직접
    // 읽고 고치는 문서이고 validation도 workspace-file boundary를 통해 수행되기 때문이다.
    pub result_output_path: String,
    pub result_output_markdown: String,
    // supporting file은 mode-dependent다. Simple mode는 auto-follow prompt가 즉시 필요하고, Detail mode는 operator가
    // 어떤 supporting doc/prompt를 둘지 직접 정하도록 비워 둔다.
    pub supplemental_files: Vec<PlanningBootstrapSupplementalFile>,
}

impl PlanningBootstrapService {
    pub fn new() -> Self {
        Self
    }

    /*
     * 모든 bootstrap caller가 이 method 하나로 산출물을 받는다.
     * 여기서 file path, format version, queue-idle prompt 존재 여부를 함께 결정해야 init으로 만든 workspace와
     * reset으로 되살린 workspace가 나중에 runtime prompt loader에서 같은 형태로 읽힌다.
     */
    pub fn build_artifacts_for_mode(
        &self,
        mode: PlanningBootstrapMode,
    ) -> PlanningBootstrapArtifacts {
        // mode selection은 여기서 중앙화한다. caller가 init인지 reset인지 authority seed인지에 따라 format version,
        // default prompt, queue-idle policy가 달라지면 같은 workspace도 진입 경로별로 다른 planning baseline을 갖게 된다.
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
            // task authority는 Simple mode에서도 빈 상태로 시작한다. queue-idle evaluator가 이후 completed turn에서
            // task를 도출할 수는 있지만, seed state가 이미 accepted work가 있는 것처럼 보이면 안 된다.
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

/*
 * direction catalog seed는 queue projection의 출발점이다.
 * task authority가 비어 있어도 direction catalog는 반드시 valid해야 한다. runtime은 이 catalog를 기준으로
 * "지금 queue가 비었는지"뿐 아니라 "새 task가 어떤 방향에 속해야 하는지"를 판단한다.
 */
fn directions_for_mode(mode: PlanningBootstrapMode) -> DirectionCatalogDocument {
    match mode {
        PlanningBootstrapMode::Detail => DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![DirectionDefinition {
                // detail bootstrap은 의도적으로 눈에 띄는 placeholder다. manual editor가 곧바로 valid schema를 열 수
                // 있게 하면서도, 실제 project direction authoring이 아직 필요하다는 사실을 숨기지 않는다.
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
            // Simple mode는 native-first startup에 맞춘다. operator가 broad workstream 하나로 시작하고, turn 결과가
            // 쌓이면 post-turn review prompt가 명시 queue item을 도출해 task authority에 반영한다.
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::ReviewAndEnqueue,
                prompt_path: DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            },
            directions: vec![DirectionDefinition {
                // catch-all direction은 최종 taxonomy가 아니라 bridge다. admin direction tool로 더 풍부한 project
                // slice가 들어오기 전까지 validation과 queue projection을 사용 가능하게 유지한다.
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

        // 이 assertion은 workspace init, authority seeding, reset, validation fixture가 공유하는 seed contract를
        // 고정한다. path/version이 흔들리면 여러 진입점의 bootstrap 결과가 동시에 달라진다.
        // path는 adapter-facing 파일 contract이고 version은 domain parser contract라 둘을 같은 테스트에서 묶는다.
        assert!(artifacts.result_output_path.ends_with("result-output.md"));
        assert_eq!(artifacts.directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(artifacts.task_authority.version, PLANNING_FORMAT_VERSION);
    }

    #[test]
    fn bootstrap_direction_catalog_remains_readable() {
        let service = PlanningBootstrapService::new();
        let artifacts = service.build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let directions = artifacts.directions;

        // Detail mode는 placeholder를 나중에 교체해야 하지만, 처음부터 manual editor에서 inspect 가능한 valid
        // direction catalog여야 한다.
        // 즉 "아직 설계가 비어 있음"과 "schema가 깨짐"을 구분해야 doctor/init UX가 불필요한 repair로 흐르지 않는다.
        assert_eq!(directions.version, PLANNING_FORMAT_VERSION);
        assert_eq!(directions.directions.len(), 1);
        assert_eq!(directions.directions[0].state, DirectionState::Active);
    }

    #[test]
    fn simple_mode_artifacts_use_generic_catch_all_direction() {
        let service = PlanningBootstrapService::new();
        let artifacts = service.build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        let directions = artifacts.directions;

        // Simple mode는 auto-follow lane을 보존해야 한다. 하나의 active direction, review-and-enqueue policy,
        // completed turn 뒤 follow-up work를 도출하는 prompt file이 함께 있어야 한다.
        // 이 세 요소 중 하나라도 빠지면 queue가 비었을 때 runtime은 다음 작업을 제안할 근거를 잃는다.
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
