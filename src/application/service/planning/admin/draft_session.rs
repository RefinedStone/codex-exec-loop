use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use chrono::Utc;

use super::projection::{map_queue_preview, map_validation_report};
use super::{
    PlanningAdminDraftFileView, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFacadeService, PlanningAdminFileKey,
    PlanningAdminQueuePreview, PlanningAdminSessionView, PlanningAdminValidationView,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord,
};
use crate::application::service::planning::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY, PlanningDraftEditorFile, PlanningDraftPromoteResult,
    PlanningDraftSaveResult, RESULT_OUTPUT_FILE_PATH,
};
use crate::domain::planning::{DirectionCatalogDocument, PlanningFileKind, PlanningWorkspaceFiles};

/*
 * draft session은 admin editor의 isolation layer다. operator가 result-output, queue-idle prompt, direction
 * detail을 고치는 동안 active planning file은 그대로 두고 staged copy만 수정한다. 이 파일의 핵심 책임은
 * "어떤 active path가 어떤 editor surface에 보이는가", "staged content와 current authority를 어떻게 합쳐
 * validation하는가", "promote 전에 queue preview를 얼마나 신뢰할 수 있는가"를 한 경계에서 유지하는 것이다.
 */
impl PlanningAdminFacadeService {
    pub fn create_draft_session(
        &self,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
    ) -> Result<PlanningAdminSessionView> {
        // draft kind마다 staging source가 다르다. full planning은 현재 accepted support files를 모아 수동 editor
        // draft를 만들고, queue-idle/detail draft는 workspace use case가 알고 있는 전용 staging workflow를 호출한다.
        // facade는 workflow 선택 뒤 공통 session view를 다시 로드해 모든 draft kind가 같은 응답 구조를 갖게 한다.
        let draft_name = match kind {
            PlanningAdminDraftKind::FullPlanning => self.stage_active_manual_editor_draft()?,
            PlanningAdminDraftKind::QueueIdlePrompt => {
                self.planning
                    .workspace
                    .stage_queue_idle_prompt_editor_session(self.workspace_dir.as_str())?
                    .draft_name
            }
            PlanningAdminDraftKind::DirectionDetail => {
                self.planning
                    .workspace
                    .stage_detail_doc_editor_session(
                        self.workspace_dir.as_str(),
                        direction_id.ok_or_else(|| {
                            anyhow!("direction detail drafts require direction_id")
                        })?,
                    )?
                    .draft_name
            }
        };
        self.load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name,
            kind,
            direction_id: direction_id.map(str::to_string),
        })
    }
    pub fn load_draft_session(
        &self,
        request: PlanningAdminDraftLoadRequest,
    ) -> Result<PlanningAdminSessionView> {
        // draft load도 default authority를 먼저 보장한다. editor가 단순히 staged file을 여는 작업처럼 보여도
        // session view에는 validation과 queue preview가 포함되므로 half-initialized workspace를 기준으로 계산하면 안 된다.
        self.ensure_default_authority()?;
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(self.workspace_dir.as_str(), &request.draft_name)?;
        self.build_session_view(request.kind, request.direction_id, loaded)
    }
    pub fn save_draft(
        &self,
        request: PlanningAdminDraftMutationRequest,
    ) -> Result<(PlanningDraftSaveResult, PlanningAdminSessionView)> {
        // save는 현재 editor surface에 보이는 file만 저장한다. draft directory 안에 다른 staged artifact가 있더라도
        // specialized editor가 숨겨진 파일을 body 없음으로 덮어쓰지 않게 visible-file resolution을 먼저 수행한다.
        let visible_files = self.resolve_mutated_visible_files(&request)?;
        let result = self.planning.workspace.save_draft_editor_files(
            self.workspace_dir.as_str(),
            &request.draft_name,
            &visible_files,
        )?;
        let session = self.load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name: request.draft_name,
            kind: request.kind,
            direction_id: request.direction_id,
        })?;
        Ok((result, session))
    }
    pub fn promote_draft(
        &self,
        request: PlanningAdminDraftMutationRequest,
    ) -> Result<(PlanningDraftPromoteResult, PlanningAdminSessionView)> {
        // promote도 save와 같은 visible-file resolution을 사용한다. 저장과 승격이 서로 다른 file selection 정책을
        // 가지면 "저장은 되었지만 promote 때 다른 파일이 반영되는" 위험이 생기므로 두 경로를 같은 helper에 묶는다.
        let visible_files = self.resolve_mutated_visible_files(&request)?;
        let result = self.planning.workspace.promote_draft_editor_files(
            self.workspace_dir.as_str(),
            &request.draft_name,
            &visible_files,
        )?;
        let session = self.load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name: request.draft_name,
            kind: request.kind,
            direction_id: request.direction_id,
        })?;
        Ok((result, session))
    }
    pub(super) fn resolve_mutated_visible_files(
        &self,
        request: &PlanningAdminDraftMutationRequest,
    ) -> Result<Vec<PlanningDraftEditorFile>> {
        // posted editor body와 storage의 staged record를 병합한다. request는 key/body만 알고 있고 active/staged path
        // pairing은 workspace storage가 authority이므로, path는 loaded record에서 보존하고 body만 request 값으로 교체한다.
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(self.workspace_dir.as_str(), &request.draft_name)?;
        let update_map = request
            .files
            .iter()
            .map(|update| (update.key, update.body.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut files = Vec::with_capacity(loaded.staged_files.len());
        for file in loaded.staged_files {
            let Some(key) = file_key_for_kind(request.kind, &file.active_path) else {
                continue;
            };
            files.push(PlanningDraftEditorFile {
                active_path: file.active_path,
                staged_path: file.staged_path,
                body: update_map.get(&key).cloned().unwrap_or(file.body),
            });
        }
        Ok(files)
    }
    pub(super) fn build_session_view(
        &self,
        kind: PlanningAdminDraftKind,
        direction_id: Option<String>,
        loaded: PlanningDraftLoadRecord,
    ) -> Result<PlanningAdminSessionView> {
        // session rendering은 staged files를 selected surface에 맞게 filter한 뒤, 같은 staged content로 계산한
        // validation/queue preview를 붙인다. editor가 보고 있는 body와 validation이 서로 다른 source에서 나오면
        // promote 전 판단이 불가능해진다.
        let validation = self.validate_loaded_draft(&loaded)?;
        let files = loaded
            .staged_files
            .into_iter()
            .filter_map(|file| {
                file_key_for_kind(kind, &file.active_path).map(|key| PlanningAdminDraftFileView {
                    key,
                    label: key.label().to_string(),
                    active_path: file.active_path,
                    editor_language: key.editor_language().to_string(),
                    body: file.body,
                })
            })
            .collect::<Vec<_>>();
        Ok(PlanningAdminSessionView {
            kind,
            direction_id,
            draft_name: loaded.draft_name,
            draft_directory: loaded.draft_directory,
            editor_heading: kind.editor_heading().to_string(),
            return_path: kind.return_path().to_string(),
            files,
            validation: validation.validation,
            queue_preview: validation.queue_preview,
        })
    }
    fn validate_loaded_draft(
        &self,
        loaded: &PlanningDraftLoadRecord,
    ) -> Result<PlanningAdminDraftValidationSnapshot> {
        // validation은 staged edit와 current authority를 합성한다. direction/task authority는 DB snapshot이 원천이고,
        // result_output은 staged body가 있으면 그것을 우선한다. 이 조합 덕분에 prompt/detail만 편집하는 draft도
        // 전체 planning workspace 관점의 validation 결과를 받을 수 있다.
        let staged_files = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<BTreeMap<_, _>>();
        let directions = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(self.workspace_dir.as_str())?
            .ok_or_else(|| anyhow!("default planning authority seed did not provide directions"))?
            .directions;
        let task_authority_json = self
            .planning_task_repository_port
            .load_task_authority_snapshot(self.workspace_dir.as_str())?
            .map(|snapshot| serde_json::to_string(&snapshot.task_authority))
            .transpose()?
            .unwrap_or_else(|| "{\"version\":1,\"tasks\":[]}".to_string());
        let result_output_markdown = self.load_effective_draft_body(
            &staged_files,
            RESULT_OUTPUT_FILE_PATH,
            PlanningFileKind::ResultOutput,
        )?;
        let mut result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions: &directions,
                    task_authority_json: &task_authority_json,
                    result_output_markdown: &result_output_markdown,
                });
        if let Some(directions) = result.directions.as_ref() {
            // supporting file validation은 staged file을 먼저 보고, 없을 때 active workspace file로 fallback한다.
            // draft가 detail document를 새로 만들거나 고치는 중이면 active file만 검사해서는 promote 전에 오류가
            // 사라졌는지 확인할 수 없다.
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| {
                        staged_files.contains_key(path)
                            || self
                                .planning_workspace_port
                                .load_optional_planning_file(self.workspace_dir.as_str(), path)
                                .ok()
                                .flatten()
                                .is_some()
                    },
                    &mut result.report,
                );
        }
        let queue_preview = if result.report.is_valid() {
            // queue preview는 intentionally best-effort다. validation이 실패한 draft에서는 queue build 오류를 덧씌우지
            // 않고 validation issue를 먼저 보여준다. 유효한 draft에서만 projection을 시도해 promote 후 queue 모습을
            // 참고 정보로 제공한다.
            match (result.directions.as_ref(), result.task_authority.as_ref()) {
                (Some(directions), Some(task_authority)) => self
                    .priority_queue_service
                    .build_projection(directions, task_authority)
                    .ok()
                    .map(|projection| map_queue_preview(&projection)),
                _ => None,
            }
        } else {
            None
        };
        Ok(PlanningAdminDraftValidationSnapshot {
            validation: map_validation_report(&result.report),
            queue_preview,
        })
    }
    fn load_effective_draft_body<'a>(
        &self,
        staged_files: &BTreeMap<&'a str, &'a str>,
        path: &'static str,
        file_kind: PlanningFileKind,
    ) -> Result<String> {
        // core file은 staged content를 우선하지만 없으면 active workspace content로 fallback한다. queue-idle/detail처럼
        // 좁은 editor도 result-output을 포함한 전체 workspace validation을 받아야 하기 때문이다.
        if let Some(body) = staged_files.get(path) {
            return Ok((*body).to_string());
        }
        self.planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), path)?
            .ok_or_else(|| missing_core_draft_file_error(path, file_kind))
    }
    pub(super) fn stage_active_manual_editor_draft(&self) -> Result<String> {
        // full planning draft는 result_output과 direction authority가 현재 참조하는 supporting prompt/detail file을
        // 함께 stage한다. manual editor가 accepted planning context 전체를 한 draft에서 점검할 수 있게 하는 경로다.
        self.ensure_default_authority()?;
        let directions = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(self.workspace_dir.as_str())?
            .ok_or_else(|| anyhow!("default planning authority seed did not provide directions"))?
            .directions;
        let result_output_markdown = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), RESULT_OUTPUT_FILE_PATH)?
            .ok_or_else(|| {
                anyhow!("default planning authority seed did not provide result output")
            })?;
        let mut files = vec![PlanningDraftFileRecord {
            active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            body: result_output_markdown,
        }];
        let supporting_paths = collect_direction_supporting_paths(&directions);
        for path in supporting_paths {
            // 참조는 되어 있지만 아직 active file이 없는 supporting path는 stage하지 않는다. validation 단계에서 missing
            // issue로 보고되며, draft directory에 빈 파일을 몰래 만들어 accepted state와 다른 의미를 부여하지 않는다.
            if let Some(body) = self
                .planning_workspace_port
                .load_optional_planning_file(self.workspace_dir.as_str(), &path)?
            {
                files.push(PlanningDraftFileRecord {
                    active_path: path,
                    body,
                });
            }
        }
        let now = Utc::now();
        let draft_name = format!(
            "admin-{}Z-{:09}",
            now.format("%Y%m%dT%H%M%S"),
            now.timestamp_subsec_nanos()
        );
        self.planning_workspace_port.stage_planning_draft_files(
            self.workspace_dir.as_str(),
            &draft_name,
            &files,
        )?;
        Ok(draft_name)
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlanningAdminDraftValidationSnapshot {
    // validation은 admin surface용 DTO다. domain validation report를 직접 들고 있지 않아 session response가
    // projection 계층의 stable string contract를 따른다.
    validation: PlanningAdminValidationView,
    // queue_preview는 draft가 valid할 때만 채워지는 보조 예측이다. None은 queue 계산 실패 또는 invalid draft를
    // 모두 표현하므로 caller는 validation을 우선 표시해야 한다.
    queue_preview: Option<PlanningAdminQueuePreview>,
}

