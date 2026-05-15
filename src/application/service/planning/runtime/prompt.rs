/*
 * planning runtime의 읽기 모델 경계다. operator가 관리하는 workspace markdown과 DB가
 * 승인한 direction/task authority를 한 번의 일관된 read로 합친 뒤, policy.rs, facade.rs,
 * TUI overlay, auto-follow prompt assembly가 공유하는 `PlanningRuntimeProjection`으로 낮춘다.
 *
 * 중요한 점은 validator 입력 형태가 여전히 "파일 묶음"이라는 것이다. runtime은 오래된
 * 검증/fragment 조립 코드를 넓히지 않기 위해 file-shaped contract를 유지하지만,
 * task authority의 신뢰 원천은 operator 파일이 아니라 accepted DB snapshot이다. 따라서
 * DB ledger를 JSON으로 직렬화해 validator에 넣고, result-output 같은 operator instruction만
 * workspace 파일에서 읽는다.
 */
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningWorkspaceFiles, RuntimeProjection, RuntimeWorkspaceStatus,
    TaskAuthorityDocument, TaskDefinition,
};
use anyhow::{Context, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod fragment;

use self::fragment::{build_prompt_fragment, trimmed_non_empty};

const MAX_PROPOSAL_SUMMARY_TITLES: usize = 2;

#[derive(Clone)]
pub struct PlanningPromptService {
    // workspace port와 repository port는 분리해 둔다. runtime prompt loading은 operator-authored markdown file과
    // DB-accepted planning authority라는 두 authority plane을 한 projection으로 합치기 때문이다.
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    authority_seed_service: PlanningAuthoritySeedService,
}

pub type PlanningRuntimeProjection = RuntimeProjection;
pub type PlanningRuntimeWorkspaceStatus = RuntimeWorkspaceStatus;

impl PlanningPromptService {
    #[cfg(test)]
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
            Arc::new(crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort),
        )
    }

    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            authority_seed_service: PlanningAuthoritySeedService::new(
                planning_workspace_port.clone(),
                planning_task_repository_port.clone(),
                planning_validation_service.clone(),
                priority_queue_service.clone(),
            ),
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
            planning_task_repository_port,
        }
    }

    pub fn load_runtime_projection(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningRuntimeProjection> {
        /*
         * runtime planning read pipeline이다. 복구 가능한 planning 문제는 application error로
         * 터뜨리지 않고 invalid projection으로 접는다. TUI와 repair service가 incomplete file,
         * validation failure, queue construction failure를 같은 projection surface에서 설명해야
         * 하기 때문이다.
         */
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let workspace_present = workspace_record.has_any_files();
        if !workspace_present {
            return Ok(PlanningRuntimeProjection::uninitialized());
        }

        let missing_paths = missing_workspace_paths(&workspace_record);
        if !missing_paths.is_empty() {
            /*
             * operator 파일이 일부만 있으면 planning은 시작됐지만 신뢰할 수 없는 상태다.
             * inactive로 되돌리면 사용자가 왜 planning이 사라졌는지 알 수 없으므로 invalid로
             * 유지해 repair/doctor 안내가 보이게 한다.
             */
            return Ok(PlanningRuntimeProjection::invalid(format!(
                "planning files incomplete: missing {}",
                missing_paths.join(", ")
            ))
            .with_workspace_present(workspace_present));
        }

        // runtime validation은 task-ledger 파일이 아니라 accepted DB authority를 사용한다.
        // 다만 validator의 입력 계약은 file-shaped workspace bundle이므로 여기서 adapter처럼
        // 두 authority plane을 한 구조로 묶는다.
        let task_authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "planning task authority is unavailable; initialize or repair the planning database"
                )
            })?;
        let direction_authority_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "planning direction authority is unavailable; initialize or repair the planning database"
                )
            })?;
        let authority_task_authority_json =
            serde_json::to_string(&task_authority_snapshot.task_authority)
                .context("failed to serialize task authority ledger")?;
        let workspace_files = workspace_record_to_files(
            &workspace_record,
            &direction_authority_snapshot.directions,
            &authority_task_authority_json,
        );

        let mut validation_result = self
            .planning_validation_service
            .validate_workspace_files(workspace_files);
        if let Some(directions) = validation_result.directions.as_ref() {
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| {
                        self.planning_workspace_port
                            .load_optional_planning_file(workspace_dir, path)
                            .ok()
                            .flatten()
                            .is_some()
                    },
                    &mut validation_result.report,
                );
        }

        if !validation_result.is_valid() {
            let first_error = validation_result
                .report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning validation failed".to_string());
            return Ok(PlanningRuntimeProjection::invalid(format!(
                "planning validation failed: {first_error}"
            ))
            .with_workspace_present(workspace_present));
        }

        let directions = validation_result
            .directions
            .expect("valid planning directions should be available");
        let task_authority = validation_result
            .task_authority
            .expect("valid planning task ledger should be available");
        let stored_queue_projection = Some(task_authority_snapshot.queue_projection);
        let current_queue_projection = match self
            .priority_queue_service
            .build_projection(&directions, &task_authority)
        {
            Ok(queue_projection) => queue_projection,
            Err(error) => {
                /*
                 * ledger가 schema/semantic validation을 통과해도 execution precondition이
                 * 모순이면 queue construction이 실패할 수 있다. 이 경우도 runtime failure로
                 * 끊지 않고 invalid runtime projection으로 표면화해 repair 경로가 구체적인
                 * 원인을 보여 주게 한다.
                 */
                return Ok(PlanningRuntimeProjection::invalid(format!(
                    "planning queue build failed: {error}"
                ))
                .with_workspace_present(workspace_present));
            }
        };

        // 저장된 projection이 live rebuild와 같으면 저장본을 우선한다. repository가 보존한
        // ordering metadata를 살리되, authority 변경 뒤의 stale projection은 즉시 버린다.
        let queue_projection = match stored_queue_projection {
            Some(stored_queue_projection)
                if stored_queue_projection == current_queue_projection =>
            {
                stored_queue_projection
            }
            _ => current_queue_projection,
        };
        let result_output_markdown = workspace_record
            .result_output_markdown
            .as_deref()
            .expect("complete planning workspace should include result output");
        let queue_summary = queue_projection.queue_summary();
        let proposal_summary = queue_projection.proposal_summary(MAX_PROPOSAL_SUMMARY_TITLES);
        let prompt_fragment =
            build_prompt_fragment(&directions, &queue_projection, result_output_markdown);

        let queue_idle_prompt_path =
            trimmed_non_empty(directions.queue_idle.prompt_path.as_str()).map(str::to_string);
        let task_authority_signature = normalized_task_authority_signature(&task_authority);
        let queue_head_task_signature = queue_projection
            .next_task
            .as_ref()
            .and_then(|queue_head| {
                task_authority
                    .tasks
                    .iter()
                    .find(|task| task.id.trim() == queue_head.task_id.trim())
            })
            .map(normalized_task_signature);

        Ok(PlanningRuntimeProjection {
            workspace_present,
            workspace_status: if queue_projection.next_task.is_some() {
                PlanningRuntimeWorkspaceStatus::ReadyWithTask
            } else {
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            },
            prompt_fragment: Some(prompt_fragment),
            queue_summary: Some(queue_summary),
            proposal_summary,
            queue_idle_policy: directions.queue_idle.policy,
            queue_idle_prompt_path,
            queue_head: queue_projection.next_task.clone(),
            queue_projection: Some(queue_projection),
            task_authority_signature: Some(task_authority_signature),
            queue_head_task_signature,
            failure_reason: None,
            auto_follow_pause_reason: None,
        })
    }
}

