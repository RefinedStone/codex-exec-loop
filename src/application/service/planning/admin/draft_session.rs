use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use chrono::Utc;

use super::facade::PlanningAdminDraftStageMetadata;
use super::projection::{map_queue_preview, map_validation_report};
use super::{
    PlanningAdminDraftFileView, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFacadeService, PlanningAdminFileKey,
    PlanningAdminQueuePreview, PlanningAdminSessionView, PlanningAdminValidationView,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord,
};
#[cfg(test)]
use crate::application::service::planning::DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH;
use crate::application::service::planning::{
    PlanningDraftEditorFile, PlanningDraftPromoteResult, PlanningDraftSaveResult,
    RESULT_OUTPUT_FILE_PATH, validate_planning_draft_name,
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
        if kind == PlanningAdminDraftKind::DirectionDetail && direction_id.is_none() {
            return Err(anyhow!("direction detail drafts require direction_id"));
        }
        self.require_atomic_draft_editor()?;
        // draft kind마다 staging source가 다르다. full planning은 현재 accepted support files를 모아 수동 editor
        // draft를 만들고, queue-idle/detail draft는 workspace use case가 알고 있는 전용 staging workflow를 호출한다.
        // facade는 workflow 선택 뒤 공통 session view를 다시 로드해 모든 draft kind가 같은 응답 구조를 갖게 한다.
        let (draft_name, source_planning_revision, editable_active_paths) = match kind {
            PlanningAdminDraftKind::FullPlanning => {
                let (draft_name, source_planning_revision) =
                    self.stage_active_manual_editor_draft()?;
                (
                    draft_name,
                    source_planning_revision,
                    BTreeSet::from([RESULT_OUTPUT_FILE_PATH.to_string()]),
                )
            }
            PlanningAdminDraftKind::QueueIdlePrompt => {
                let session = self
                    .planning
                    .workspace
                    .stage_queue_idle_prompt_editor_session(self.workspace_dir.as_str())?;
                let editable_active_paths = session
                    .editable_files
                    .iter()
                    .map(|file| file.active_path.clone())
                    .collect();
                (
                    session.draft_name,
                    session.source_planning_revision.ok_or_else(|| {
                        anyhow!("queue-idle prompt draft source revision is unavailable")
                    })?,
                    editable_active_paths,
                )
            }
            PlanningAdminDraftKind::DirectionDetail => {
                let session = self.planning.workspace.stage_detail_doc_editor_session(
                    self.workspace_dir.as_str(),
                    direction_id
                        .ok_or_else(|| anyhow!("direction detail drafts require direction_id"))?,
                )?;
                let editable_active_paths = session
                    .editable_files
                    .iter()
                    .map(|file| file.active_path.clone())
                    .collect();
                (
                    session.draft_name,
                    session.source_planning_revision.ok_or_else(|| {
                        anyhow!("direction detail draft source revision is unavailable")
                    })?,
                    editable_active_paths,
                )
            }
        };
        self.remember_draft_stage_metadata(
            &draft_name,
            kind,
            direction_id,
            source_planning_revision,
            editable_active_paths,
        )?;
        self.load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name,
            kind,
            direction_id: direction_id.map(str::to_string),
        })
    }
    fn require_atomic_draft_editor(&self) -> Result<()> {
        if self
            .planning_workspace_port
            .uses_repo_scoped_authority(self.workspace_dir.as_str())
            && self
                .planning_authority_port
                .supports_atomic_planning_authority_documents()
        {
            return Ok(());
        }
        Err(anyhow!(
            "planning maintenance editors require the atomic workspace authority"
        ))
    }
    pub fn load_draft_session(
        &self,
        request: PlanningAdminDraftLoadRequest,
    ) -> Result<PlanningAdminSessionView> {
        ensure_valid_draft_name(&request.draft_name)?;
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
        ensure_valid_draft_name(&request.draft_name)?;
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
        ensure_valid_draft_name(&request.draft_name)?;
        let metadata = self.require_draft_stage_metadata(
            &request.draft_name,
            request.kind,
            request.direction_id.as_deref(),
        )?;
        // promote도 save와 같은 visible-file resolution을 사용한다. 저장과 승격이 서로 다른 file selection 정책을
        // 가지면 "저장은 되었지만 promote 때 다른 파일이 반영되는" 위험이 생기므로 두 경로를 같은 helper에 묶는다.
        let visible_files = self.resolve_mutated_visible_files(&request)?;
        let source_planning_revision = metadata.source_planning_revision;
        let result = self
            .planning
            .workspace
            .promote_draft_editor_files_at_revision(
                self.workspace_dir.as_str(),
                &request.draft_name,
                &visible_files,
                source_planning_revision,
            )?;
        if let Some(committed_planning_revision) = result.committed_planning_revision {
            self.advance_draft_stage_revision(
                &request.draft_name,
                source_planning_revision,
                committed_planning_revision,
            )?;
        }
        let session = self.load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name: request.draft_name,
            kind: request.kind,
            direction_id: request.direction_id,
        })?;
        Ok((result, session))
    }
    fn remember_draft_stage_metadata(
        &self,
        draft_name: &str,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
        source_planning_revision: i64,
        editable_active_paths: BTreeSet<String>,
    ) -> Result<()> {
        let metadata = PlanningAdminDraftStageMetadata {
            kind,
            direction_id: direction_id.map(str::to_string),
            source_planning_revision,
            editable_active_paths,
        };
        self.draft_stage_metadata
            .lock()
            .map_err(|_| anyhow!("admin draft metadata store is unavailable"))?
            .insert(draft_name.to_string(), metadata);
        Ok(())
    }
    fn require_draft_stage_metadata(
        &self,
        draft_name: &str,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
    ) -> Result<PlanningAdminDraftStageMetadata> {
        let metadata = self.load_draft_stage_metadata(draft_name)?.ok_or_else(|| {
            anyhow!("admin draft metadata is unavailable for `{draft_name}`; create a new draft")
        })?;
        Self::ensure_draft_stage_identity(&metadata, kind, direction_id)?;
        Ok(metadata)
    }
    fn load_draft_stage_metadata(
        &self,
        draft_name: &str,
    ) -> Result<Option<PlanningAdminDraftStageMetadata>> {
        Ok(self
            .draft_stage_metadata
            .lock()
            .map_err(|_| anyhow!("admin draft metadata store is unavailable"))?
            .get(draft_name)
            .cloned())
    }
    fn ensure_draft_stage_identity(
        metadata: &PlanningAdminDraftStageMetadata,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
    ) -> Result<()> {
        if metadata.kind != kind || metadata.direction_id.as_deref() != direction_id {
            return Err(anyhow!(
                "admin draft identity does not match its server-side stage metadata; create a new draft"
            ));
        }
        Ok(())
    }
    fn resolve_editable_active_paths(
        &self,
        draft_name: &str,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
    ) -> Result<BTreeSet<String>> {
        if let Some(metadata) = self.load_draft_stage_metadata(draft_name)? {
            Self::ensure_draft_stage_identity(&metadata, kind, direction_id)?;
            return Ok(metadata.editable_active_paths);
        }
        match kind {
            PlanningAdminDraftKind::FullPlanning => {
                Ok(BTreeSet::from([RESULT_OUTPUT_FILE_PATH.to_string()]))
            }
            PlanningAdminDraftKind::QueueIdlePrompt => {
                let directions = self.load_current_direction_catalog()?;
                let prompt_path = directions.queue_idle.prompt_path.trim();
                if prompt_path.is_empty() {
                    return Err(anyhow!("queue-idle prompt path is unavailable"));
                }
                Ok(BTreeSet::from([prompt_path.to_string()]))
            }
            PlanningAdminDraftKind::DirectionDetail => {
                let direction_id = direction_id
                    .ok_or_else(|| anyhow!("direction detail drafts require direction_id"))?
                    .trim();
                let directions = self.load_current_direction_catalog()?;
                let direction = directions
                    .directions
                    .iter()
                    .find(|direction| direction.id.trim() == direction_id)
                    .ok_or_else(|| anyhow!("unknown direction id: {direction_id}"))?;
                let detail_doc_path = direction.detail_doc_path.trim();
                if detail_doc_path.is_empty() {
                    return Err(anyhow!(
                        "direction `{direction_id}` detail document path is unavailable"
                    ));
                }
                Ok(BTreeSet::from([detail_doc_path.to_string()]))
            }
        }
    }
    fn load_current_direction_catalog(&self) -> Result<DirectionCatalogDocument> {
        self.planning_task_repository_port
            .load_direction_authority_snapshot(self.workspace_dir.as_str())?
            .map(|snapshot| snapshot.directions)
            .ok_or_else(|| anyhow!("default planning authority seed did not provide directions"))
    }
    fn advance_draft_stage_revision(
        &self,
        draft_name: &str,
        source_planning_revision: i64,
        committed_planning_revision: i64,
    ) -> Result<()> {
        let mut metadata_by_draft = self
            .draft_stage_metadata
            .lock()
            .map_err(|_| anyhow!("admin draft metadata store is unavailable"))?;
        let metadata = metadata_by_draft.get_mut(draft_name).ok_or_else(|| {
            anyhow!("admin draft metadata is unavailable for `{draft_name}`; create a new draft")
        })?;
        if metadata.source_planning_revision != source_planning_revision {
            return Err(anyhow!(
                "admin draft source planning revision changed while promotion completed; reload and retry"
            ));
        }
        metadata.source_planning_revision = committed_planning_revision;
        Ok(())
    }
    pub(super) fn resolve_mutated_visible_files(
        &self,
        request: &PlanningAdminDraftMutationRequest,
    ) -> Result<Vec<PlanningDraftEditorFile>> {
        ensure_valid_draft_name(&request.draft_name)?;
        // posted editor body와 storage의 staged record를 병합한다. request는 key/body만 알고 있고 active/staged path
        // pairing은 workspace storage가 authority이므로, path는 loaded record에서 보존하고 body만 request 값으로 교체한다.
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(self.workspace_dir.as_str(), &request.draft_name)?;
        let editable_active_paths = self.resolve_editable_active_paths(
            &request.draft_name,
            request.kind,
            request.direction_id.as_deref(),
        )?;
        let update_map = request
            .files
            .iter()
            .map(|update| (update.key, update.body.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut files = Vec::with_capacity(loaded.staged_files.len());
        for file in loaded.staged_files {
            if !editable_active_paths.contains(&file.active_path) {
                continue;
            }
            let key = file_key_for_kind(request.kind);
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
        let editable_active_paths =
            self.resolve_editable_active_paths(&loaded.draft_name, kind, direction_id.as_deref())?;
        let validation = self.validate_loaded_draft(&loaded)?;
        let files = loaded
            .staged_files
            .into_iter()
            .filter(|file| editable_active_paths.contains(&file.active_path))
            .map(|file| {
                let key = file_key_for_kind(kind);
                PlanningAdminDraftFileView {
                    key,
                    label: key.label().to_string(),
                    active_path: file.active_path,
                    editor_language: key.editor_language().to_string(),
                    body: file.body,
                }
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
    pub(super) fn stage_active_manual_editor_draft(&self) -> Result<(String, i64)> {
        // full planning draft는 result_output과 direction authority가 현재 참조하는 supporting prompt/detail file을
        // 함께 stage한다. manual editor가 accepted planning context 전체를 한 draft에서 점검할 수 있게 하는 경로다.
        self.ensure_default_authority()?;
        let direction_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(self.workspace_dir.as_str())?
            .ok_or_else(|| anyhow!("default planning authority seed did not provide directions"))?;
        let source_planning_revision = direction_snapshot.planning_revision;
        let directions = direction_snapshot.directions;
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
        Ok((draft_name, source_planning_revision))
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

fn ensure_valid_draft_name(draft_name: &str) -> Result<()> {
    validate_planning_draft_name(draft_name)
        .map_err(|error| anyhow!("invalid planning draft name `{draft_name}`: {error}"))
}

fn file_key_for_kind(kind: PlanningAdminDraftKind) -> PlanningAdminFileKey {
    match kind {
        PlanningAdminDraftKind::FullPlanning => PlanningAdminFileKey::ResultOutput,
        PlanningAdminDraftKind::QueueIdlePrompt => PlanningAdminFileKey::QueueIdlePrompt,
        PlanningAdminDraftKind::DirectionDetail => PlanningAdminFileKey::DirectionDetail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_authority_port::{
        NoopPlanningAuthorityPort, PlanningAuthorityPort,
    };
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
        PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftLoadFileRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning::admin::PlanningAdminDraftFileUpdate;
    use crate::application::service::planning::{
        PLANNING_DIRECTION_DOCS_DIRECTORY, PlanningServices,
    };
    use crate::domain::planning::{
        DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION, QueueIdleConfig,
        QueueIdlePolicy,
    };
    use std::fs;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn draft_kind_maps_to_its_editor_file_key() {
        assert_eq!(
            file_key_for_kind(PlanningAdminDraftKind::FullPlanning),
            PlanningAdminFileKey::ResultOutput
        );
        assert_eq!(
            file_key_for_kind(PlanningAdminDraftKind::QueueIdlePrompt),
            PlanningAdminFileKey::QueueIdlePrompt
        );
        assert_eq!(
            file_key_for_kind(PlanningAdminDraftKind::DirectionDetail),
            PlanningAdminFileKey::DirectionDetail
        );
    }

    #[test]
    fn supporting_path_collection_deduplicates_and_sorts_authority_references() {
        let directions = direction_catalog_with_supporting_paths();

        assert_eq!(
            collect_direction_supporting_paths(&directions),
            vec![
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
                "directions/general.md".to_string(),
                "directions/release.md".to_string(),
            ]
        );
    }

    #[test]
    fn supporting_path_collection_ignores_blank_authority_references() {
        let mut directions = direction_catalog_with_supporting_paths();
        directions.queue_idle.prompt_path = "   ".to_string();
        directions.directions[0].detail_doc_path = "\t".to_string();
        directions.directions[1].detail_doc_path = " directions/release.md ".to_string();
        directions.directions.truncate(2);

        assert_eq!(
            collect_direction_supporting_paths(&directions),
            vec!["directions/release.md".to_string()]
        );
    }

    #[test]
    fn missing_core_draft_file_error_labels_all_core_file_kinds() {
        assert_eq!(
            missing_core_draft_file_error("directions.json", PlanningFileKind::Directions)
                .to_string(),
            "draft is missing required directions content at directions.json"
        );
        assert_eq!(
            missing_core_draft_file_error("task-authority.json", PlanningFileKind::TaskAuthority)
                .to_string(),
            "draft is missing required task authority content at task-authority.json"
        );
        assert_eq!(
            missing_core_draft_file_error(RESULT_OUTPUT_FILE_PATH, PlanningFileKind::ResultOutput)
                .to_string(),
            format!(
                "draft is missing required result output content at {}",
                RESULT_OUTPUT_FILE_PATH
            )
        );
    }

    #[test]
    fn create_full_planning_draft_stages_active_result_output_with_validation() {
        let fixture = TestAdminFixture::new_sqlite("admin-full-draft");

        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::FullPlanning, None)
            .expect("full planning draft should open");

        assert_eq!(session.kind, PlanningAdminDraftKind::FullPlanning);
        assert_eq!(session.editor_heading, "Full Planning Draft");
        assert_eq!(session.return_path, "/admin");
        assert_eq!(session.files.len(), 1);
        assert_eq!(session.files[0].key, PlanningAdminFileKey::ResultOutput);
        assert_eq!(session.files[0].editor_language, "markdown");
        assert!(session.files[0].body.contains("# Result Output Prompt"));
        assert!(session.validation.is_valid);
        assert_eq!(session.validation.error_count, 0);
        assert!(session.queue_preview.is_some());
    }

    #[test]
    fn direct_workspace_rejects_all_admin_draft_staging_before_mutation() {
        let fixture = TestAdminFixture::new("admin-direct-draft-guard");

        for (kind, direction_id) in [
            (PlanningAdminDraftKind::FullPlanning, None),
            (PlanningAdminDraftKind::QueueIdlePrompt, None),
            (
                PlanningAdminDraftKind::DirectionDetail,
                Some("general-workstream"),
            ),
        ] {
            let error = fixture
                .facade
                .create_draft_session(kind, direction_id)
                .expect_err("direct workspace draft staging should fail closed");
            assert_eq!(
                error.to_string(),
                "planning maintenance editors require the atomic workspace authority"
            );
        }

        assert!(
            fixture
                .task_repository_port
                .load_direction_authority_snapshot(&fixture.workspace.path)
                .expect("direction authority should remain readable")
                .is_none()
        );
        assert!(
            fixture
                .task_repository_port
                .load_task_authority_snapshot(&fixture.workspace.path)
                .expect("task authority should remain readable")
                .is_none()
        );
        assert_eq!(
            fs::read_dir(&fixture.workspace.path)
                .expect("workspace should remain readable")
                .count(),
            0
        );
    }

    #[test]
    fn full_planning_draft_skips_missing_supporting_paths_without_empty_staged_files() {
        let fixture = TestAdminFixture::new_sqlite("admin-full-draft-missing-support");
        fixture
            .facade
            .ensure_default_authority()
            .expect("authority seed should be available before custom directions");
        let mut directions = fixture
            .task_repository_port
            .load_direction_authority_snapshot(&fixture.workspace.path)
            .expect("direction authority should load")
            .expect("default direction authority should be seeded")
            .directions;
        directions.directions.push(DirectionDefinition {
            id: "missing-support".to_string(),
            title: "Missing support".to_string(),
            summary: "References a detail doc that is not present yet.".to_string(),
            success_criteria: vec!["Missing detail is reported by validation.".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: "directions/missing.md".to_string(),
            state: DirectionState::Active,
        });
        fixture
            .task_repository_port
            .commit_direction_authority_snapshot(
                &fixture.workspace.path,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions: &directions,
                    authority_mutation_owner_token: None,
                },
            )
            .expect("custom direction authority should commit");

        let (draft_name, _) = fixture
            .facade
            .stage_active_manual_editor_draft()
            .expect("full draft staging should skip missing supporting files");
        let loaded = fixture
            .workspace_port
            .load_planning_draft_files(&fixture.workspace.path, &draft_name)
            .expect("staged draft should load");

        assert!(
            loaded
                .staged_files
                .iter()
                .any(|file| file.active_path == RESULT_OUTPUT_FILE_PATH)
        );
        assert!(
            loaded
                .staged_files
                .iter()
                .all(|file| file.active_path != "directions/missing.md")
        );
    }

    #[test]
    fn full_planning_draft_promote_updates_visible_result_output_and_keeps_hidden_prompt() {
        let fixture = TestAdminFixture::new_sqlite("admin-full-draft-promote");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::FullPlanning, None)
            .expect("full planning draft should open");
        let original_queue_idle_prompt = fixture
            .workspace_port
            .load_optional_planning_file(
                &fixture.workspace.path,
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH,
            )
            .expect("active queue-idle prompt should load")
            .expect("queue-idle prompt should be seeded");
        let edited_result_output =
            "# Result Output Prompt\n\n- Promoted full planning result contract.\n";

        let (promote_result, promoted_session) = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
                files: vec![
                    PlanningAdminDraftFileUpdate {
                        key: PlanningAdminFileKey::ResultOutput,
                        body: edited_result_output.to_string(),
                    },
                    PlanningAdminDraftFileUpdate {
                        key: PlanningAdminFileKey::QueueIdlePrompt,
                        body: "# Ignored Hidden Prompt\n\nThis body must not replace the hidden prompt."
                            .to_string(),
                    },
                ],
            })
            .expect("full planning draft promotion should succeed");

        assert_eq!(promote_result.draft_name, session.draft_name);
        assert_eq!(promote_result.promoted_file_count, 1);
        assert!(promote_result.validation_report.is_valid());
        assert_eq!(promoted_session.files.len(), 1);
        assert_eq!(promoted_session.files[0].body, edited_result_output);
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(&fixture.workspace.path, RESULT_OUTPUT_FILE_PATH)
                .expect("active result output should load")
                .expect("result output should be promoted"),
            edited_result_output
        );
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(
                    &fixture.workspace.path,
                    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
                )
                .expect("active queue-idle prompt should load")
                .expect("queue-idle prompt should remain promoted from staged context"),
            original_queue_idle_prompt
        );
    }

    #[test]
    fn full_planning_draft_promote_rejects_stale_authority_before_active_write() {
        let fixture = TestAdminFixture::new_sqlite("admin-full-draft-stale-revision");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::FullPlanning, None)
            .expect("full planning draft should open");
        let original_result_output = fixture
            .workspace_port
            .load_optional_planning_file(&fixture.workspace.path, RESULT_OUTPUT_FILE_PATH)
            .expect("active result output should load");
        let (source_revision, concurrent_revision) =
            commit_concurrent_direction_change(&fixture, "concurrent full planning change");

        let error = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name,
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
                files: vec![PlanningAdminDraftFileUpdate {
                    key: PlanningAdminFileKey::ResultOutput,
                    body: "# Result Output\n\nMust not replace newer planning state.\n".to_string(),
                }],
            })
            .expect_err("stale full planning draft should not promote");

        assert_eq!(
            error.to_string(),
            format!(
                "planning authority changed from revision {source_revision} to {concurrent_revision}; reload and retry"
            )
        );
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(&fixture.workspace.path, RESULT_OUTPUT_FILE_PATH)
                .expect("active result output should remain readable"),
            original_result_output
        );
    }

    #[test]
    fn full_planning_draft_rejects_revision_drift_and_preserves_hidden_support_file() {
        let fixture = TestAdminFixture::new_sqlite("admin-full-draft-hidden-drift");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::FullPlanning, None)
            .expect("full planning draft should open");
        let source_revision = fixture
            .task_repository_port
            .load_task_authority_snapshot(&fixture.workspace.path)
            .expect("task authority should load")
            .expect("task authority should exist")
            .planning_revision;
        let original_result_output = fixture
            .workspace_port
            .load_optional_planning_file(&fixture.workspace.path, RESULT_OUTPUT_FILE_PATH)
            .expect("active result output should load");
        let concurrent_prompt = "# Queue Review\n\nPreserve this concurrent prompt.\n";
        fixture
            .workspace_port
            .replace_planning_workspace_file(
                &fixture.workspace.path,
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH,
                Some(concurrent_prompt),
            )
            .expect("concurrent prompt edit should succeed");
        let concurrent_revision = fixture
            .task_repository_port
            .load_task_authority_snapshot(&fixture.workspace.path)
            .expect("task authority should load")
            .expect("task authority should exist")
            .planning_revision;

        let error = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name,
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
                files: session
                    .files
                    .into_iter()
                    .map(|file| PlanningAdminDraftFileUpdate {
                        key: file.key,
                        body: file.body,
                    })
                    .collect(),
            })
            .expect_err("hidden supporting-file drift should block promotion");

        assert_eq!(
            error.to_string(),
            format!(
                "planning authority changed from revision {source_revision} to {concurrent_revision}; reload and retry"
            )
        );
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(
                    &fixture.workspace.path,
                    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH,
                )
                .expect("active prompt should remain readable")
                .as_deref(),
            Some(concurrent_prompt)
        );
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(&fixture.workspace.path, RESULT_OUTPUT_FILE_PATH)
                .expect("active result output should remain readable"),
            original_result_output
        );
    }

    #[test]
    fn specialized_draft_creation_requires_direction_id_for_detail_editor() {
        let fixture = TestAdminFixture::new("admin-detail-draft-missing-id");

        let error = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::DirectionDetail, None)
            .expect_err("detail draft without direction id should fail");

        assert_eq!(
            error.to_string(),
            "direction detail drafts require direction_id"
        );
    }

    #[test]
    fn draft_session_rejects_malformed_draft_names_before_workspace_access() {
        let fixture = TestAdminFixture::new("admin-invalid-draft-name");

        let load_error = fixture
            .facade
            .load_draft_session(PlanningAdminDraftLoadRequest {
                draft_name: "../outside".to_string(),
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
            })
            .expect_err("malformed load draft name should fail");
        assert!(
            load_error
                .to_string()
                .contains("invalid planning draft name `../outside`")
        );

        let save_error = fixture
            .facade
            .save_draft(PlanningAdminDraftMutationRequest {
                draft_name: "bad:name".to_string(),
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
                files: Vec::new(),
            })
            .expect_err("malformed save draft name should fail");
        assert!(
            save_error
                .to_string()
                .contains("invalid planning draft name `bad:name`")
        );
    }

    #[test]
    fn queue_idle_draft_save_updates_visible_file_and_keeps_hidden_context() {
        let fixture = TestAdminFixture::new_sqlite("admin-queue-idle-save");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::QueueIdlePrompt, None)
            .expect("queue-idle draft should open");

        let (save_result, saved_session) = fixture
            .facade
            .save_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::QueueIdlePrompt,
                direction_id: None,
                files: vec![
                    PlanningAdminDraftFileUpdate {
                        key: PlanningAdminFileKey::ResultOutput,
                        body: "ignored hidden result body".to_string(),
                    },
                    PlanningAdminDraftFileUpdate {
                        key: PlanningAdminFileKey::QueueIdlePrompt,
                        body: "# Queue Review\n\nUse the latest answer only.".to_string(),
                    },
                ],
            })
            .expect("queue-idle draft save should succeed");

        assert_eq!(save_result.draft_name, session.draft_name);
        assert!(save_result.validation_report.is_valid());
        assert_eq!(saved_session.files.len(), 1);
        assert_eq!(
            saved_session.files[0].key,
            PlanningAdminFileKey::QueueIdlePrompt
        );
        assert_eq!(
            saved_session.files[0].body,
            "# Queue Review\n\nUse the latest answer only."
        );

        let loaded = fixture
            .workspace_port
            .load_planning_draft_files(&fixture.workspace.path, &saved_session.draft_name)
            .expect("saved draft should remain loadable");
        let result_output = loaded
            .staged_files
            .iter()
            .find(|file| file.active_path == RESULT_OUTPUT_FILE_PATH)
            .expect("hidden result output should remain staged");
        assert_ne!(result_output.body, "ignored hidden result body");
    }

    #[test]
    fn direction_detail_draft_only_saves_and_promotes_the_selected_staged_document() {
        let fixture = TestAdminFixture::new_sqlite("admin-direction-detail-draft");
        fixture
            .facade
            .ensure_default_authority()
            .expect("authority should be seeded before adding another direction");
        let mut direction_snapshot = fixture
            .task_repository_port
            .load_direction_authority_snapshot(&fixture.workspace.path)
            .expect("direction authority should load")
            .expect("direction authority should exist");
        let observed_planning_revision = direction_snapshot.planning_revision;
        let secondary_path = format!("{PLANNING_DIRECTION_DOCS_DIRECTORY}/secondary.md");
        direction_snapshot
            .directions
            .directions
            .push(DirectionDefinition {
                id: "secondary-workstream".to_string(),
                title: "Secondary".to_string(),
                summary: "Secondary work".to_string(),
                success_criteria: vec!["Secondary work is done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: secondary_path.clone(),
                state: DirectionState::Active,
            });
        fixture
            .task_repository_port
            .commit_direction_authority_snapshot(
                &fixture.workspace.path,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: Some(observed_planning_revision),
                    directions: &direction_snapshot.directions,
                    authority_mutation_owner_token: None,
                },
            )
            .expect("secondary direction should commit");
        let secondary_body = "# Secondary Direction\n\nKeep this staged context unchanged.\n";
        fixture
            .workspace_port
            .replace_planning_workspace_file(
                &fixture.workspace.path,
                &secondary_path,
                Some(secondary_body),
            )
            .expect("secondary direction document should be stored");

        let session = fixture
            .facade
            .create_draft_session(
                PlanningAdminDraftKind::DirectionDetail,
                Some("general-workstream"),
            )
            .expect("direction detail draft should open for seeded direction");

        assert_eq!(session.kind, PlanningAdminDraftKind::DirectionDetail);
        assert_eq!(session.direction_id.as_deref(), Some("general-workstream"));
        assert_eq!(session.files.len(), 1);
        assert_eq!(session.files[0].key, PlanningAdminFileKey::DirectionDetail);
        let selected_path = session.files[0].active_path.clone();
        assert_ne!(selected_path, secondary_path);
        assert!(session.validation.is_valid);

        let staged = fixture
            .workspace_port
            .load_planning_draft_files(&fixture.workspace.path, &session.draft_name)
            .expect("direction draft should load");
        assert!(
            staged
                .staged_files
                .iter()
                .any(|file| file.active_path == secondary_path && file.body == secondary_body)
        );
        assert!(
            staged
                .staged_files
                .iter()
                .filter(|file| file
                    .active_path
                    .starts_with(PLANNING_DIRECTION_DOCS_DIRECTORY))
                .count()
                >= 2
        );

        let restarted_facade = PlanningAdminFacadeService::from_planning_with_authority(
            fixture.workspace.path.clone(),
            fixture.facade.planning.clone(),
            fixture.workspace_port.clone(),
            fixture.facade.planning_authority_port.clone(),
            fixture.task_repository_port.clone(),
        );
        let reloaded = restarted_facade
            .load_draft_session(PlanningAdminDraftLoadRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::DirectionDetail,
                direction_id: Some("general-workstream".to_string()),
            })
            .expect("metadata-free reload should use the selected authoritative detail path");
        assert_eq!(reloaded.files.len(), 1);
        assert_eq!(reloaded.files[0].active_path, selected_path);

        let selected_body = "# General Direction\n\nOnly this document should change.\n";
        let (_, saved) = restarted_facade
            .save_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::DirectionDetail,
                direction_id: Some("general-workstream".to_string()),
                files: vec![PlanningAdminDraftFileUpdate {
                    key: PlanningAdminFileKey::DirectionDetail,
                    body: selected_body.to_string(),
                }],
            })
            .expect("metadata-free save should update only the selected detail document");
        let saved_staged = fixture
            .workspace_port
            .load_planning_draft_files(&fixture.workspace.path, &session.draft_name)
            .expect("saved direction draft should load");
        assert!(
            saved_staged
                .staged_files
                .iter()
                .any(|file| file.active_path == secondary_path && file.body == secondary_body)
        );

        let (promote_result, _) = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name,
                kind: PlanningAdminDraftKind::DirectionDetail,
                direction_id: Some("general-workstream".to_string()),
                files: saved
                    .files
                    .into_iter()
                    .map(|file| PlanningAdminDraftFileUpdate {
                        key: file.key,
                        body: file.body,
                    })
                    .collect(),
            })
            .expect("selected direction detail should promote");
        assert_eq!(promote_result.promoted_file_count, 1);
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(&fixture.workspace.path, &selected_path)
                .expect("selected direction detail should load")
                .as_deref(),
            Some(selected_body)
        );
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(&fixture.workspace.path, &secondary_path)
                .expect("secondary direction detail should load")
                .as_deref(),
            Some(secondary_body)
        );
    }

    #[test]
    fn queue_idle_prompt_promote_rejects_stale_server_side_source_revision() {
        let fixture = TestAdminFixture::new_sqlite("admin-queue-idle-stale-revision");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::QueueIdlePrompt, None)
            .expect("queue-idle draft should open");
        let original_prompt = fixture
            .workspace_port
            .load_optional_planning_file(
                &fixture.workspace.path,
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH,
            )
            .expect("active queue-idle prompt should load");
        let (source_revision, concurrent_revision) =
            commit_concurrent_direction_change(&fixture, "concurrent queue-idle change");

        let error = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name,
                kind: PlanningAdminDraftKind::QueueIdlePrompt,
                direction_id: None,
                files: vec![PlanningAdminDraftFileUpdate {
                    key: PlanningAdminFileKey::QueueIdlePrompt,
                    body: "# Queue Review\n\nMust not replace newer planning state.\n".to_string(),
                }],
            })
            .expect_err("stale queue-idle draft should not promote");

        assert_eq!(
            error.to_string(),
            format!(
                "planning authority changed from revision {source_revision} to {concurrent_revision}; reload and retry"
            )
        );
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(
                    &fixture.workspace.path,
                    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH,
                )
                .expect("active queue-idle prompt should remain readable"),
            original_prompt
        );
        assert_eq!(
            fixture
                .task_repository_port
                .load_direction_authority_snapshot(&fixture.workspace.path)
                .expect("direction authority should load")
                .expect("direction authority should remain present")
                .directions
                .directions[0]
                .summary,
            "concurrent queue-idle change"
        );
    }

    #[test]
    fn direction_detail_promote_rejects_stale_server_side_source_revision() {
        let fixture = TestAdminFixture::new_sqlite("admin-direction-detail-stale-revision");
        let session = fixture
            .facade
            .create_draft_session(
                PlanningAdminDraftKind::DirectionDetail,
                Some("general-workstream"),
            )
            .expect("direction detail draft should open");
        let detail_path = session.files[0].active_path.clone();
        let original_detail = fixture
            .workspace_port
            .load_optional_planning_file(&fixture.workspace.path, &detail_path)
            .expect("active direction detail should load");
        let (source_revision, concurrent_revision) =
            commit_concurrent_direction_change(&fixture, "concurrent direction-detail change");

        let error = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name,
                kind: PlanningAdminDraftKind::DirectionDetail,
                direction_id: Some("general-workstream".to_string()),
                files: vec![PlanningAdminDraftFileUpdate {
                    key: PlanningAdminFileKey::DirectionDetail,
                    body: "# Direction Detail\n\nMust not replace newer planning state.\n"
                        .to_string(),
                }],
            })
            .expect_err("stale direction detail draft should not promote");

        assert_eq!(
            error.to_string(),
            format!(
                "planning authority changed from revision {source_revision} to {concurrent_revision}; reload and retry"
            )
        );
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(&fixture.workspace.path, &detail_path)
                .expect("active direction detail should remain readable"),
            original_detail
        );
        assert_eq!(
            fixture
                .task_repository_port
                .load_direction_authority_snapshot(&fixture.workspace.path)
                .expect("direction authority should load")
                .expect("direction authority should remain present")
                .directions
                .directions[0]
                .summary,
            "concurrent direction-detail change"
        );
    }

    #[test]
    fn specialized_promote_advances_server_revision_for_the_next_edit() {
        let fixture = TestAdminFixture::new_sqlite("admin-specialized-repeated-promote");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::QueueIdlePrompt, None)
            .expect("queue-idle draft should open");
        let draft_name = session.draft_name.clone();
        let source_revision = fixture
            .task_repository_port
            .load_task_authority_snapshot(&fixture.workspace.path)
            .expect("task authority should load")
            .expect("task authority should exist")
            .planning_revision;
        let mut first_files = session
            .files
            .into_iter()
            .map(|file| PlanningAdminDraftFileUpdate {
                key: file.key,
                body: file.body,
            })
            .collect::<Vec<_>>();
        first_files[0].body =
            "# Queue Review\n\nAdvance the atomic authority revision.\n".to_string();

        let (first_result, first_session) = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: draft_name.clone(),
                kind: PlanningAdminDraftKind::QueueIdlePrompt,
                direction_id: None,
                files: first_files,
            })
            .expect("first specialized promotion should succeed");
        let first_revision = first_result
            .committed_planning_revision
            .expect("successful promotion should report its committed revision");
        assert!(first_revision > source_revision);

        let (second_result, _) = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name,
                kind: PlanningAdminDraftKind::QueueIdlePrompt,
                direction_id: None,
                files: first_session
                    .files
                    .into_iter()
                    .map(|file| PlanningAdminDraftFileUpdate {
                        key: file.key,
                        body: file.body,
                    })
                    .collect(),
            })
            .expect("next specialized promotion should use the advanced server revision");

        assert!(
            second_result
                .committed_planning_revision
                .is_some_and(|revision| revision >= first_revision)
        );
    }

    #[test]
    fn specialized_promote_rejects_client_kind_that_bypasses_server_metadata() {
        let fixture = TestAdminFixture::new_sqlite("admin-specialized-metadata-required");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::QueueIdlePrompt, None)
            .expect("queue-idle draft should open");
        let original_result_output = fixture
            .workspace_port
            .load_optional_planning_file(&fixture.workspace.path, RESULT_OUTPUT_FILE_PATH)
            .expect("active result output should load");

        let error = fixture
            .facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name,
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
                files: vec![PlanningAdminDraftFileUpdate {
                    key: PlanningAdminFileKey::ResultOutput,
                    body: "# Result Output\n\nMust not bypass specialized draft guard.\n"
                        .to_string(),
                }],
            })
            .expect_err("client-provided draft kind must not bypass server metadata");

        assert_eq!(
            error.to_string(),
            "admin draft identity does not match its server-side stage metadata; create a new draft"
        );
        assert_eq!(
            fixture
                .workspace_port
                .load_optional_planning_file(&fixture.workspace.path, RESULT_OUTPUT_FILE_PATH)
                .expect("active result output should remain readable"),
            original_result_output
        );
    }

    #[test]
    fn full_planning_draft_remains_readable_but_fails_closed_after_metadata_is_lost() {
        let fixture = TestAdminFixture::new_sqlite("admin-full-planning-metadata-lost");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::FullPlanning, None)
            .expect("full planning draft should open");
        let restarted_facade = PlanningAdminFacadeService::from_planning_with_authority(
            fixture.workspace.path.clone(),
            fixture.facade.planning.clone(),
            fixture.workspace_port.clone(),
            fixture.facade.planning_authority_port.clone(),
            fixture.task_repository_port.clone(),
        );

        let loaded = restarted_facade
            .load_draft_session(PlanningAdminDraftLoadRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
            })
            .expect("full planning draft should remain readable after restart");
        let (_, saved) = restarted_facade
            .save_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
                files: loaded
                    .files
                    .into_iter()
                    .map(|file| PlanningAdminDraftFileUpdate {
                        key: file.key,
                        body: file.body,
                    })
                    .collect(),
            })
            .expect("full planning draft should remain saveable after restart");
        let error = restarted_facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::FullPlanning,
                direction_id: None,
                files: saved
                    .files
                    .into_iter()
                    .map(|file| PlanningAdminDraftFileUpdate {
                        key: file.key,
                        body: file.body,
                    })
                    .collect(),
            })
            .expect_err("full planning draft without server metadata must fail closed");

        assert_eq!(
            error.to_string(),
            format!(
                "admin draft metadata is unavailable for `{}`; create a new draft",
                session.draft_name
            )
        );
    }

    #[test]
    fn specialized_promote_fails_closed_after_server_metadata_is_lost() {
        let fixture = TestAdminFixture::new_sqlite("admin-specialized-metadata-lost");
        let session = fixture
            .facade
            .create_draft_session(PlanningAdminDraftKind::QueueIdlePrompt, None)
            .expect("queue-idle draft should open");
        let restarted_facade = PlanningAdminFacadeService::from_planning_with_authority(
            fixture.workspace.path.clone(),
            fixture.facade.planning.clone(),
            fixture.workspace_port.clone(),
            fixture.facade.planning_authority_port.clone(),
            fixture.task_repository_port.clone(),
        );
        let loaded = restarted_facade
            .load_draft_session(PlanningAdminDraftLoadRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::QueueIdlePrompt,
                direction_id: None,
            })
            .expect("specialized draft should remain readable after metadata loss");
        let (_, saved) = restarted_facade
            .save_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::QueueIdlePrompt,
                direction_id: None,
                files: loaded
                    .files
                    .into_iter()
                    .map(|file| PlanningAdminDraftFileUpdate {
                        key: file.key,
                        body: file.body,
                    })
                    .collect(),
            })
            .expect("specialized draft should remain saveable after metadata loss");

        let error = restarted_facade
            .promote_draft(PlanningAdminDraftMutationRequest {
                draft_name: session.draft_name.clone(),
                kind: PlanningAdminDraftKind::QueueIdlePrompt,
                direction_id: None,
                files: saved
                    .files
                    .into_iter()
                    .map(|file| PlanningAdminDraftFileUpdate {
                        key: file.key,
                        body: file.body,
                    })
                    .collect(),
            })
            .expect_err("specialized draft without server metadata must fail closed");

        assert_eq!(
            error.to_string(),
            format!(
                "admin draft metadata is unavailable for `{}`; create a new draft",
                session.draft_name
            )
        );
    }

    #[test]
    fn invalid_session_view_suppresses_queue_preview_and_keeps_editor_body_visible() {
        let fixture = TestAdminFixture::new("admin-invalid-session-view");
        fixture
            .facade
            .ensure_default_authority()
            .expect("authority seed should be available before draft validation");
        let loaded = PlanningDraftLoadRecord {
            draft_name: "invalid-result-output-draft".to_string(),
            draft_directory: "drafts/invalid-result-output-draft".to_string(),
            staged_files: vec![PlanningDraftLoadFileRecord {
                active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                staged_path: "drafts/invalid-result-output-draft/result-output.md".to_string(),
                body: "not a heading".to_string(),
            }],
        };

        let session = fixture
            .facade
            .build_session_view(PlanningAdminDraftKind::FullPlanning, None, loaded)
            .expect("invalid draft should still build a session view");

        assert_eq!(session.files.len(), 1);
        assert_eq!(session.files[0].body, "not a heading");
        assert!(!session.validation.is_valid);
        assert!(session.validation.error_count > 0);
        assert!(session.queue_preview.is_none());
    }

    #[test]
    fn session_view_requires_effective_result_output_for_validation() {
        let fixture = TestAdminFixture::new("admin-missing-result-output");
        fixture
            .facade
            .ensure_default_authority()
            .expect("authority seed should be available before draft validation");
        fixture
            .workspace_port
            .replace_planning_workspace_file(&fixture.workspace.path, RESULT_OUTPUT_FILE_PATH, None)
            .expect("active result output should be removable for missing-core test");
        let loaded = PlanningDraftLoadRecord {
            draft_name: "draft-without-result-output".to_string(),
            draft_directory: "drafts/draft-without-result-output".to_string(),
            staged_files: Vec::new(),
        };

        let error = fixture
            .facade
            .build_session_view(PlanningAdminDraftKind::FullPlanning, None, loaded)
            .expect_err("session without result output should fail");

        assert_eq!(
            error.to_string(),
            format!(
                "draft is missing required result output content at {}",
                RESULT_OUTPUT_FILE_PATH
            )
        );
    }

    fn commit_concurrent_direction_change(fixture: &TestAdminFixture, summary: &str) -> (i64, i64) {
        let mut snapshot = fixture
            .task_repository_port
            .load_direction_authority_snapshot(&fixture.workspace.path)
            .expect("direction authority should load")
            .expect("direction authority should be present");
        let source_revision = snapshot.planning_revision;
        snapshot.directions.directions[0].summary = summary.to_string();
        let concurrent_revision = match fixture
            .task_repository_port
            .commit_direction_authority_snapshot(
                &fixture.workspace.path,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: Some(source_revision),
                    directions: &snapshot.directions,
                    authority_mutation_owner_token: None,
                },
            )
            .expect("concurrent direction mutation should commit")
        {
            PlanningTaskAuthorityCommitResult::Committed {
                planning_revision, ..
            } => planning_revision,
            conflict => panic!("unexpected concurrent direction conflict: {conflict:?}"),
        };
        (source_revision, concurrent_revision)
    }

    fn direction_catalog_with_supporting_paths() -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::ReviewAndEnqueue,
                prompt_path: DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            },
            directions: vec![
                DirectionDefinition {
                    id: "general-workstream".to_string(),
                    title: "General".to_string(),
                    summary: "General work".to_string(),
                    success_criteria: vec!["Done".to_string()],
                    scope_hints: Vec::new(),
                    detail_doc_path: "directions/general.md".to_string(),
                    state: DirectionState::Active,
                },
                DirectionDefinition {
                    id: "release".to_string(),
                    title: "Release".to_string(),
                    summary: "Release work".to_string(),
                    success_criteria: vec!["Shipped".to_string()],
                    scope_hints: Vec::new(),
                    detail_doc_path: "directions/release.md".to_string(),
                    state: DirectionState::Active,
                },
                DirectionDefinition {
                    id: "duplicate".to_string(),
                    title: "Duplicate".to_string(),
                    summary: "Duplicate detail path".to_string(),
                    success_criteria: vec!["Done".to_string()],
                    scope_hints: Vec::new(),
                    detail_doc_path: "directions/general.md".to_string(),
                    state: DirectionState::Paused,
                },
            ],
        }
    }

    struct TestAdminFixture {
        workspace: TempPlanningWorkspace,
        facade: PlanningAdminFacadeService,
        workspace_port: Arc<dyn PlanningWorkspacePort>,
        task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    }

    impl TestAdminFixture {
        fn new(prefix: &str) -> Self {
            let workspace = TempPlanningWorkspace::new(prefix);
            let workspace_port: Arc<dyn PlanningWorkspacePort> =
                Arc::new(FilesystemPlanningWorkspaceAdapter::new());
            let authority_port = Arc::new(NoopPlanningAuthorityPort::default());
            let task_repository_port = Arc::new(NoopPlanningTaskRepositoryPort);
            let planning = PlanningServices::from_ports(
                workspace_port.clone(),
                authority_port.clone(),
                task_repository_port.clone(),
                Arc::new(NoopPlanningWorkerPort),
            );
            let facade = PlanningAdminFacadeService::from_planning_with_authority(
                workspace.path.clone(),
                planning,
                workspace_port.clone(),
                authority_port,
                task_repository_port.clone(),
            );
            Self {
                workspace,
                facade,
                workspace_port,
                task_repository_port,
            }
        }

        fn new_sqlite(prefix: &str) -> Self {
            let workspace = TempPlanningWorkspace::new_git(prefix);
            let sqlite = Arc::new(SqlitePlanningAuthorityAdapter::new());
            let workspace_port: Arc<dyn PlanningWorkspacePort> = Arc::new(
                FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(sqlite.clone()),
            );
            let authority_port: Arc<dyn PlanningAuthorityPort> = sqlite.clone();
            let task_repository_port: Arc<dyn PlanningTaskRepositoryPort> = sqlite;
            let planning = PlanningServices::from_ports(
                workspace_port.clone(),
                authority_port.clone(),
                task_repository_port.clone(),
                Arc::new(NoopPlanningWorkerPort),
            );
            let facade = PlanningAdminFacadeService::from_planning_with_authority(
                workspace.path.clone(),
                planning,
                workspace_port.clone(),
                authority_port,
                task_repository_port.clone(),
            );
            Self {
                workspace,
                facade,
                workspace_port,
                task_repository_port,
            }
        }
    }

    struct TempPlanningWorkspace {
        path: String,
    }

    impl TempPlanningWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&path).expect("temp planning workspace should be created");
            Self {
                path: path.display().to_string(),
            }
        }

        fn new_git(prefix: &str) -> Self {
            let workspace = Self::new(prefix);
            let status = Command::new("git")
                .args(["init", "--quiet", workspace.path.as_str()])
                .status()
                .expect("git init should run");
            assert!(status.success(), "git init should succeed");
            workspace
        }
    }

    impl Drop for TempPlanningWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
