use serde_json::Value;

use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY,
};
use crate::application::service::planning::shared::planning_paths::is_valid_planning_markdown_path;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningFileKind, PlanningSemanticValidationService,
    PlanningValidationReport, PlanningValidationResult, PlanningWorkspaceFiles, QueueIdlePolicy,
    TaskAuthorityDocument,
};

/*
 * planning authority가 draft, repair, reset, promotion 결과로 받아들여지기 전 통과하는
 * application-level validation gate다. 이 계층은 workspace contract를 알아야만 판단할 수
 * 있는 규칙, 즉 task authority JSON 구문/serde shape, result-output markdown 구조,
 * direction supporting file sandbox와 존재 여부를 소유한다.
 *
 * task와 direction의 의미 관계 같은 cross-document invariant는 domain validator가 계속
 * 소유한다. 이 서비스가 application 규칙과 domain 규칙을 하나의 `PlanningValidationReport`
 * 안에 모으는 이유는 TUI, doctor, admin preview, runtime snapshot이 서로 다른 에러 형식을
 * 다시 매핑하지 않고 같은 issue code를 보여 주게 하기 위해서다.
 */
#[derive(Default, Clone)]
pub struct PlanningValidationService;

// result-output.md의 template marker는 hard blocker가 아니라 warning이다. operator가 아직
// 문구를 다듬는 중일 수 있으므로 worker output contract 자체가 비어 있거나 깨진 경우와 구분한다.
const PLACEHOLDER_MARKERS: &[&str] = &[
    "{{", "}}", "todo", "tbd", "<replace", "[replace", "<fill", "[fill",
];

impl PlanningValidationService {
    pub fn new() -> Self {
        Self
    }

    pub fn validate_workspace_files(
        &self,
        files: PlanningWorkspaceFiles<'_>,
    ) -> PlanningValidationResult {
        /*
         * validation은 한 번의 pass에서 독립적인 문제를 최대한 모은다. parse failure도 즉시
         * return하지 않고 report에 적재하며, 이후 단계에는 성공적으로 parse된 document만 넘긴다.
         * 이 방식 덕분에 editor/doctor는 syntax, 구조, markdown, semantic 문제를 한 화면에
         * 보여 주면서도 invalid authority를 domain validator에 억지로 먹이지 않는다.
         */
        let mut report = PlanningValidationReport::new();
        let directions = Some(files.directions.clone());
        let task_authority_value =
            self.parse_task_authority_value(files.task_authority_json, &mut report);
        let task_authority = task_authority_value.and_then(|task_authority_value| {
            self.parse_task_authority(task_authority_value, &mut report)
        });
        self.validate_result_output_markdown(files.result_output_markdown, &mut report);
        // semantic validation은 의도적으로 마지막이다. 앞 단계에서 serde domain document로
        // 낮아진 값만 받아 task/direction 관계, graph, version 의미를 검증하게 한다.
        PlanningSemanticValidationService::new().validate(
            directions.as_ref(),
            task_authority.as_ref(),
            &mut report,
        );

        PlanningValidationResult {
            directions,
            task_authority,
            report,
        }
    }

