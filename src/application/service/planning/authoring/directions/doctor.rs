/*
학습 주석: directions doctor는 operator가 직접 고친 direction catalog와 supporting markdown files 사이의
불일치를 복구하는 서비스 흐름입니다. catalog 안의 path mapping을 canonical planning 경로로 고치고,
누락된 detail doc/queue-idle prompt 파일을 기본 본문으로 생성한 뒤 validation까지 다시 돌려 outcome으로 돌려줍니다.
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
    // 학습 주석: doctor_workspace는 direction authority를 "검사 후 필요한 최소 복구만 적용"하는 entrypoint입니다.
    // catalog 수정과 file 생성은 staging map에 모았다가 뒤에서 한 번씩 쓰고, 마지막 validation report로 남은 문제를 알려 줍니다.
    pub fn doctor_workspace(&self, workspace_dir: &str) -> Result<PlanningDoctorOutcome> {
        // 학습 주석: complete workspace는 현재 direction catalog와 supporting files를 판단할 기준점입니다.
        // directions는 수정 가능한 사본으로 분리해, 실제 변경이 있을 때만 commit하도록 합니다.
        let workspace = self.load_complete_workspace(workspace_dir)?;
        let mut directions = workspace.directions.clone();
        // 학습 주석: outcome counters는 doctor가 무엇을 실제로 고쳤는지 admin/TUI가 설명할 수 있게 하는 audit 값입니다.
        let mut repaired_detail_doc_mappings = 0;
        let mut created_detail_doc_files = 0;
        let mut repaired_queue_idle_prompt_mapping = false;
        let mut created_queue_idle_prompt_file = false;
        // 학습 주석: supporting file 생성 요청은 path별로 모아 둡니다. 같은 target path를 여러 direction이 공유해도
        // insert 결과로 중복 생성을 피하고 counter를 한 번만 올릴 수 있습니다.
        let mut pending_supporting_files = std::collections::HashMap::<String, String>::new();

        // 학습 주석: 각 direction의 detail_doc_path는 `docs/planning/directions/...md` 안쪽의 유효 markdown이어야 합니다.
        // 비어 있거나 잘못된 path는 direction id에서 파생한 기본 path로 되돌립니다.
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
                // 학습 주석: catalog mapping을 고치는 것은 파일 생성과 별개입니다. direction 문서가 어디에 있어야
                // 하는지 authority부터 canonical하게 만든 뒤, 아래에서 파일 존재 여부를 채웁니다.
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
                // 학습 주석: detail doc이 없으면 direction metadata에서 기본 markdown을 만듭니다. best-effort load가
                // None인 경우만 생성해, 이미 사용자가 작성한 파일을 doctor가 덮어쓰지 않게 합니다.
                created_detail_doc_files += 1;
            }
        }

        // 학습 주석: queue-idle prompt는 policy가 ReviewAndEnqueue이거나 이미 prompt path가 설정된 경우에만 복구합니다.
        // 다른 policy에서 빈 path인 workspace에 불필요한 prompt 파일을 새로 만들지 않기 위한 gate입니다.
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
                // 학습 주석: prompt mapping도 contract directory 밖이면 기본 queue-idle prompt path로 되돌립니다.
                // worker orchestration은 이 path를 통해 idle queue review prompt를 읽습니다.
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
                // 학습 주석: queue-idle prompt 파일이 없으면 shared default prompt를 생성합니다. 이 default는
                // worker가 queue idle 상태에서 제안/정리를 요청할 때 쓰는 evaluator contract의 시작점입니다.
                created_queue_idle_prompt_file = true;
            }
        }

        if directions != workspace.directions {
            // 학습 주석: catalog 사본이 실제로 달라졌을 때만 commit합니다. file만 생성한 doctor run은 direction
            // authority revision을 불필요하게 올리지 않습니다.
            self.commit_direction_catalog(workspace_dir, &directions)?;
        }
        for (relative_path, body) in pending_supporting_files {
            // 학습 주석: supporting files는 catalog commit 뒤에 씁니다. catalog가 canonical path를 가리키고,
            // 그 path에 기본 body가 존재하는 상태로 workspace를 닫기 위한 순서입니다.
            self.planning_workspace_port
                .replace_planning_workspace_file(workspace_dir, &relative_path, Some(&body))?;
        }

        // 학습 주석: 복구 후 validation을 다시 수행해 caller가 "수정은 했지만 아직 남은 문제"를 바로 볼 수 있게 합니다.
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
    // 학습 주석: validate_active_workspace는 doctor가 복구한 결과를 planning validator 관점으로 다시 확인합니다.
    // 기본 authority seed를 보장한 뒤 direction catalog, result output, supporting file presence를 같은 report에 합칩니다.
    fn validate_active_workspace(&self, workspace_dir: &str) -> Result<PlanningValidationReport> {
        // 학습 주석: doctor는 direction/supporting file repair에 집중하지만 validation에는 result output 같은
        // 기본 authority 파일도 필요합니다. seed service가 없으면 빈 workspace validation이 unrelated failure로 끝납니다.
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
                    // 학습 주석: doctor validation은 task authority 자체를 복구하지 않습니다. direction/supporting
                    // file 문제를 보려는 흐름이라 최소 빈 task authority JSON으로 validator 입력을 채웁니다.
                    task_authority_json: "{\"version\":1,\"tasks\":[]}",
                    result_output_markdown: workspace
                        .result_output_markdown
                        .as_deref()
                        .ok_or_else(|| {
                            anyhow!("default planning authority seed did not provide result output")
                        })?,
                });
        if let Some(directions) = result.directions.as_ref() {
            // 학습 주석: 첫 validation pass가 direction catalog를 파싱했을 때만 supporting file existence를 검사합니다.
            // path lookup은 workspace port의 best-effort load로 감싸 파일 본문이 아니라 존재 여부만 validator에 넘깁니다.
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
