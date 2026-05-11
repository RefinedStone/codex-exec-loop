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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
        PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftLoadFileRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning::PlanningServices;
    use crate::application::service::planning::admin::PlanningAdminDraftFileUpdate;
    use crate::domain::planning::{
        DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION, QueueIdleConfig,
        QueueIdlePolicy,
    };
    use std::fs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn file_key_mapping_filters_files_by_editor_kind() {
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::FullPlanning,
                RESULT_OUTPUT_FILE_PATH
            ),
            Some(PlanningAdminFileKey::ResultOutput)
        );
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::QueueIdlePrompt,
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
            ),
            Some(PlanningAdminFileKey::QueueIdlePrompt)
        );
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::QueueIdlePrompt,
                ".codex-exec-loop/planning/prompts/custom-review.md"
            ),
            Some(PlanningAdminFileKey::QueueIdlePrompt)
        );
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::DirectionDetail,
                ".codex-exec-loop/planning/directions/general.md"
            ),
            Some(PlanningAdminFileKey::DirectionDetail)
        );
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::QueueIdlePrompt,
                ".codex-exec-loop/planning/directions/general.md"
            ),
            None
        );
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::FullPlanning,
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
            ),
            None
        );
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::QueueIdlePrompt,
                RESULT_OUTPUT_FILE_PATH
            ),
            None
        );
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::DirectionDetail,
                RESULT_OUTPUT_FILE_PATH
            ),
            None
        );
        assert_eq!(
            file_key_for_kind(
                PlanningAdminDraftKind::DirectionDetail,
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
            ),
            None
        );
        assert_eq!(
            file_key_for_kind(PlanningAdminDraftKind::FullPlanning, "unknown/path.md"),
            None
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
        let fixture = TestAdminFixture::new("admin-full-draft");

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
    fn full_planning_draft_skips_missing_supporting_paths_without_empty_staged_files() {
        let fixture = TestAdminFixture::new("admin-full-draft-missing-support");
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
                },
            )
            .expect("custom direction authority should commit");

        let draft_name = fixture
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
        let fixture = TestAdminFixture::new("admin-full-draft-promote");
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
        assert_eq!(promote_result.promoted_file_count, 2);
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
    fn queue_idle_draft_save_updates_visible_file_and_keeps_hidden_context() {
        let fixture = TestAdminFixture::new("admin-queue-idle-save");
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
    fn direction_detail_draft_surfaces_only_selected_detail_file() {
        let fixture = TestAdminFixture::new("admin-direction-detail-draft");

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
        assert!(
            session.files[0]
                .active_path
                .starts_with(PLANNING_DIRECTION_DOCS_DIRECTORY)
        );
        assert!(session.validation.is_valid);
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
    }

    impl Drop for TempPlanningWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