    fn parse_task_authority_value(
        &self,
        task_authority_json: &str,
        report: &mut PlanningValidationReport,
    ) -> Option<Value> {
        // 첫 번째 parse는 raw JSON 구문 문제만 격리한다. 이 단계에서는 versioned task
        // schema를 아직 가정하지 않으므로 operator에게 "문서가 JSON으로도 읽히지 않는다"는
        // 가장 낮은 수준의 실패를 분리해 보고할 수 있다.
        match serde_json::from_str(task_authority_json) {
            Ok(document) => Some(document),
            Err(error) => {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "task_authority_parse_failed",
                    format!("failed to parse task authority: {error}"),
                );
                None
            }
        }
    }

    fn parse_task_authority(
        &self,
        task_authority_value: Value,
        report: &mut PlanningValidationReport,
    ) -> Option<TaskAuthorityDocument> {
        // 두 번째 parse는 구문상 유효한 JSON을 versioned task-authority domain contract로
        // 낮춘다. unknown field나 enum mismatch는 여기서 잡혀 accepted authority로 흘러가지 않는다.
        match serde_json::from_value(task_authority_value) {
            Ok(document) => Some(document),
            Err(error) => {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "task_authority_parse_failed",
                    format!("failed to parse task authority: {error}"),
                );
                None
            }
        }
    }

    pub fn validate_direction_supporting_files<F>(
        &self,
        direction_catalog: &DirectionCatalogDocument,
        mut has_file: F,
        report: &mut PlanningValidationReport,
    ) where
        F: FnMut(&str) -> bool,
    {
        /*
         * detail_doc_path는 direction과 확장 지시문을 이어 runtime prompt assembly가 따라가는
         * 링크다. 여기서는 planning-docs sandbox 규칙과 실제 존재 여부를 함께 검증해 admin
         * authoring이 workspace contract 밖의 파일이나 누락된 파일을 가리키는 direction catalog를
         * promote하지 못하게 한다.
         */
        for direction in &direction_catalog.directions {
            let direction_id = direction.id.trim();
            let detail_doc_path = direction.detail_doc_path.trim();
            if detail_doc_path.is_empty() {
                continue;
            }

            if !is_valid_planning_markdown_path(detail_doc_path, PLANNING_DIRECTION_DOCS_DIRECTORY)
            {
                report.push_error(
                    PlanningFileKind::Directions,
                    "invalid_detail_doc_path",
                    format!(
                        "direction {direction_id} detail_doc_path must point to a markdown file under {PLANNING_DIRECTION_DOCS_DIRECTORY}"
                    ),
                );
                continue;
            }

            if !has_file(detail_doc_path) {
                report.push_error(
                    PlanningFileKind::Directions,
                    "missing_detail_doc_file",
                    format!(
                        "direction {direction_id} detail_doc_path does not exist: {detail_doc_path}"
                    ),
                );
            }
        }

        /*
         * review_and_enqueue는 queue가 비었을 때 hidden worker가 후속 proposal을 만들 수 있게
         * 하는 정책이다. 이때 prompt_path가 없으면 operator가 승인한 지시 원천 없이 proposal을
         * 만들게 되므로, 평소에는 optional인 prompt mapping이 이 정책에서는 mandatory가 된다.
         */
        let prompt_path = direction_catalog.queue_idle.prompt_path.trim();
        if direction_catalog.queue_idle.policy == QueueIdlePolicy::ReviewAndEnqueue
            && prompt_path.is_empty()
        {
            report.push_error(
                PlanningFileKind::Directions,
                "missing_queue_idle_prompt_path",
                format!(
                    "queue_idle.policy=review_and_enqueue requires queue_idle.prompt_path; default path: {DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH}"
                ),
            );
            return;
        }

        if prompt_path.is_empty() {
            return;
        }

        // prompt file은 direction detail과 별도 sandbox를 쓴다. worker prompt assembly가 임의의
        // workspace 파일을 읽지 못하게 하고, queue-idle 지시문을 planning/prompts 아래로 제한한다.
        if !is_valid_planning_markdown_path(prompt_path, PLANNING_PROMPTS_DIRECTORY) {
            report.push_error(
                PlanningFileKind::Directions,
                "invalid_queue_idle_prompt_path",
                format!(
                    "queue_idle.prompt_path must point to a markdown file under {PLANNING_PROMPTS_DIRECTORY}"
                ),
            );
            return;
        }

        if !has_file(prompt_path) {
            report.push_error(
                PlanningFileKind::Directions,
                "missing_queue_idle_prompt_file",
                format!("queue_idle.prompt_path does not exist: {prompt_path}"),
            );
        }
    }

    fn validate_result_output_markdown(
        &self,
        result_output_markdown: &str,
        report: &mut PlanningValidationReport,
    ) {
        /*
         * result-output.md는 완료된 task summary를 어떤 형식으로 남길지 worker에게 알려 주는
         * runtime-facing instruction 파일이다. 빈 파일은 worker output contract가 없는 상태라
         * directions와 task authority가 모두 유효해도 error로 취급한다.
         */
        if result_output_markdown.trim().is_empty() {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "blank_result_output",
                "result-output.md must not be blank",
            );
            return;
        }

        // placeholder warning은 실제 줄 번호가 중요하지만, document shape 판단에서는 빈 줄이
        // heading/instruction 구조를 흐리면 안 된다. 그래서 line number와 trimmed text를 함께 보존한다.
        let non_empty_lines = result_output_markdown
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some((index + 1, trimmed))
                }
            })
            .collect::<Vec<_>>();

        let Some((_, first_line)) = non_empty_lines.first() else {
            return;
        };
        // 첫 줄 heading 요구는 admin preview와 prompt fragment가 같은 section boundary를 기준으로
        // result-output 지시문을 다루게 만드는 작은 markdown contract다.
        if !first_line.starts_with('#') {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "missing_result_output_heading",
                "result-output.md must start with a markdown heading",
            );
        }

        // heading만 있는 파일은 markdown으로는 유효하지만 worker가 따를 instruction contract가 아니다.
        // heading 이후에 최소 한 줄의 실제 지시가 있어야 runtime prompt에 넣을 의미가 생긴다.
        if non_empty_lines
            .iter()
            .skip(1)
            .all(|(_, line)| line.starts_with('#'))
        {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "missing_result_output_instructions",
                "result-output.md must include at least one instruction line after the heading",
            );
        }

        // placeholder marker는 warning으로 남긴다. 예시 문구를 의도적으로 남긴 것인지, 실제 미해결
        // template인지 operator가 판단할 수 있어야 하므로 accepted authority 자체를 막지는 않는다.
        for (line_number, line) in non_empty_lines {
            if let Some(marker) = placeholder_marker(line) {
                report.push_warning(
                    PlanningFileKind::ResultOutput,
                    "result_output_contains_placeholder",
                    format!(
                        "result-output.md contains unresolved placeholder marker {marker:?} on line {line_number}"
                    ),
                );
            }
        }
    }
}

fn placeholder_marker(line: &str) -> Option<&'static str> {
    // 대소문자 차이는 같은 template 위험으로 본다. TODO/todo/TBD 변형을 하나의 warning
    // code로 접어 adapter가 marker별 분기 없이 같은 repair guidance를 낼 수 있게 한다.
    let normalized = line.to_ascii_lowercase();
    PLACEHOLDER_MARKERS
        .iter()
        .copied()
        .find(|marker| normalized.contains(marker))
}

#[cfg(test)]
mod tests;
