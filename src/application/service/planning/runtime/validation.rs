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
학습 주석: PlanningValidationService는 planning draft를 실제 authority/runtime 입력으로 승격하기 전
파일 묶음의 "문법, 구조, 연결 파일" 경계를 확인하는 application service입니다. authoring init,
proposal promotion, repair/reset, directions doctor가 모두 같은 service를 쓰기 때문에 여기의
오류 코드는 admin UI와 CLI report의 공통 계약이 됩니다.

도메인 계층의 `PlanningSemanticValidationService`는 directions와 task authority 사이의 의미 관계를
검증하고, 이 파일은 JSON parse, result-output markdown shape, detail doc/prompt file 존재 여부처럼
workspace 파일 시스템과 contract path가 필요한 검증을 맡습니다.
*/
#[derive(Default, Clone)]
pub struct PlanningValidationService;

// 학습 주석: result-output.md는 worker에게 전달되는 출력 계약이므로 템플릿 marker가 남아
// 있어도 promotion을 완전히 막지는 않고 warning으로 노출합니다.
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
        // 학습 주석: validation은 가능한 한 많은 문제를 한 번에 보고해야 draft editor와 doctor가
        // 사용자에게 한 항목씩 재시도시키지 않습니다. 그래서 parse 실패도 report에 누적하고
        // 이후 단계는 available document만 넘기는 방식으로 진행합니다.
        let mut report = PlanningValidationReport::new();
        let directions = Some(files.directions.clone());
        let task_authority_value =
            self.parse_task_authority_value(files.task_authority_json, &mut report);
        let task_authority = task_authority_value.and_then(|task_authority_value| {
            self.parse_task_authority(task_authority_value, &mut report)
        });
        self.validate_result_output_markdown(files.result_output_markdown, &mut report);
        // 학습 주석: semantic validation은 parse가 성공한 authority만 받습니다. 이렇게 하면 JSON
        // 구조 오류와 cross-file 의미 오류를 같은 report에 담되, 깨진 authority를 도메인 validator에
        // 억지로 넘기지 않습니다.
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
        // 학습 주석: 첫 parse는 raw JSON syntax를 분리해서 잡습니다. syntax가 깨진 경우에는
        // schema/serde 구조 오류보다 먼저 "파일을 JSON으로 읽을 수 없음"을 보고해야 합니다.
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
        // 학습 주석: 두 번째 parse는 JSON value를 domain document shape로 내립니다. 이 단계의
        // 실패는 파일이 JSON이더라도 required field/type contract를 만족하지 않는다는 뜻입니다.
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
        // 학습 주석: catalog 안의 detail_doc_path는 runtime prompt assembly가 direction별 세부
        // 지시를 읽는 연결점입니다. path sandbox와 실제 파일 존재를 함께 확인해 admin authoring이
        // 잘못된 markdown link를 authority로 승격하지 못하게 합니다.
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

        // 학습 주석: queue-idle review_and_enqueue 정책은 worker가 idle 상태에서 prompt를 읽어
        // 후보 작업을 검토하는 mode입니다. 그래서 policy가 켜진 경우 prompt path는 optional이 아니라
        // runtime contract의 필수 파일 경로가 됩니다.
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

        // 학습 주석: prompt path도 detail doc과 같은 sandbox rule을 따릅니다. 외부 파일이나
        // 비-markdown 파일을 허용하면 worker prompt assembly가 planning workspace 밖의 내용을
        // 읽는 경로가 생깁니다.
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
        // 학습 주석: result-output.md는 task result를 어떤 markdown shape로 남길지 정의하는
        // runtime-facing instruction file입니다. 빈 파일이면 worker가 완료 산출물의 형식을 알 수 없습니다.
        if result_output_markdown.trim().is_empty() {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "blank_result_output",
                "result-output.md must not be blank",
            );
            return;
        }

        // 학습 주석: line number를 보존해 placeholder warning이 admin UI와 CLI에서 바로 수정 가능한
        // 위치를 가리키게 합니다. 비어 있는 줄은 문서 구조 판단에서는 제외합니다.
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
        // 학습 주석: 첫 의미 줄을 heading으로 강제하면 result-output 문서가 admin preview와
        // prompt fragment에서 같은 section 단위로 읽힙니다.
        if !first_line.starts_with('#') {
            report.push_error(
                PlanningFileKind::ResultOutput,
                "missing_result_output_heading",
                "result-output.md must start with a markdown heading",
            );
        }

        // 학습 주석: heading만 있는 파일은 markdown으로는 유효해 보여도 worker에게 실제 출력
        // 지시를 주지 못합니다. 최소 한 줄의 non-heading instruction을 요구합니다.
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

        // 학습 주석: placeholder는 아직 편집 중인 문서의 신호라 promotion을 error로 막지는 않습니다.
        // warning으로 남겨 operator가 의도적으로 남긴 예시와 실수로 남긴 템플릿 marker를 구분하게 합니다.
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
    // 학습 주석: marker detection은 대소문자를 무시해 TODO/todo/TBD 같은 흔한 템플릿
    // 잔여물을 같은 warning code로 모읍니다.
    let normalized = line.to_ascii_lowercase();
    PLACEHOLDER_MARKERS
        .iter()
        .copied()
        .find(|marker| normalized.contains(marker))
}

#[cfg(test)]
mod tests;