fn normalized_task_authority_signature(task_authority: &TaskAuthorityDocument) -> u64 {
    // task 순서나 dependency vector 순서만 바뀐 경우 repeat-turn detection이 authority
    // 변경으로 오해하면 안 된다. hashing 전에 ledger를 정렬해 의미 없는 순서 차이를 제거한다.
    let mut normalized_ledger = task_authority.clone();
    normalized_ledger
        .tasks
        .sort_by(|left, right| left.id.cmp(&right.id));
    for task in &mut normalized_ledger.tasks {
        task.depends_on.sort();
        task.blocked_by.sort();
    }

    normalized_json_signature(&normalized_ledger)
}

fn normalized_task_signature(task: &TaskDefinition) -> u64 {
    // queue head 반복 감지는 TaskDefinition의 normalized view를 기준으로 한다. 표시용 공백이나
    // 정렬 차이가 아니라 다음 turn에 넘겨진 실제 작업 의미가 바뀌었는지를 보려는 값이다.
    normalized_json_signature(&task.normalized())
}

fn normalized_json_signature<T>(value: &T) -> u64
where
    T: serde::Serialize,
{
    // projection signature는 로컬 process 내부의 비교 신호이므로 serde JSON + DefaultHasher로 충분하다.
    // 외부 저장/호환 포맷이 아니어서 hash algorithm 안정성을 API 계약으로 노출하지 않는다.
    let json = serde_json::to_string(value)
        .expect("valid planning state should serialize into a signature");
    let mut hasher = DefaultHasher::new();
    json.hash(&mut hasher);
    hasher.finish()
}