fn collect_direction_supporting_paths(directions: &DirectionCatalogDocument) -> Vec<String> {
    // authority가 참조하는 supporting file path를 중복 없이 정렬된 순서로 모은다. 같은 prompt/detail file을 여러
    // direction이 공유해도 draft에는 한 번만 들어가고, stage order가 안정적이라 diff와 test fixture가 흔들리지 않는다.
    let mut paths = BTreeSet::new();
    let prompt_path = directions.queue_idle.prompt_path.trim();
    if !prompt_path.is_empty() {
        paths.insert(prompt_path.to_string());
    }
    for direction in &directions.directions {
        let detail_doc_path = direction.detail_doc_path.trim();
        if !detail_doc_path.is_empty() {
            paths.insert(detail_doc_path.to_string());
        }
    }

    paths.into_iter().collect()
}

fn missing_core_draft_file_error(path: &'static str, file_kind: PlanningFileKind) -> anyhow::Error {
    // core draft file missing은 validation issue가 아니라 session construction 실패다. result_output 같은 핵심 문서가
    // staged에도 active workspace에도 없으면 editor가 보여 줄 기준 content가 없기 때문이다.
    anyhow!(
        "draft is missing required {} content at {}",
        match file_kind {
            PlanningFileKind::Directions => "directions",
            PlanningFileKind::TaskAuthority => "task authority",
            PlanningFileKind::ResultOutput => "result output",
        },
        path
    )
}

