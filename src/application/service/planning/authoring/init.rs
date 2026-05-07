use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningStagedFileRecord,
    PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningValidationReport,
    TaskAuthorityDocument,
};
use anyhow::{Result, anyhow};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;

/*
 * PlanningInitService는 bootstrap artifact를 operator-visible draft 또는 active planning workspace로 전환하는
 * 경계다. workspace markdown file, DB direction authority, DB task authority, queue projection이 서로 다른
 * 저장소에 있지만, init/promotion에서는 validation 뒤 하나의 accepted planning state처럼 함께 써야 한다. 그래서
 * 이 service가 staging, validation, active write, rollback, authority commit 순서를 모두 소유한다.
 */
#[derive(Clone)]
pub struct PlanningInitService {
    // workspace file은 operator-editable markdown을 저장하고, repository authority는 accepted JSON state를 저장한다.
    // init/promotion이 한쪽만 갱신한 상태로 끝나지 않게 두 port를 같은 service에서 조율한다.
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
    planning_validation_service: PlanningValidationService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    priority_queue_service: PriorityQueueService,
}

#[derive(Debug, Clone)]
pub struct PlanningInitStageResult {
    // staging 결과는 draft 위치와 validation state를 함께 돌려준다. active file을 덮어쓰기 전에 operator가 bootstrap
    // draft를 열어 수정할 수 있어야 하기 때문이다.
    pub mode: PlanningBootstrapMode,
    pub draft_name: String,
    pub draft_directory: String,
    pub staged_files: Vec<PlanningStagedFileRecord>,
    pub staged_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

impl PlanningInitStageResult {
    pub fn is_valid(&self) -> bool {
        self.validation_report.is_valid()
    }
    pub fn status_text(&self) -> String {
        // compact status는 command/TUI feedback용 한 줄 문구다. 전체 validation report는 별도 surface가 보여 주고,
        // 여기서는 mode/draft/files/validity만 빠르게 확인할 수 있게 한다.
        format!(
            "planning init staged / mode: {} / draft: {} / files: {} / validation: {}",
            match self.mode {
                PlanningBootstrapMode::Detail => "detail",
                PlanningBootstrapMode::Simple => "simple",
            },
            self.draft_name,
            self.staged_file_count,
            if self.is_valid() {
                "ok"
            } else {
                "needs attention"
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftEditorFile {
    // active_path는 최종 workspace target이고 staged_path는 draft copy다. 둘을 같이 노출해야 editor가 지금 고치는
    // 격리 사본과 나중에 promotion될 active file을 혼동하지 않는다.
    pub active_path: String,
    pub staged_path: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftEditorSession {
    // manual editor는 operator-editable file만 본다. 하지만 validation은 전체 staged draft directory를 기준으로
    // 계산되어 숨겨진 supporting file 누락도 promotion 전에 드러난다.
    pub draft_name: String,
    pub draft_directory: String,
    pub editable_files: Vec<PlanningDraftEditorFile>,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftSaveResult {
    // save result는 promotion 결과와 다르게 file count를 보고하지 않는다. staged draft를 갱신하고 validation snapshot만
    // 새로 계산했을 뿐, active workspace나 DB authority에는 아직 아무 효과가 없기 때문이다.
    pub draft_name: String,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftPromoteResult {
    // validation이 실패하면 promoted_file_count는 0이다. caller는 시도 자체는 정상 처리됐지만 active workspace
    // state가 바뀌지 않았다는 사실을 operator에게 보여 줄 수 있다.
    pub draft_name: String,
    pub promoted_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceInitResult {
    // direct init은 staging과 달리 bootstrap file을 즉시 active workspace에 쓴다. created_paths는 operator가
    // 실제 생성된 planning-relative path를 감사할 수 있게 하는 기록이다.
    // DB direction/task authority seed도 함께 일어나지만, 그 효과는 file path 목록이 아니라 mode 선택으로 설명된다.
    pub mode: PlanningBootstrapMode,
    pub created_file_count: usize,
    pub created_paths: Vec<String>,
}
impl PlanningInitService {
    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
        planning_validation_service: PlanningValidationService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        // production composition은 모든 boundary를 명시적으로 주입한다. bootstrap, validation, repository commit,
        // queue projection을 adapter test에서 갈아 끼울 수 있게 하는 조립 지점이다.
        Self {
            planning_workspace_port,
            planning_bootstrap_service,
            planning_validation_service,
            planning_task_repository_port,
            priority_queue_service,
        }
    }

    pub fn stage_simple_mode_draft(&self, workspace_dir: &str) -> Result<PlanningInitStageResult> {
        // Simple mode staging은 queue-idle-ready bootstrap을 draft에만 만든다. active planning file과 accepted DB
        // authority는 건드리지 않아 operator가 auto-follow baseline을 검토한 뒤 promotion을 결정할 수 있다.
        self.stage_draft(workspace_dir, PlanningBootstrapMode::Simple)
    }

    pub fn stage_manual_editor_session(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningDraftEditorSession> {
        // manual editor는 Detail bootstrap에서 시작한다. operator가 placeholder direction taxonomy를 실제 project
        // taxonomy로 바꾼 뒤 promotion할 수 있게 하기 위한 authoring-first 경로다.
        self.stage_editor_session(workspace_dir, PlanningBootstrapMode::Detail)
    }

    pub fn load_manual_editor_session(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftEditorSession> {
        // draft load는 원래 stage result의 validation을 믿지 않고 staged file에서 다시 계산한다. editor save가
        // 반복될 수 있으므로 session view는 항상 현재 draft body 기준이어야 한다.
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)?;
        let validation_report = self.validate_loaded_draft(workspace_dir, &loaded)?;
        Ok(PlanningDraftEditorSession {
            draft_name: loaded.draft_name,
            draft_directory: loaded.draft_directory,
            editable_files: loaded
                .staged_files
                .into_iter()
                // manual init은 현재 result-output만 editor에 노출한다. authority JSON은 free-form text가 아니라
                // validated bootstrap struct에서 commit되므로, 이 surface에서 임의 JSON 편집을 허용하지 않는다.
                .filter(|file| is_operator_editable_draft_path(file.active_path.as_str()))
                .map(|file| PlanningDraftEditorFile {
                    active_path: file.active_path,
                    staged_path: file.staged_path,
                    body: file.body,
                })
                .collect(),
            validation_report,
        })
    }

    pub fn has_planning_workspace(&self, workspace_dir: &str) -> Result<bool> {
        // active workspace 탐지는 file 기반이다. 오래된 workspace는 DB authority snapshot보다 먼저 만들어졌을 수 있어
        // repository 상태만 보면 이미 존재하는 planning workspace를 놓칠 수 있다.
        Ok(self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?
            .has_any_files())
    }

    pub fn has_planning_candidate_workspace(&self, workspace_dir: &str) -> Result<bool> {
        // candidate 탐지는 full active workspace 이전에 init overlay가 만든 staged/generated planning file을 찾는다.
        // UI가 "초기화 가능"과 "이미 후보 draft가 있음"을 구분하는 데 쓰인다.
        Ok(self
            .planning_workspace_port
            .load_planning_workspace_candidate_files(workspace_dir)?
            .has_any_files())
    }
    pub fn initialize_simple_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceInitResult> {
        // direct simple init은 editor를 거치지 않는 빠른 경로다. bootstrap을 검증한 뒤 file을 쓰고, accepted authority와
        // queue projection을 seed한다.
        self.initialize_workspace(workspace_dir, PlanningBootstrapMode::Simple)
    }

    pub fn save_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftSaveResult> {
        // save는 editor가 보낸 file만 staged copy에서 교체한다. 그 뒤 draft를 다시 읽어 validation을 보고하지만
        // active workspace로 promote하지는 않는다.
        let loaded = self.replace_and_load_draft_editor_files(workspace_dir, draft_name, files)?;
        Ok(PlanningDraftSaveResult {
            draft_name: draft_name.to_string(),
            validation_report: self.validate_loaded_draft(workspace_dir, &loaded)?,
        })
    }
    pub fn promote_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftPromoteResult> {
        // editor promotion은 먼저 최신 editor body를 draft directory에 저장한 뒤 staged draft promotion과 같은 경로를
        // 탄다. save와 promote가 서로 다른 validation source를 보지 않게 하는 흐름이다.
        let loaded = self.replace_and_load_draft_editor_files(workspace_dir, draft_name, files)?;
        self.promote_loaded_draft(workspace_dir, draft_name, loaded)
    }
    pub fn promote_staged_draft(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftPromoteResult> {
        // non-editor promotion은 이미 complete draft를 stage한 admin flow가 검증된 active-state transition만 필요할 때
        // 사용한다. editor body merge 없이 loaded draft를 그대로 promotion한다.
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)?;
        self.promote_loaded_draft(workspace_dir, draft_name, loaded)
    }
    fn replace_and_load_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftLoadRecord> {
        // editor save는 staged copy만 수정한다. active_path는 workspace adapter가 draft directory 안에서 대응 file을
        // 찾는 key로 쓰이며, 같은 path의 active workspace file은 건드리지 않는다.
        for file in files {
            self.planning_workspace_port.replace_planning_draft_file(
                workspace_dir,
                draft_name,
                &file.active_path,
                &file.body,
            )?;
        }

        self.planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)
    }

    fn promote_loaded_draft(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        loaded: PlanningDraftLoadRecord,
    ) -> Result<PlanningDraftPromoteResult> {
        // promotion은 validation-gated다. invalid draft는 error가 아니라 promoted_file_count 0인 정상 결과를 돌려
        // UI가 infrastructure failure처럼 보이지 않고 validation detail을 그대로 보여 줄 수 있게 한다.
        let validation_result = self.validate_loaded_draft_result(workspace_dir, &loaded)?;
        let validation_report = validation_result.report.clone();
        if !validation_report.is_valid() {
            return Ok(PlanningDraftPromoteResult {
                draft_name: draft_name.to_string(),
                promoted_file_count: 0,
                validation_report,
            });
        }
        // 여기부터는 validation이 parsed authority document를 제공한다는 전제 아래 active state transition을 준비한다.
        // raw staged text를 다시 조합하지 않고 validation_result의 domain value만 쓰는 이유는 promotion과 direct init이
        // 같은 normalized authority를 repository에 넣게 하기 위해서다.
        let directions = validation_result
            .directions
            .as_ref()
            .ok_or_else(|| anyhow!("valid staged draft did not include directions"))?;
        let task_authority = validation_result
            .task_authority
            .as_ref()
            .ok_or_else(|| anyhow!("valid staged draft did not include task-authority"))?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| anyhow!("valid staged draft queue build failed: {error}"))?;
        // 교체될 active file마다 pre-promotion snapshot을 저장한다. 뒤에서 workspace write나 authority write가 실패하면
        // 이 body들이 rollback source of truth가 된다.
        let mut previous_active_files = HashMap::new();
        for file in &loaded.staged_files {
            previous_active_files.insert(
                file.active_path.clone(),
                self.planning_workspace_port
                    .load_optional_planning_file(workspace_dir, &file.active_path)?,
            );
        }
        let mut applied_paths = Vec::with_capacity(loaded.staged_files.len());
        let promote_result = (|| -> Result<()> {
            // workspace file을 DB authority보다 먼저 쓴다. 성공 경로에서는 committed authority가 missing active markdown을
            // 가리키지 않아야 하기 때문이다. partial workspace write는 아래 rollback이 처리한다.
            // 반대로 DB commit 뒤 file write를 하면 rollback으로 되돌릴 수 없는 accepted authority가 먼저 노출될 수 있다.
            for file in &loaded.staged_files {
                self.planning_workspace_port
                    .replace_planning_workspace_file(
                        workspace_dir,
                        &file.active_path,
                        Some(file.body.as_str()),
                    )?;
                applied_paths.push(file.active_path.clone());
            }
            self.commit_direction_authority_from_bootstrap(workspace_dir, directions)?;
            // draft promotion은 operator authority rewrite다. incremental task command를 적용하는 것이 아니라,
            // validation이 끝난 accepted task authority snapshot을 통째로 교체한다.
            self.planning_task_repository_port
                .commit_task_authority_snapshot(
                    workspace_dir,
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: None,
                        task_authority,
                        queue_projection: &queue_projection,
                    },
                )?;
            Ok(())
        })();
        if let Err(error) = promote_result {
            // 여기서 rollback하는 대상은 workspace file write다. DB authority write가 workspace replacement 뒤 실패하면
            // active file layer를 마지막으로 알던 상태로 되돌리고, 원래 authority error를 그대로 표면화한다.
            // rollback 실패 메시지에 수동 복구 path를 싣는 이유는 이 service가 DB commit 실패와 file 복원 실패를 동시에
            // 완전히 자동 복구할 수 없기 때문이다.
            if let Err(rollback_error) = self.restore_promoted_active_state(
                workspace_dir,
                &applied_paths,
                &previous_active_files,
            ) {
                let mut manual_recovery_paths = applied_paths.clone();
                manual_recovery_paths.sort();
                manual_recovery_paths.dedup();
                return Err(anyhow!(
                    "failed to promote staged draft `{draft_name}`: {error}; rollback failed: {rollback_error}; manual recovery may be required for: {}",
                    manual_recovery_paths.join(", ")
                ));
            }
            return Err(error);
        }
        Ok(PlanningDraftPromoteResult {
            draft_name: draft_name.to_string(),
            promoted_file_count: loaded.staged_files.len(),
            validation_report,
        })
    }
    fn restore_promoted_active_state(
        &self,
        workspace_dir: &str,
        applied_paths: &[String],
        previous_active_files: &HashMap<String, Option<String>>,
    ) -> Result<()> {
        // rollback은 write 역순으로 수행한다. 정상 draft는 unique active path를 가져야 하지만, 중복 path가 들어와도
        // replacement stack처럼 되돌아가게 하는 방어적 순서다.
        for active_path in applied_paths.iter().rev() {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    active_path,
                    previous_active_files
                        .get(active_path)
                        .and_then(|body| body.as_deref()),
                )?;
        }
        Ok(())
    }
    fn stage_draft(
        &self,
        workspace_dir: &str,
        mode: PlanningBootstrapMode,
    ) -> Result<PlanningInitStageResult> {
        // staging은 bootstrap file을 isolated draft directory에 materialize한다. operator가 active file로 만들기 전에
        // 검토하고 고칠 수 있는 reversible path다.
        let bootstrap = self.prepare_bootstrap_workspace(mode);
        let draft_name = build_bootstrap_draft_name(Utc::now());
        let stage_record = self.planning_workspace_port.stage_planning_draft_files(
            workspace_dir,
            &draft_name,
            &bootstrap.files,
        )?;
        Ok(PlanningInitStageResult {
            mode,
            draft_name: stage_record.draft_name,
            draft_directory: stage_record.draft_directory,
            staged_files: stage_record.staged_files.clone(),
            staged_file_count: stage_record.staged_files.len(),
            validation_report: bootstrap.validation_report,
        })
    }
    fn stage_editor_session(
        &self,
        workspace_dir: &str,
        mode: PlanningBootstrapMode,
    ) -> Result<PlanningDraftEditorSession> {
        // editor session은 얇은 composition이다. bootstrap draft를 stage한 뒤 common draft-view projection으로 다시
        // load해, 새로 만든 draft와 기존 draft load가 같은 session shape를 갖게 한다.
        let staged = self.stage_draft(workspace_dir, mode)?;
        self.load_manual_editor_session(workspace_dir, &staged.draft_name)
    }

    fn validate_loaded_draft(
        &self,
        workspace_dir: &str,
        loaded: &PlanningDraftLoadRecord,
    ) -> Result<PlanningValidationReport> {
        Ok(self
            .validate_loaded_draft_result(workspace_dir, loaded)?
            .report)
    }

    fn validate_loaded_draft_result(
        &self,
        workspace_dir: &str,
        loaded: &PlanningDraftLoadRecord,
    ) -> Result<crate::domain::planning::PlanningValidationResult> {
        // draft validation은 accepted direction authority가 있으면 그것을 사용하고, 첫 manual draft처럼 아직 authority가
        // 없으면 Detail bootstrap으로 fallback한다.
        let directions = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.directions)
            .unwrap_or_else(|| {
                self.planning_bootstrap_service
                    .build_artifacts_for_mode(PlanningBootstrapMode::Detail)
                    .directions
            });
        // staged map은 editable/supporting file body의 유일한 source다. active workspace file은 의도적으로 무시해
        // draft가 promotion 전에 내부적으로 complete한지 검증한다.
        let staged_file_map = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<HashMap<_, _>>();
        let task_authority_json = default_empty_task_authority_json();
        // manual bootstrap draft는 task authority editing을 노출하지 않는다. direction/result-output과 supporting-file
        // reference를 검증하기 위해 empty valid authority document를 중립 입력으로 사용한다.
        let mut result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions: &directions,
                task_authority_json: &task_authority_json,
                result_output_markdown: staged_file_map
                    .get(RESULT_OUTPUT_FILE_PATH)
                    .copied()
                    .unwrap_or_default(),
            },
        );
        if let Some(directions) = result.directions.as_ref() {
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    // supporting doc은 draft에 stage되어 있을 때만 present로 본다. bootstrap plan이 supporting path를
                    // 가리키면서 실제 file을 포함하지 않는 경우를 여기서 잡아낸다.
                    |path| staged_file_map.contains_key(path),
                    &mut result.report,
                );
        }
        Ok(result)
    }
    fn initialize_workspace(
        &self,
        workspace_dir: &str,
        mode: PlanningBootstrapMode,
    ) -> Result<PlanningWorkspaceInitResult> {
        // direct init은 기존 active workspace 위에서 실행되지 않는다. 의도적인 교체는 reset이나 draft promotion이
        // 담당해야 하며, init은 "처음 만드는" 경로로 남긴다.
        if self.has_planning_workspace(workspace_dir)? {
            anyhow::bail!(
                "planning workspace already exists; reset or reuse the existing workspace instead"
            );
        }
        let bootstrap = self.prepare_bootstrap_workspace(mode);
        if !bootstrap.validation_report.is_valid() {
            // file이나 authority state를 쓰기 전에 fail-fast한다. bootstrap validation error는 operator가 고쳐야 하는
            // configuration 문제이므로 partial write를 남기지 않는다.
            let first_error = bootstrap
                .validation_report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning bootstrap validation failed".to_string());
            anyhow::bail!("planning bootstrap validation failed: {first_error}");
        }
        // initialization 성공 경로에서 accepted authority가 missing bootstrap markdown을 가리키지 않도록 file write를
        // authority commit보다 먼저 수행한다.
        for file in &bootstrap.files {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    &file.active_path,
                    Some(&file.body),
                )?;
        }
        self.commit_direction_authority_from_bootstrap(workspace_dir, &bootstrap.directions)?;
        self.commit_task_authority_from_bootstrap(
            workspace_dir,
            &bootstrap.directions,
            &bootstrap.task_authority,
        )?;
        Ok(PlanningWorkspaceInitResult {
            mode,
            created_file_count: bootstrap.files.len(),
            created_paths: bootstrap
                .files
                .iter()
                .map(|file| file.active_path.clone())
                .collect(),
        })
    }
    fn prepare_bootstrap_workspace(&self, mode: PlanningBootstrapMode) -> BootstrapWorkspacePlan {
        // pure bootstrap artifact set을 staging과 direct init이 공통으로 소비하는 concrete workspace plan으로 바꾼다.
        // 이 함수가 두 경로의 validation 기준을 하나로 묶는다.
        let artifacts = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(mode);
        let task_authority_json = serde_json::to_string(&artifacts.task_authority)
            .expect("bootstrap task authority should serialize");
        // draft-specific path metadata를 붙이기 전에 accepted authority로 commit될 정확한 문서 조합을 검증한다.
        let mut validation_result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions: &artifacts.directions,
                task_authority_json: &task_authority_json,
                result_output_markdown: &artifacts.result_output_markdown,
            },
        );
        if let Some(directions) = validation_result.directions.as_ref() {
            // bootstrap validation 중 사용할 수 있는 supporting file은 supplemental_files뿐이다. direction catalog가
            // seed plan에 없는 detail doc이나 prompt file을 가리키는 경우를 여기서 잡는다.
            let staged_supporting_paths = artifacts
                .supplemental_files
                .iter()
                .map(|file| file.active_path.as_str())
                .collect::<Vec<_>>();
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| staged_supporting_paths.contains(&path),
                    &mut validation_result.report,
                );
        }
        // workspace-backed file만 draft/active file list에 들어간다. DB authority document는 아래 structured field에
        // 남겨 free-form markdown write 경로와 섞이지 않게 한다.
        let mut files = vec![PlanningDraftFileRecord {
            active_path: artifacts.result_output_path,
            body: artifacts.result_output_markdown,
        }];
        files.extend(artifacts.supplemental_files.into_iter().map(Into::into));

        BootstrapWorkspacePlan {
            files,
            directions: artifacts.directions,
            task_authority: artifacts.task_authority,
            validation_report: validation_result.report,
        }
    }
    fn commit_direction_authority_from_bootstrap(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
    ) -> Result<()> {
        // bootstrap과 draft promotion은 validation 뒤 accepted direction authority를 교체하는 system-owned rewrite다.
        // editor session의 optimistic revision check를 사용하지 않는 이유다.
        self.planning_task_repository_port
            .commit_direction_authority_snapshot(
                workspace_dir,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions,
                },
            )
            .map(|_| ())
    }
    fn commit_task_authority_from_bootstrap(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
    ) -> Result<()> {
        // queue projection은 task authority와 같은 boundary에서 파생한다. accepted task state와 scheduler-facing
        // projection이 서로 다른 시점의 데이터를 보지 않게 하기 위해서다.
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| anyhow!("valid bootstrap queue build failed: {error}"))?;
        // bootstrap은 complete system-owned authority snapshot을 seed한다. task-level mutation command는 incremental
        // change용이므로 이 초기화 경로에서는 의도적으로 우회한다.
        self.planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority,
                    queue_projection: &queue_projection,
                },
            )
            .map(|_| ())
    }
}

