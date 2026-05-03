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
 * Direction maintenance sits between DB-backed direction authority and
 * workspace-backed markdown files. The catalog names each supporting file, but
 * the file bodies live in the planning workspace, so this service keeps
 * mapping, staging, validation, and operator summaries aligned.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionsSupportingFileStatus {
    MissingMapping,
    Ready,
    BrokenMapping,
}
impl DirectionsSupportingFileStatus {
    pub fn label(self) -> &'static str {
        // These labels are presentation-facing status atoms; the admin and TUI
        // surfaces use them to summarize whether repair is needed.
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
    // Summary rows intentionally avoid full direction bodies. The maintenance
    // surface only needs identity plus supporting-file health.
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_status: DirectionsSupportingFileStatus,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceSummary {
    // This projection is the admin checklist: per-direction detail docs,
    // aggregate repair counts, and the queue-idle prompt mapping.
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
    // Runtime queue-idle review reads normalized prompt markdown from here. The
    // policy still comes from authority, so disabling review does not require
    // deleting the supporting prompt file.
    pub policy: QueueIdlePolicy,
    pub prompt_path: Option<String>,
    pub prompt_markdown: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDoctorOutcome {
    // Doctor output separates authority mapping repair from workspace file
    // creation so operator feedback can explain which layer changed.
    pub repaired_detail_doc_mappings: usize,
    pub created_detail_doc_files: usize,
    pub repaired_queue_idle_prompt_mapping: bool,
    pub created_queue_idle_prompt_file: bool,
    pub validation_report: PlanningValidationReport,
}
impl PlanningDoctorOutcome {
    pub fn applied_fix_count(&self) -> usize {
        // The coarse count is for status copy; validation_report carries the
        // deeper post-repair contract state.
        self.repaired_detail_doc_mappings
            + self.created_detail_doc_files
            + usize::from(self.repaired_queue_idle_prompt_mapping)
            + usize::from(self.created_queue_idle_prompt_file)
    }
}
#[derive(Clone)]
pub struct PlanningDirectionsService {
    // Workspace port owns markdown bodies, repository port owns direction
    // authority, and validation binds the two views into one coherent contract.
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
            // The seed service prevents each direction-maintenance entrypoint
            // from duplicating "make planning usable before reading it" logic.
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
        // Direction maintenance can be the first planning feature a workspace
        // touches, so reads always pass through default-authority seeding.
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
        // Direction edits commit only the catalog. Supporting markdown bodies
        // stay in workspace drafts and are promoted by the shared draft flow.
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
        // Summary loading performs health checks without opening every body:
        // configured paths must live under the expected planning directories
        // and resolve through the workspace port.
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
                // Trimmed ids/titles keep read-only projections clean without
                // mutating direction authority during summary rendering.
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
        // Missing means authority has no path; broken means a configured path
        // is invalid or the referenced workspace file cannot be loaded.
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
        // Runtime review tolerates an absent prompt body by returning None, but
        // still exposes authority policy/path for orchestration decisions.
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
        // Opening a detail-doc editor may first repair the catalog path. The
        // chosen path is committed to authority, while the markdown body is
        // staged as a workspace file for validation and later promotion.
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
        // Replace any loaded copy so the draft contains the resolver-selected
        // detail file exactly once.
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
        // Queue-idle prompt editing follows the same split: authority stores
        // the prompt path, workspace draft files store the editable markdown.
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
        // Normalized/default prompt content is staged so the editor opens with
        // a meaningful review contract even when the active file is missing.
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
        // Specialized maintenance drafts carry result-output plus every loaded
        // supporting file, but only editable_paths are exposed to the UI. The
        // hidden files are present so validation sees a full planning picture.
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
        // Build the source snapshot for editor drafts by joining authoritative
        // directions, result-output markdown, and referenced supporting files.
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
        // Missing supporting files are omitted here. The resolver for the
        // selected editor target can create an empty/default staged body later.
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
        // Validate staged supporting files before active workspace files so an
        // in-progress draft can fix broken mappings before promotion.
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
        // This editor does not modify task authority. A minimal valid document
        // is enough to run direction/result-output validation in isolation.
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
        // Supporting file reads are advisory here; callers convert absence into
        // explicit status, fallback body, or validation diagnostics.
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
        // A mapped supporting file is healthy only when it stays inside the
        // expected planning directory and resolves through the workspace port.
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
        // Preserve a valid configured path even when the file is absent; an
        // empty staged body lets the operator create the missing document.
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
        // Invalid or absent mappings fall back to the deterministic detail-doc
        // path used by doctor/admin repair flows.
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
        // Preserve a valid configured prompt path and normalize loaded content
        // into the canonical queue-idle review prompt shape.
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
        // The fallback path/body aligns Simple-mode bootstrap, doctor repair,
        // and queue-idle runtime review on the same default prompt contract.
        let fallback_path = DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string();
        let fallback_body = self
            .load_supporting_file_best_effort(workspace_dir, &fallback_path)
            .unwrap_or_else(|| DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string());
        Ok((fallback_path, fallback_body))
    }
}

struct ActiveDirectionsWorkspace {
    // Internal aggregate used only while staging a maintenance draft. It joins
    // authority and workspace bodies long enough to create a consistent draft.
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