fn file_key_for_kind(
    kind: PlanningAdminDraftKind,
    active_path: &str,
) -> Option<PlanningAdminFileKey> {
    // active path는 staged record를 editor pane key로 분류한다. 이어지는 kind match는 full/queue-idle/detail editor가
    // 자기 surface에 속한 파일만 보게 하는 두 번째 guard다.
    let key = if active_path == RESULT_OUTPUT_FILE_PATH {
        PlanningAdminFileKey::ResultOutput
    } else if active_path == DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
        || active_path.starts_with(&format!("{PLANNING_PROMPTS_DIRECTORY}/"))
    {
        PlanningAdminFileKey::QueueIdlePrompt
    } else if active_path.starts_with(&format!("{PLANNING_DIRECTION_DOCS_DIRECTORY}/")) {
        PlanningAdminFileKey::DirectionDetail
    } else {
        return None;
    };
    match kind {
        PlanningAdminDraftKind::FullPlanning => {
            matches!(key, PlanningAdminFileKey::ResultOutput).then_some(key)
        }
        PlanningAdminDraftKind::QueueIdlePrompt => {
            matches!(key, PlanningAdminFileKey::QueueIdlePrompt).then_some(key)
        }
        PlanningAdminDraftKind::DirectionDetail => {
            matches!(key, PlanningAdminFileKey::DirectionDetail).then_some(key)
        }
    }
}