struct BootstrapWorkspacePlan {
    // internal plan은 validation 이후, staging 또는 direct initialization 이전의 workspace file과 DB authority
    // document를 함께 보관한다.
    files: Vec<PlanningDraftFileRecord>,
    directions: DirectionCatalogDocument,
    task_authority: TaskAuthorityDocument,
    validation_report: PlanningValidationReport,
}

fn is_operator_editable_draft_path(active_path: &str) -> bool {
    // manual init editor는 의도적으로 좁다. authority JSON은 bootstrap struct와 validation에서 파생되며,
    // free-form text로 편집하지 않는다.
    matches!(active_path, RESULT_OUTPUT_FILE_PATH)
}

fn default_empty_task_authority_json() -> String {
    // 이 surface가 direction/result-output만 편집하더라도 validation에는 task-authority document가 필요하다. 빈
    // versioned authority가 그 검사에 대한 neutral document다.
    serde_json::to_string(&TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: Vec::new(),
    })
    .expect("empty task authority should serialize")
}

fn build_bootstrap_draft_name(now: chrono::DateTime<Utc>) -> String {
    // timestamp와 nanoseconds를 함께 써서 동시에 stage된 bootstrap draft를 구분한다. 동시에 operator가 생성 시각을
    // 이름에서 바로 볼 수 있게 한다.
    format!(
        "bootstrap-{}Z-{:09}",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    )
}
#[cfg(test)]
mod tests {
    use super::is_operator_editable_draft_path;
    use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
    #[test]
    fn operator_editable_draft_paths_exclude_task_authority_artifacts() {
        // UI boundary를 고정한다. structured authority editing이 이 surface에 추가되기 전까지 manual bootstrap
        // editor에는 result-output만 들어가야 한다.
        assert!(is_operator_editable_draft_path(RESULT_OUTPUT_FILE_PATH));
        assert!(!is_operator_editable_draft_path(
            ".codex-exec-loop/planning/direction-authority"
        ));
        assert!(!is_operator_editable_draft_path("DB task authority"));
    }
}
