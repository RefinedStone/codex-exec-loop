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

#[cfg(test)]
mod tests {
    use super::{PlanningPromptService, PlanningRuntimeWorkspaceStatus, workspace_record_to_files};
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
        PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning::PlanningValidationService;
    use crate::application::service::planning::shared::prompt_sections::runtime_task_authority_contract_rules;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
        PriorityQueueProjection, PriorityQueueTask, QueueIdleConfig, TaskAuthorityDocument,
        TaskStatus,
    };
    use anyhow::{Result, anyhow};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Clone, Copy)]
    enum ClearAuthorityOnSecondLoad {
        Task,
        Direction,
    }

    #[derive(Debug)]
    struct PromptTestWorkspacePort {
        record: Mutex<PlanningWorkspaceLoadRecord>,
        optional_files: Mutex<BTreeMap<String, String>>,
        persist_commits: bool,
        load_count: Mutex<usize>,
        clear_on_second_load: Option<(
            Arc<NoopPlanningTaskRepositoryPort>,
            ClearAuthorityOnSecondLoad,
        )>,
    }

    impl PromptTestWorkspacePort {
        fn absent_non_persistent() -> Self {
            Self {
                record: Mutex::new(PlanningWorkspaceLoadRecord::default()),
                optional_files: Mutex::new(BTreeMap::new()),
                persist_commits: false,
                load_count: Mutex::new(0),
                clear_on_second_load: None,
            }
        }

        fn with_result_output(result_output_markdown: &str) -> Self {
            Self {
                record: Mutex::new(PlanningWorkspaceLoadRecord {
                    result_output_markdown: Some(result_output_markdown.to_string()),
                }),
                optional_files: Mutex::new(BTreeMap::new()),
                persist_commits: true,
                load_count: Mutex::new(0),
                clear_on_second_load: None,
            }
        }

        fn with_optional_file(self, path: &str, body: &str) -> Self {
            self.optional_files
                .lock()
                .expect("optional file store should not be poisoned")
                .insert(path.to_string(), body.to_string());
            self
        }

        fn clear_authority_on_second_load(
            mut self,
            repository: Arc<NoopPlanningTaskRepositoryPort>,
            authority: ClearAuthorityOnSecondLoad,
        ) -> Self {
            self.clear_on_second_load = Some((repository, authority));
            self
        }
    }

    impl PlanningWorkspacePort for PromptTestWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            Err(anyhow!(
                "stage_planning_draft_files is outside runtime prompt loading"
            ))
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            Err(anyhow!(
                "load_planning_draft_files is outside runtime prompt loading"
            ))
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!(
                "replace_planning_draft_file is outside runtime prompt loading"
            ))
        }

        fn load_planning_workspace_files(
            &self,
            workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            let mut load_count = self
                .load_count
                .lock()
                .expect("workspace load count should not be poisoned");
            *load_count += 1;
            if *load_count == 2
                && let Some((repository, authority)) = &self.clear_on_second_load
            {
                match authority {
                    ClearAuthorityOnSecondLoad::Task => {
                        repository.clear_task_authority_snapshot(workspace_dir)?;
                    }
                    ClearAuthorityOnSecondLoad::Direction => {
                        repository.clear_direction_authority_snapshot(workspace_dir)?;
                    }
                }
            }
            Ok(self
                .record
                .lock()
                .expect("workspace record should not be poisoned")
                .clone())
        }

        fn load_planning_workspace_candidate_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Err(anyhow!(
                "load_planning_workspace_candidate_files is outside runtime prompt loading"
            ))
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            if self.persist_commits {
                *self
                    .record
                    .lock()
                    .expect("workspace record should not be poisoned") = record.clone();
            }
            Ok(())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            Ok(self
                .optional_files
                .lock()
                .expect("optional file store should not be poisoned")
                .get(relative_path)
                .cloned())
        }

        fn load_optional_planning_candidate_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            Err(anyhow!(
                "load_optional_planning_candidate_file is outside runtime prompt loading"
            ))
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
            body: Option<&str>,
        ) -> Result<()> {
            if self.persist_commits {
                let mut optional_files = self
                    .optional_files
                    .lock()
                    .expect("optional file store should not be poisoned");
                if let Some(body) = body {
                    optional_files.insert(relative_path.to_string(), body.to_string());
                } else {
                    optional_files.remove(relative_path);
                }
            }
            Ok(())
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<()> {
            self.optional_files
                .lock()
                .expect("optional file store should not be poisoned")
                .remove(relative_path);
            Ok(())
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!(
                "archive_rejected_planning_file is outside runtime prompt loading"
            ))
        }
    }

    fn direction(detail_doc_path: &str) -> DirectionDefinition {
        DirectionDefinition {
            id: "general-workstream".to_string(),
            title: "General workstream".to_string(),
            summary: "default".to_string(),
            success_criteria: vec!["done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: detail_doc_path.to_string(),
            state: DirectionState::Active,
        }
    }

    fn directions(detail_doc_path: &str) -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![direction(detail_doc_path)],
        }
    }

    fn empty_task_authority() -> TaskAuthorityDocument {
        TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: Vec::new(),
        }
    }

    fn empty_queue_projection() -> PriorityQueueProjection {
        PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        }
    }

    fn stale_queue_projection() -> PriorityQueueProjection {
        PriorityQueueProjection {
            next_task: None,
            active_tasks: vec![PriorityQueueTask {
                rank: 1,
                task_id: "stale-task".to_string(),
                direction_id: "general-workstream".to_string(),
                direction_title: "General workstream".to_string(),
                task_title: "Stale stored queue task".to_string(),
                status: TaskStatus::Ready,
                combined_priority: 50,
                updated_at: "2026-05-23T00:00:00Z".to_string(),
                rank_reasons: vec!["stale stored projection".to_string()],
            }],
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        }
    }

    fn unique_workspace(label: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        format!("runtime-prompt-{label}-{nanos}")
    }

    fn seed_authority(
        repository: &NoopPlanningTaskRepositoryPort,
        workspace: &str,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
        queue_projection: &PriorityQueueProjection,
    ) {
        repository
            .commit_direction_authority_snapshot(
                workspace,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions,
                },
            )
            .expect("direction authority should commit");
        repository
            .commit_task_authority_snapshot(
                workspace,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority,
                    queue_projection,
                },
            )
            .expect("task authority should commit");
    }

    fn prompt_service(
        workspace_port: PromptTestWorkspacePort,
        repository: Arc<NoopPlanningTaskRepositoryPort>,
    ) -> PlanningPromptService {
        PlanningPromptService::with_task_repository(
            Arc::new(workspace_port),
            PlanningValidationService::new(),
            crate::domain::planning::PriorityQueueService::new(),
            repository,
        )
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
        let directions = directions("");

        let files = workspace_record_to_files(&record, &directions, "{\"version\":1,\"tasks\":[]}");

        assert_eq!(files.directions, &directions);
        assert_eq!(files.task_authority_json, "{\"version\":1,\"tasks\":[]}");
        assert_eq!(files.result_output_markdown, "# Result Output Prompt");
    }

    #[test]
    fn runtime_projection_reports_uninitialized_when_seeded_workspace_still_has_no_operator_files()
    {
        let workspace = unique_workspace("uninitialized");
        let repository = Arc::new(NoopPlanningTaskRepositoryPort);
        let service = prompt_service(PromptTestWorkspacePort::absent_non_persistent(), repository);

        let projection = service
            .load_runtime_projection(&workspace)
            .expect("uninitialized projection should be recoverable");

        assert_eq!(
            projection.workspace_status,
            PlanningRuntimeWorkspaceStatus::Uninitialized
        );
        assert!(!projection.workspace_present);
        assert!(projection.prompt_fragment.is_none());
    }

    #[test]
    fn runtime_projection_reports_task_authority_that_disappears_after_seed() {
        let workspace = unique_workspace("missing-task-authority");
        let repository = Arc::new(NoopPlanningTaskRepositoryPort);
        let directions = directions("");
        let task_authority = empty_task_authority();
        seed_authority(
            repository.as_ref(),
            &workspace,
            &directions,
            &task_authority,
            &empty_queue_projection(),
        );
        let workspace_port = PromptTestWorkspacePort::with_result_output("# Result Output")
            .clear_authority_on_second_load(repository.clone(), ClearAuthorityOnSecondLoad::Task);
        let service = prompt_service(workspace_port, repository);

        let error = service
            .load_runtime_projection(&workspace)
            .expect_err("missing task authority should be reported as an error");

        assert!(
            error
                .to_string()
                .contains("planning task authority is unavailable")
        );
    }

    #[test]
    fn runtime_projection_reports_direction_authority_that_disappears_after_seed() {
        let workspace = unique_workspace("missing-direction-authority");
        let repository = Arc::new(NoopPlanningTaskRepositoryPort);
        let directions = directions("");
        let task_authority = empty_task_authority();
        seed_authority(
            repository.as_ref(),
            &workspace,
            &directions,
            &task_authority,
            &empty_queue_projection(),
        );
        let workspace_port = PromptTestWorkspacePort::with_result_output("# Result Output")
            .clear_authority_on_second_load(
                repository.clone(),
                ClearAuthorityOnSecondLoad::Direction,
            );
        let service = prompt_service(workspace_port, repository);

        let error = service
            .load_runtime_projection(&workspace)
            .expect_err("missing direction authority should be reported as an error");

        assert!(
            error
                .to_string()
                .contains("planning direction authority is unavailable")
        );
    }

    #[test]
    fn runtime_projection_discards_stale_stored_queue_and_validates_supporting_files() {
        let workspace = unique_workspace("stale-stored-queue");
        let repository = Arc::new(NoopPlanningTaskRepositoryPort);
        let detail_doc_path = ".codex-exec-loop/planning/directions/general-workstream.md";
        let directions = directions(detail_doc_path);
        let task_authority = empty_task_authority();
        seed_authority(
            repository.as_ref(),
            &workspace,
            &directions,
            &task_authority,
            &stale_queue_projection(),
        );
        let workspace_port =
            PromptTestWorkspacePort::with_result_output("# Result Output\nContinue queued work.")
                .with_optional_file(detail_doc_path, "# General detail");
        let service = prompt_service(workspace_port, repository);

        let projection = service
            .load_runtime_projection(&workspace)
            .expect("runtime projection should load");

        assert_eq!(
            projection.workspace_status,
            PlanningRuntimeWorkspaceStatus::ReadyNoTask,
            "{:?}",
            projection.failure_reason
        );
        assert_eq!(
            projection.queue_summary.as_deref(),
            Some("queue idle: no executable planning task")
        );
        assert!(
            projection
                .queue_projection
                .as_ref()
                .expect("queue projection should be present")
                .active_tasks
                .is_empty()
        );
        assert!(
            projection
                .prompt_fragment
                .as_deref()
                .expect("prompt fragment should be present")
                .contains(
                    "detail_doc_path=.codex-exec-loop/planning/directions/general-workstream.md"
                )
        );
    }

    #[test]
    fn prompt_workspace_test_double_rejects_surfaces_unused_by_runtime_prompt_loading() {
        let workspace_port = PromptTestWorkspacePort::with_result_output("# Result Output")
            .with_optional_file("obsolete.md", "remove me");
        let record = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("updated".to_string()),
        };

        assert!(
            workspace_port
                .stage_planning_draft_files("workspace", "draft", &[])
                .is_err()
        );
        assert!(
            workspace_port
                .load_planning_draft_files("workspace", "draft")
                .is_err()
        );
        assert!(
            workspace_port
                .replace_planning_draft_file("workspace", "draft", "path", "body")
                .is_err()
        );
        assert!(
            workspace_port
                .load_planning_workspace_candidate_files("workspace")
                .is_err()
        );
        workspace_port
            .commit_planning_workspace_files("workspace", &record)
            .expect("test double should accept active workspace commits");
        workspace_port
            .replace_planning_workspace_file("workspace", "optional.md", Some("body"))
            .expect("test double should accept optional file writes");
        assert_eq!(
            workspace_port
                .load_optional_planning_file("workspace", "optional.md")
                .expect("optional file should load"),
            Some("body".to_string())
        );
        workspace_port
            .replace_planning_workspace_file("workspace", "optional.md", None)
            .expect("test double should accept optional file removals");
        assert!(
            workspace_port
                .load_optional_planning_candidate_file("workspace", "optional.md")
                .is_err()
        );
        workspace_port
            .remove_planning_workspace_entry("workspace", "obsolete.md")
            .expect("test double should accept workspace entry removal");
        assert!(
            workspace_port
                .archive_rejected_planning_file("workspace", "archive", "path", "body")
                .is_err()
        );
    }
}