fn workspace_record_to_files<'a>(
    workspace_record: &'a PlanningWorkspaceLoadRecord,
    directions: &'a DirectionCatalogDocument,
    task_authority_json: &'a str,
) -> PlanningWorkspaceFiles<'a> {
    // validator의 기존 file-shaped 입력을 유지하면서 제거된 task-authority 파일 자리에
    // DB-backed authority를 넣는 adapter다. runtime prompt assembly는 이 덕분에 저장소
    // 이관 사실을 몰라도 같은 validation result를 소비할 수 있다.
    PlanningWorkspaceFiles {
        directions,
        task_authority_json,
        result_output_markdown: workspace_record
            .result_output_markdown
            .as_deref()
            .expect("complete planning workspace should include result output"),
    }
}

fn missing_workspace_paths(workspace_record: &PlanningWorkspaceLoadRecord) -> Vec<&'static str> {
    // direction/task authority는 이제 DB snapshot에서 오므로 workspace missing path로
    // 보고하지 않는다. operator가 실제로 복구해야 하는 파일만 surface에 남긴다.
    let mut missing_paths = Vec::new();
    if workspace_record.result_output_markdown.is_none() {
        missing_paths.push(RESULT_OUTPUT_FILE_PATH);
    }
    missing_paths
}

#[cfg(test)]
mod tests {
    use super::{missing_workspace_paths, workspace_record_to_files};
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
    use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
    use crate::application::service::planning::shared::prompt_sections::runtime_task_authority_contract_rules;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig,
    };

    #[test]
    fn missing_workspace_paths_only_reports_operator_files() {
        let record = PlanningWorkspaceLoadRecord {
            result_output_markdown: None,
        };

        assert_eq!(
            missing_workspace_paths(&record),
            vec![RESULT_OUTPUT_FILE_PATH]
        );
    }

    #[test]
    fn task_authority_contract_uses_db_authority_source_of_truth() {
        let rules = runtime_task_authority_contract_rules().join("\n");

        assert!(rules.contains("accepted DB authority"));
        assert!(!rules.contains("task-ledger.json"));
    }

    #[test]
    fn planning_runtime_read_model_uses_projection_vocabulary() {
        let legacy_type_name = ["PlanningRuntime", "Snapshot"].concat();
        let legacy_loader_name = ["load_runtime", "_snapshot"].concat();
        let legacy_field_name = ["runtime", "_snapshot"].concat();
        for source in [
            include_str!("prompt.rs"),
            include_str!("facade.rs"),
            include_str!("policy.rs"),
            include_str!("../application_projection.rs"),
            include_str!("../use_cases.rs"),
        ] {
            assert!(!source.contains(&legacy_type_name));
            assert!(!source.contains(&legacy_loader_name));
            assert!(!source.contains(&legacy_field_name));
        }
    }

    #[test]
    fn workspace_record_combines_db_task_authority_with_operator_files() {
        let record = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("# Result Output Prompt".to_string()),
        };
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General workstream".to_string(),
                summary: "default".to_string(),
                success_criteria: vec!["done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        };

        let files = workspace_record_to_files(&record, &directions, "{\"version\":1,\"tasks\":[]}");

        assert_eq!(files.directions, &directions);
        assert_eq!(files.task_authority_json, "{\"version\":1,\"tasks\":[]}");
        assert_eq!(files.result_output_markdown, "# Result Output Prompt");
    }
}
