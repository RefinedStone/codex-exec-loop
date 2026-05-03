/*
 * Directions doctor는 operator가 직접 고친 direction catalog와 supporting markdown file 사이의 불일치를 복구하는
 * service flow다. 이 flow는 validation error를 단순히 보고하는 데서 끝나지 않고, 안전하게 자동 복구할 수 있는
 * path mapping과 missing default file만 고친다. catalog 안의 mapping을 canonical planning 경로로 정리하고,
 * 누락된 detail doc/queue-idle prompt file을 기본 body로 생성한 뒤 validation을 다시 돌려 "자동 복구 후에도 남은
 * 문제"를 outcome에 담는다.
 */
use anyhow::{Result, anyhow};

use super::supporting_files::{
    build_default_detail_doc_markdown, default_validated_direction_detail_doc_path,
};
use super::{PlanningDirectionsService, PlanningDoctorOutcome, trimmed_non_empty};
use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY,
};
use crate::application::service::planning::shared::planning_paths::is_valid_planning_markdown_path;
use crate::domain::planning::{PlanningValidationReport, PlanningWorkspaceFiles, QueueIdlePolicy};

impl PlanningDirectionsService {
    #[allow(dead_code)]
    // doctor_workspace는 direction authority를 "검사 후 필요한 최소 복구만 적용"하는 entrypoint다. catalog 수정과
    // file 생성은 즉시 쓰지 않고 staging map/counter에 모았다가, authority commit과 file write를 순서대로 한 번씩
    // 수행한다. 마지막 validation report는 doctor가 자동 복구한 뒤에도 operator가 봐야 할 남은 문제를 알려 준다.
    pub fn doctor_workspace(&self, workspace_dir: &str) -> Result<PlanningDoctorOutcome> {
        // complete workspace는 현재 direction catalog와 supporting files를 판단할 기준점이다. directions는 수정 가능한
        // 사본으로 분리해, 실제 mapping 변경이 있을 때만 commit하고 file 생성만 필요한 doctor run에서는 revision을 올리지 않는다.
        let workspace = self.load_complete_workspace(workspace_dir)?;
        let mut directions = workspace.directions.clone();
        // outcome counters는 doctor가 실제로 고친 layer를 admin/TUI가 설명할 수 있게 하는 audit 값이다. mapping repair와
        // file creation을 나누어야 "authority를 고쳤는지"와 "workspace markdown을 만들었는지"가 드러난다.
        let mut repaired_detail_doc_mappings = 0;
        let mut created_detail_doc_files = 0;
        let mut repaired_queue_idle_prompt_mapping = false;
        let mut created_queue_idle_prompt_file = false;
        // supporting file 생성 요청은 path별로 모아 둔다. 같은 target path를 여러 direction이 공유해도 insert 결과로
        // 중복 생성을 피하고 counter를 한 번만 올릴 수 있다. 실제 file write는 catalog commit 이후에 수행한다.
        let mut pending_supporting_files = std::collections::HashMap::<String, String>::new();

        // 각 direction의 detail_doc_path는 `docs/planning/directions/...md` 안쪽의 유효 markdown이어야 한다. 비어 있거나
        // 잘못된 path는 direction id에서 파생한 기본 path로 되돌린다. 이 복구는 path traversal, 잘못된 확장자, legacy
        // 위치를 canonical planning directory로 되돌리는 authority repair다.
        for direction in directions.directions.clone() {
            let configured_path = trimmed_non_empty(direction.detail_doc_path.as_str());
            let target_path = if configured_path.is_some_and(|path| {
                is_valid_planning_markdown_path(path, PLANNING_DIRECTION_DOCS_DIRECTORY)
            }) {
                configured_path.expect("checked above").to_string()
            } else {
                default_validated_direction_detail_doc_path(&direction.id)?
            };

            if configured_path != Some(target_path.as_str()) {
                // catalog mapping을 고치는 것은 file 생성과 별개다. direction 문서가 어디에 있어야 하는지 authority부터
                // canonical하게 만든 뒤, 아래에서 file 존재 여부를 채운다. 이렇게 해야 validation과 runtime prompt fragment가
                // 같은 canonical path를 source of truth로 본다.
                super::set_direction_detail_doc_path(&mut directions, &direction.id, &target_path)?;
                repaired_detail_doc_mappings += 1;
            }

            if self
                .load_supporting_file_best_effort(workspace_dir, &target_path)
                .is_none()
                && pending_supporting_files
                    .insert(
                        target_path.clone(),
                        build_default_detail_doc_markdown(&direction),
                    )
                    .is_none()
            {
                // detail doc이 없으면 direction metadata에서 기본 markdown scaffold를 만든다. best-effort load가 None인
                // 경우만 생성해, 이미 operator가 작성한 file을 doctor가 덮어쓰지 않게 한다.
                created_detail_doc_files += 1;
            }
        }

        // queue-idle prompt는 policy가 ReviewAndEnqueue이거나 이미 prompt path가 설정된 경우에만 복구한다. 다른 policy에서
        // 빈 path인 workspace에 불필요한 prompt file을 새로 만들지 않기 위한 gate다. 즉 doctor는 review 기능이 꺼진
        // catalog에 새 behavior를 암묵적으로 켜지 않는다.
        let configured_prompt_path = trimmed_non_empty(directions.queue_idle.prompt_path.as_str());
        let should_repair_queue_idle_prompt = directions.queue_idle.policy
            == QueueIdlePolicy::ReviewAndEnqueue
            || configured_prompt_path.is_some();
        if should_repair_queue_idle_prompt {
            let target_prompt_path = if configured_prompt_path.is_some_and(|path| {
                is_valid_planning_markdown_path(path, PLANNING_PROMPTS_DIRECTORY)
            }) {
                configured_prompt_path.expect("checked above").to_string()
            } else {
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()
            };

            if configured_prompt_path != Some(target_prompt_path.as_str()) {
                // prompt mapping도 contract directory 밖이면 기본 queue-idle prompt path로 되돌린다. worker orchestration은
                // 이 path를 통해 idle queue review prompt를 읽으므로, invalid path를 방치하면 queue-idle follow-up 평가가
                // 조용히 비활성화되거나 잘못된 file을 읽을 수 있다.
                super::set_queue_idle_prompt_path(&mut directions, &target_prompt_path);
                repaired_queue_idle_prompt_mapping = true;
            }

            if self
                .load_supporting_file_best_effort(workspace_dir, &target_prompt_path)
                .is_none()
                && pending_supporting_files
                    .insert(
                        target_prompt_path,
                        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string(),
                    )
                    .is_none()
            {
                // queue-idle prompt file이 없으면 shared default prompt를 생성한다. 이 default는 worker가 queue idle 상태에서
                // proposal/cleanup 판단을 요청할 때 쓰는 evaluator contract의 시작점이다.
                created_queue_idle_prompt_file = true;
            }
        }

        if directions != workspace.directions {
            // catalog 사본이 실제로 달라졌을 때만 commit한다. file만 생성한 doctor run은 direction authority revision을
            // 불필요하게 올리지 않아 operator edit conflict 가능성을 줄인다.
            self.commit_direction_catalog(workspace_dir, &directions)?;
        }
        for (relative_path, body) in pending_supporting_files {
            // supporting file은 catalog commit 뒤에 쓴다. 성공 경로에서는 catalog가 canonical path를 가리키고, 그 path에
            // 기본 body가 존재하는 상태로 workspace를 닫는다. file write가 실패하면 caller가 path와 body 생성 intent를
            // outcome 전 error로 받게 된다.
            self.planning_workspace_port
                .replace_planning_workspace_file(workspace_dir, &relative_path, Some(&body))?;
        }

        // 복구 후 validation을 다시 수행해 caller가 "수정은 했지만 아직 남은 문제"를 바로 볼 수 있게 한다. doctor는
        // 모든 validation error를 자동 해결하는 도구가 아니라 safe repair를 적용한 뒤 상태를 재보고하는 도구다.
        let validation_report = self.validate_active_workspace(workspace_dir)?;

        Ok(PlanningDoctorOutcome {
            repaired_detail_doc_mappings,
            created_detail_doc_files,
            repaired_queue_idle_prompt_mapping,
            created_queue_idle_prompt_file,
            validation_report,
        })
    }

