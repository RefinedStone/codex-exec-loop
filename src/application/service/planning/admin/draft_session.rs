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
 * Draft sessions are the admin editor's isolation layer. The active planning
 * files remain untouched while the operator edits staged copies; every save,
 * validation preview, and promotion must preserve that distinction and expose
 * only the file kind that belongs to the selected editor surface.
 */
impl PlanningAdminFacadeService {
    pub fn create_draft_session(
        &self,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
    ) -> Result<PlanningAdminSessionView> {
        // Each draft kind is staged by the workspace service because it owns the
        // active/staged path mapping. This facade only chooses the right staging
        // workflow and then renders the common session view.
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
        // Loading a draft also seeds default authority so validation never runs
        // against a half-initialized planning workspace.
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
        // Save persists only the editor-visible files. Hidden staged files stay
        // untouched so a specialized editor cannot accidentally overwrite
        // unrelated planning artifacts in the same draft directory.
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
        // Promotion uses the same visible-file resolution as save, then reloads
        // the session so validation reflects the active files after promotion.
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
        // Merge posted editor bodies with the staged records. The active/staged
        // path pair from storage is preserved; the request is allowed to replace
        // only bodies keyed by the current editor kind.
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
        // Session rendering filters staged files down to the selected surface
        // and pairs them with validation/queue preview computed from the same
        // staged content.
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
        // Validation composes staged edits with current authority: result_output
        // may be staged, while directions/tasks still come from DB authority for
        // specialized prompt/detail drafts.
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
            // Supporting file checks must see staged files first, then active
            // workspace files. That lets a draft add or fix a detail document
            // before promotion.
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
            // Queue preview is intentionally best-effort: invalid drafts show
            // validation issues instead of masking them with queue errors.
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
        // Core files prefer staged content but fall back to active workspace
        // content so narrow draft kinds can still validate the full workspace.
        if let Some(body) = staged_files.get(path) {
            return Ok((*body).to_string());
        }
        self.planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), path)?
            .ok_or_else(|| missing_core_draft_file_error(path, file_kind))
    }
    pub(super) fn stage_active_manual_editor_draft(&self) -> Result<String> {
        // Full planning draft stages result_output plus all supporting prompt
        // and direction detail files currently referenced by direction authority.
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
    validation: PlanningAdminValidationView,
    queue_preview: Option<PlanningAdminQueuePreview>,
}

fn collect_direction_supporting_paths(directions: &DirectionCatalogDocument) -> Vec<String> {
    // Collect unique supporting files from authority in sorted order so draft
    // creation is deterministic and does not duplicate shared prompt paths.
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
    // Active paths classify staged records into editor panes. The second match
    // enforces that a narrow editor sees only its own file class.
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
