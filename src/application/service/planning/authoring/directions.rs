use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY, RESULT_OUTPUT_FILE_PATH, default_direction_detail_doc_path,
};
use crate::application::service::planning::shared::planning_paths::is_valid_planning_markdown_path;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningValidationReport, QueueIdlePolicy,
};
use anyhow::{Result, anyhow};
use chrono::Utc;
use std::collections::HashSet;
use std::sync::Arc;
#[cfg(test)]
mod doctor;
mod supporting_files;
use self::supporting_files::{
    normalize_queue_idle_review_prompt_markdown, set_direction_detail_doc_path,
    set_queue_idle_prompt_path, trimmed_non_empty,
};

/*
 * direction maintenance는 DB-backed direction authority와 workspace-backed markdown file 사이에 놓인 authoring
 * boundary다. catalog는 supporting file path만 알고 있고, 실제 detail doc/prompt body는 planning workspace에
 * 있다. 이 service는 그 둘을 같은 contract처럼 다루기 위해 mapping repair, editor staging, validation, operator
 * summary를 한 흐름으로 맞춘다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionsSupportingFileStatus {
    MissingMapping,
    Ready,
    BrokenMapping,
}
impl DirectionsSupportingFileStatus {
    pub fn label(self) -> &'static str {
        // label은 admin/TUI가 supporting file 상태를 짧게 보여 주는 presentation-facing atom이다. domain enum 이름을
        // 그대로 노출하지 않고 "unset/ready/broken"으로 고정해 route/template 쪽 표시 계약을 안정화한다.
        match self {
            Self::MissingMapping => "unset",
            Self::Ready => "ready",
            Self::BrokenMapping => "broken",
        }
    }
    pub fn needs_attention(self) -> bool {
        self != Self::Ready
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceDirectionSummary {
    // summary row는 full direction body를 의도적으로 피한다. directions page는 operator가 어떤 detail doc을 고쳐야
    // 하는지 판단하는 목록이라 identity와 supporting-file health만 필요하다.
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_status: DirectionsSupportingFileStatus,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceSummary {
    // 이 projection은 admin checklist다. direction별 detail doc 상태, aggregate repair count, queue-idle prompt
    // mapping을 한 번에 내려 operator가 repair/editor 진입점을 고를 수 있게 한다.
    pub directions: Vec<DirectionsMaintenanceDirectionSummary>,
    pub missing_detail_doc_count: usize,
    pub broken_detail_doc_count: usize,
    pub queue_idle_policy: QueueIdlePolicy,
    pub queue_idle_prompt_path: Option<String>,
    pub queue_idle_prompt_status: DirectionsSupportingFileStatus,
    pub parse_error: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueIdleReviewContext {
    // runtime queue-idle review는 여기서 normalized prompt markdown을 읽는다. policy는 여전히 authority에서 오므로,
    // review를 끄는 일과 supporting prompt file을 삭제하는 일은 분리된다.
    pub policy: QueueIdlePolicy,
    pub prompt_path: Option<String>,
    pub prompt_markdown: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDoctorOutcome {
    // doctor output은 authority mapping repair와 workspace file creation을 분리해 기록한다. operator feedback이
    // "catalog path를 고쳤는가"와 "실제 markdown file을 만들었는가"를 구분해서 설명할 수 있게 하기 위함이다.
    pub repaired_detail_doc_mappings: usize,
    pub created_detail_doc_files: usize,
    pub repaired_queue_idle_prompt_mapping: bool,
    pub created_queue_idle_prompt_file: bool,
    pub validation_report: PlanningValidationReport,
}
impl PlanningDoctorOutcome {
    pub fn applied_fix_count(&self) -> usize {
        // coarse count는 status copy용이다. post-repair contract의 자세한 상태는 validation_report가 담으므로,
        // 여기서는 몇 가지 자동 repair가 적용됐는지만 합산한다.
        self.repaired_detail_doc_mappings
            + self.created_detail_doc_files
            + usize::from(self.repaired_queue_idle_prompt_mapping)
            + usize::from(self.created_queue_idle_prompt_file)
    }
}
#[derive(Clone)]
pub struct PlanningDirectionsService {
    // workspace port는 markdown body를 소유하고, repository port는 direction authority를 소유한다. validation은
    // 두 view를 하나의 coherent planning contract로 묶는다.
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    authority_seed_service: PlanningAuthoritySeedService,
}
impl PlanningDirectionsService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            // seed service는 direction-maintenance entrypoint마다 "읽기 전에 planning을 usable하게 만들기" 로직을
            // 복제하지 않게 한다. summary/editor/runtime context가 모두 같은 default authority baseline에서 시작한다.
            authority_seed_service: PlanningAuthoritySeedService::new(
                planning_workspace_port.clone(),
                planning_task_repository_port.clone(),
                planning_validation_service.clone(),
                priority_queue_service,
            ),
            planning_workspace_port,
            planning_task_repository_port,
            planning_validation_service,
        }
    }

    fn load_direction_catalog(&self, workspace_dir: &str) -> Result<DirectionCatalogDocument> {
        // direction maintenance가 workspace에서 처음 사용하는 planning 기능일 수 있다. 그래서 catalog read는 항상
        // default-authority seeding을 먼저 통과해 missing direction authority를 normal startup state로 복구한다.
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        self.planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.directions)
            .ok_or_else(|| anyhow!("default planning authority seed did not provide directions"))
    }
    fn commit_direction_catalog(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
    ) -> Result<()> {
        // direction edit는 catalog만 commit한다. supporting markdown body는 workspace draft에 남고 shared draft
        // promotion flow가 active file로 옮긴다. path authority와 body authority를 한 commit에 섞지 않는 경계다.
        match self
            .planning_task_repository_port
            .commit_direction_authority_snapshot(
                workspace_dir,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => Ok(()),
            PlanningTaskAuthorityCommitResult::Conflict { .. } => Err(anyhow!(
                "planning direction authority changed while editing; retry"
            )),
        }
    }

    pub fn load_summary(&self, workspace_dir: &str) -> Result<DirectionsMaintenanceSummary> {
        // summary loading은 모든 body를 열어 보여 주기보다 health check에 집중한다. configured path가 expected
        // planning directory 아래에 있는지, workspace port로 실제 읽을 수 있는지 검사해 repair 필요성을 판단한다.
        let catalog = self.load_direction_catalog(workspace_dir)?;
        let queue_idle_prompt_path =
            trimmed_non_empty(catalog.queue_idle.prompt_path.as_str()).map(str::to_string);
        let queue_idle_prompt_status = self.supporting_file_status(
            workspace_dir,
            queue_idle_prompt_path.as_deref(),
            PLANNING_PROMPTS_DIRECTORY,
        );
        let directions = catalog
            .directions
            .into_iter()
            .map(|direction| {
                // id/title은 trim해서 read-only projection을 깨끗하게 만든다. summary rendering 중 authority를 mutate하지
                // 않으면서도 admin 목록에는 불필요한 공백이 보이지 않게 한다.
                let detail_doc_path =
                    trimmed_non_empty(direction.detail_doc_path.as_str()).map(str::to_string);
                let detail_doc_status = self.supporting_file_status(
                    workspace_dir,
                    detail_doc_path.as_deref(),
                    PLANNING_DIRECTION_DOCS_DIRECTORY,
                );
                Ok(DirectionsMaintenanceDirectionSummary {
                    id: direction.id.trim().to_string(),
                    title: direction.title.trim().to_string(),
                    detail_doc_path,
                    detail_doc_status,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // Missing은 authority에 path가 없다는 뜻이고, broken은 configured path가 invalid하거나 referenced workspace
        // file을 읽을 수 없다는 뜻이다. 둘을 나눠야 admin이 mapping 생성과 file 생성/수정을 다른 action으로 보여 준다.
        let missing_detail_doc_count = directions
            .iter()
            .filter(|direction| {
                direction.detail_doc_status == DirectionsSupportingFileStatus::MissingMapping
            })
            .count();
        let broken_detail_doc_count = directions
            .iter()
            .filter(|direction| {
                direction.detail_doc_status == DirectionsSupportingFileStatus::BrokenMapping
            })
            .count();
        Ok(DirectionsMaintenanceSummary {
            directions,
            missing_detail_doc_count,
            broken_detail_doc_count,
            queue_idle_policy: catalog.queue_idle.policy,
            queue_idle_prompt_path,
            queue_idle_prompt_status,
            parse_error: None,
        })
    }

    pub fn load_queue_idle_review_context(
        &self,
        workspace_dir: &str,
    ) -> Result<QueueIdleReviewContext> {
        // runtime review는 prompt body 부재를 None으로 낮춘다. 그래도 authority policy/path는 그대로 노출해
        // orchestration이 "review disabled"와 "review enabled but prompt missing"을 구분할 수 있게 한다.
        let directions = self.load_direction_catalog(workspace_dir)?;
        let prompt_path =
            trimmed_non_empty(directions.queue_idle.prompt_path.as_str()).map(str::to_string);
        let prompt_markdown = prompt_path
            .as_deref()
            .and_then(|path| self.load_supporting_file_best_effort(workspace_dir, path))
            .map(|prompt| normalize_queue_idle_review_prompt_markdown(&prompt));
        Ok(QueueIdleReviewContext {
            policy: directions.queue_idle.policy,
            prompt_path,
            prompt_markdown,
        })
    }
    pub fn stage_detail_doc_editor_session(
        &self,
        workspace_dir: &str,
        direction_id: &str,
    ) -> Result<PlanningDraftEditorSession> {
        // detail-doc editor를 열 때 catalog path를 먼저 repair할 수 있다. 선택된 path는 direction authority에 commit하고,
        // markdown body는 validation과 later promotion을 위해 workspace draft file로 stage한다.
        let mut workspace = self.load_complete_workspace(workspace_dir)?;
        let selected_direction = workspace
            .directions
            .directions
            .iter()
            .find(|direction| direction.id.trim() == direction_id.trim())
            .ok_or_else(|| anyhow!("unknown direction id: {}", direction_id.trim()))?;
        let (detail_doc_path, detail_doc_body) = self.resolve_detail_doc_editor_target(
            workspace_dir,
            direction_id,
            trimmed_non_empty(selected_direction.detail_doc_path.as_str()),
        )?;
        set_direction_detail_doc_path(&mut workspace.directions, direction_id, &detail_doc_path)?;
        self.commit_direction_catalog(workspace_dir, &workspace.directions)?;
        workspace
            .extra_files
            .retain(|file| file.active_path != detail_doc_path);
        // resolver가 선택한 detail file이 draft에 정확히 한 번만 들어가도록 이미 load된 copy를 교체한다.
        workspace.extra_files.push(PlanningDraftFileRecord {
            active_path: detail_doc_path.clone(),
            body: detail_doc_body,
        });

        self.stage_session_from_source(workspace_dir, workspace, &[detail_doc_path])
    }

    pub fn stage_queue_idle_prompt_editor_session(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningDraftEditorSession> {
        // queue-idle prompt editing도 같은 split을 따른다. authority는 prompt path를 저장하고, workspace draft file은
        // operator가 편집할 markdown body를 저장한다.
        let mut workspace = self.load_complete_workspace(workspace_dir)?;
        let (prompt_path, prompt_body) = self.resolve_queue_idle_prompt_editor_target(
            workspace_dir,
            trimmed_non_empty(workspace.directions.queue_idle.prompt_path.as_str()),
        )?;
        set_queue_idle_prompt_path(&mut workspace.directions, &prompt_path);
        self.commit_direction_catalog(workspace_dir, &workspace.directions)?;
        workspace
            .extra_files
            .retain(|file| file.active_path != prompt_path);
        // active file이 없거나 legacy copy여도 editor가 의미 있는 review contract로 열리도록 normalized/default prompt
        // content를 stage한다.
        workspace.extra_files.push(PlanningDraftFileRecord {
            active_path: prompt_path.clone(),
            body: prompt_body,
        });

        self.stage_session_from_source(workspace_dir, workspace, &[prompt_path])
    }

    fn stage_session_from_source(
        &self,
        workspace_dir: &str,
        source: ActiveDirectionsWorkspace,
        editable_paths: &[String],
    ) -> Result<PlanningDraftEditorSession> {
        // specialized maintenance draft는 result-output과 load된 supporting file 전체를 담지만, UI에는 editable_paths만
        // 노출한다. 숨겨진 파일은 validation이 전체 planning picture를 볼 수 있게 하는 context다.
        let draft_name = build_maintenance_draft_name();
        let mut files = vec![PlanningDraftFileRecord {
            active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            body: source.result_output_markdown,
        }];
        files.extend(source.extra_files);
        self.planning_workspace_port.stage_planning_draft_files(
            workspace_dir,
            &draft_name,
            &files,
        )?;
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, &draft_name)?;
        let validation_report =
            self.validate_loaded_draft(workspace_dir, &source.directions, &loaded)?;
        let editable_path_set = editable_paths.iter().cloned().collect::<HashSet<_>>();
        Ok(PlanningDraftEditorSession {
            draft_name: loaded.draft_name.clone(),
            draft_directory: loaded.draft_directory.clone(),
            editable_files: loaded
                .staged_files
                .into_iter()
                .filter(|file| editable_path_set.contains(file.active_path.as_str()))
                .map(|file| PlanningDraftEditorFile {
                    active_path: file.active_path,
                    staged_path: file.staged_path,
                    body: file.body,
                })
                .collect(),
            validation_report,
        })
    }

    fn load_complete_workspace(&self, workspace_dir: &str) -> Result<ActiveDirectionsWorkspace> {
        // editor draft의 source snapshot은 authoritative directions, result-output markdown, referenced supporting
        // files를 합쳐 만든다. 이 aggregate는 active workspace를 그대로 복사하는 것이 아니라 editor가 검증 가능한
        // draft를 만들기 위한 임시 view다.
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let directions = self.load_direction_catalog(workspace_dir)?;
        let mut active_workspace = ActiveDirectionsWorkspace {
            directions,
            result_output_markdown: workspace.result_output_markdown.ok_or_else(|| {
                anyhow!("default planning authority seed did not provide result output")
            })?,
            extra_files: Vec::new(),
        };
        let mut supporting_paths = HashSet::new();
        if let Some(prompt_path) =
            trimmed_non_empty(active_workspace.directions.queue_idle.prompt_path.as_str())
        {
            supporting_paths.insert(prompt_path.to_string());
        }
        supporting_paths.extend(
            active_workspace
                .directions
                .directions
                .iter()
                .filter_map(|direction| trimmed_non_empty(direction.detail_doc_path.as_str()))
                .map(str::to_string),
        );
        // missing supporting file은 여기서 생략한다. 선택된 editor target의 resolver가 나중에 empty/default staged body를
        // 만들 수 있으므로, source snapshot 단계에서 없는 파일을 암묵적으로 생성하지 않는다.
        for supporting_path in supporting_paths {
            if let Some(body) =
                self.load_supporting_file_best_effort(workspace_dir, &supporting_path)
            {
                active_workspace.extra_files.push(PlanningDraftFileRecord {
                    active_path: supporting_path,
                    body,
                });
            }
        }
        Ok(active_workspace)
    }
    fn validate_loaded_draft(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
        loaded: &PlanningDraftLoadRecord,
    ) -> Result<PlanningValidationReport> {
        // staged supporting file을 active workspace file보다 먼저 검증한다. in-progress draft가 broken mapping을 고치는
        // 중이라면 promotion 전 validation이 staged fix를 반영해야 한다.
        let staged_file_map = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<std::collections::HashMap<_, _>>();
        let result_output_markdown =
            if let Some(body) = staged_file_map.get(RESULT_OUTPUT_FILE_PATH).copied() {
                body.to_string()
            } else {
                self.planning_workspace_port
                    .load_optional_planning_file(workspace_dir, RESULT_OUTPUT_FILE_PATH)?
                    .unwrap_or_default()
            };
        // 이 editor는 task authority를 수정하지 않는다. direction/result-output validation을 독립적으로 돌리기 위해
        // minimal valid task authority document만 사용한다.
        let mut result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions,
                task_authority_json: "{\"version\":1,\"tasks\":[]}",
                result_output_markdown: &result_output_markdown,
            },
        );
        self.planning_validation_service
            .validate_direction_supporting_files(
                directions,
                |path| {
                    staged_file_map.contains_key(path)
                        || self
                            .load_supporting_file_best_effort(workspace_dir, path)
                            .is_some()
                },
                &mut result.report,
            );
        Ok(result.report)
    }

    fn load_supporting_file_best_effort(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Option<String> {
        // supporting file read는 여기서 advisory다. caller가 absence를 explicit status, fallback body, validation
        // diagnostic 중 어떤 의미로 낮출지 결정한다.
        self.planning_workspace_port
            .load_optional_planning_file(workspace_dir, relative_path)
            .ok()
            .flatten()
    }

    fn supporting_file_status(
        &self,
        workspace_dir: &str,
        configured_path: Option<&str>,
        required_prefix: &str,
    ) -> DirectionsSupportingFileStatus {
        // mapped supporting file은 expected planning directory 안에 있고 workspace port로 읽힐 때만 healthy다. path
        // prefix validation과 file existence를 같이 보는 이유다.
        let Some(path) = configured_path else {
            return DirectionsSupportingFileStatus::MissingMapping;
        };
        if !is_valid_planning_markdown_path(path, required_prefix) {
            return DirectionsSupportingFileStatus::BrokenMapping;
        }
        if self
            .load_supporting_file_best_effort(workspace_dir, path)
            .is_some()
        {
            DirectionsSupportingFileStatus::Ready
        } else {
            DirectionsSupportingFileStatus::BrokenMapping
        }
    }

    fn resolve_detail_doc_editor_target(
        &self,
        workspace_dir: &str,
        direction_id: &str,
        configured_path: Option<&str>,
    ) -> Result<(String, String)> {
        // valid configured path는 file이 없어도 보존한다. empty staged body를 열어 주면 operator가 missing document를
        // 같은 path에 새로 만들 수 있다.
        if let Some(path) = configured_path
            .filter(|path| is_valid_planning_markdown_path(path, PLANNING_DIRECTION_DOCS_DIRECTORY))
        {
            match self
                .planning_workspace_port
                .load_optional_planning_file(workspace_dir, path)
            {
                Ok(Some(body)) => return Ok((path.to_string(), body)),
                Ok(None) => return Ok((path.to_string(), String::new())),
                Err(_) => {}
            }
        }
        // invalid/absent mapping은 doctor/admin repair flow와 같은 deterministic detail-doc path로 fallback한다.
        let fallback_path = default_direction_detail_doc_path(direction_id);
        let fallback_body = self
            .load_supporting_file_best_effort(workspace_dir, &fallback_path)
            .unwrap_or_default();
        Ok((fallback_path, fallback_body))
    }

    fn resolve_queue_idle_prompt_editor_target(
        &self,
        workspace_dir: &str,
        configured_path: Option<&str>,
    ) -> Result<(String, String)> {
        // valid configured prompt path는 보존하되, loaded content는 canonical queue-idle review prompt shape로
        // normalize한다. legacy file authority copy가 runtime으로 새지 않게 하는 repair 지점이다.
        if let Some(path) = configured_path
            .filter(|path| is_valid_planning_markdown_path(path, PLANNING_PROMPTS_DIRECTORY))
        {
            match self
                .planning_workspace_port
                .load_optional_planning_file(workspace_dir, path)
            {
                Ok(Some(body)) => {
                    return Ok((
                        path.to_string(),
                        normalize_queue_idle_review_prompt_markdown(&body),
                    ));
                }
                Ok(None) => {
                    return Ok((
                        path.to_string(),
                        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string(),
                    ));
                }
                Err(_) => {}
            }
        }
        // fallback path/body는 Simple-mode bootstrap, doctor repair, queue-idle runtime review를 같은 default prompt
        // contract에 맞춘다.
        let fallback_path = DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string();
        let fallback_body = self
            .load_supporting_file_best_effort(workspace_dir, &fallback_path)
            .unwrap_or_else(|| DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string());
        Ok((fallback_path, fallback_body))
    }
}

struct ActiveDirectionsWorkspace {
    // maintenance draft를 stage하는 동안만 쓰는 internal aggregate다. consistent draft를 만들 수 있을 만큼만 authority와
    // workspace body를 결합한다.
    directions: DirectionCatalogDocument,
    result_output_markdown: String,
    extra_files: Vec<PlanningDraftFileRecord>,
}
fn build_maintenance_draft_name() -> String {
    let now = Utc::now();
    format!(
        "directions-{}Z-{:09}",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    )
}