    #[allow(dead_code)]
    // validate_active_workspace는 doctor가 복구한 결과를 planning validator 관점으로 다시 확인한다. 기본 authority seed를
    // 보장한 뒤 direction catalog, result output, supporting file presence를 같은 report에 합쳐 doctor outcome의
    // post-repair evidence로 사용한다.
    fn validate_active_workspace(&self, workspace_dir: &str) -> Result<PlanningValidationReport> {
        // doctor는 direction/supporting file repair에 집중하지만 validation에는 result output 같은 기본 authority file도 필요하다.
        // seed service가 없으면 빈 workspace validation이 unrelated failure로 끝나므로, repair 결과 확인 전에 baseline을 보장한다.
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let directions = self.load_direction_catalog(workspace_dir)?;
        let mut result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions: &directions,
                    // doctor validation은 task authority 자체를 복구하지 않는다. direction/supporting file 문제를 보려는
                    // 흐름이라 최소 빈 task authority JSON으로 validator 입력을 채운다.
                    task_authority_json: "{\"version\":1,\"tasks\":[]}",
                    result_output_markdown: workspace
                        .result_output_markdown
                        .as_deref()
                        .ok_or_else(|| {
                            anyhow!("default planning authority seed did not provide result output")
                        })?,
                });
        if let Some(directions) = result.directions.as_ref() {
            // 첫 validation pass가 direction catalog를 파싱했을 때만 supporting file existence를 검사한다. path lookup은
            // workspace port의 best-effort load로 감싸 file body가 아니라 존재 여부만 validator에 넘긴다.
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| {
                        self.load_supporting_file_best_effort(workspace_dir, path)
                            .is_some()
                    },
                    &mut result.report,
                );
        }

        Ok(result.report)
    }
}
