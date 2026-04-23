use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH,
};
use crate::application::service::planning::{
    DirectionsMaintenanceSummary, PlanningDoctorReport, PlanningDraftEditorFile,
    PlanningDraftPromoteResult, PlanningDraftSaveResult, PlanningResetTarget,
    PlanningRuntimeSnapshot, PlanningServices,
};
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningFileKind, PlanningValidationReport, PlanningWorkspaceFiles,
    PriorityQueueSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningAdminDraftKind {
    FullPlanning,
    Directions,
    TaskLedger,
    QueueIdlePrompt,
    DirectionDetail,
}

impl PlanningAdminDraftKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::FullPlanning => "full planning",
            Self::Directions => "directions",
            Self::TaskLedger => "task ledger",
            Self::QueueIdlePrompt => "queue-idle prompt",
            Self::DirectionDetail => "direction detail",
        }
    }

    pub fn editor_heading(self) -> &'static str {
        match self {
            Self::FullPlanning => "Full Planning Draft",
            Self::Directions => "Directions Draft",
            Self::TaskLedger => "Task Draft",
            Self::QueueIdlePrompt => "Queue-Idle Prompt Draft",
            Self::DirectionDetail => "Direction Detail Draft",
        }
    }

    pub fn return_path(self) -> &'static str {
        match self {
            Self::FullPlanning => "/admin",
            Self::Directions | Self::QueueIdlePrompt | Self::DirectionDetail => "/admin/directions",
            Self::TaskLedger => "/admin/tasks",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::FullPlanning => "full_planning",
            Self::Directions => "directions",
            Self::TaskLedger => "task_ledger",
            Self::QueueIdlePrompt => "queue_idle_prompt",
            Self::DirectionDetail => "direction_detail",
        }
    }
}

impl fmt::Display for PlanningAdminDraftKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PlanningAdminFileKey {
    Directions,
    TaskLedger,
    ResultOutput,
    QueueIdlePrompt,
    DirectionDetail,
}

impl PlanningAdminFileKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::Directions => "Directions",
            Self::TaskLedger => "Task Ledger",
            Self::ResultOutput => "Result Output",
            Self::QueueIdlePrompt => "Queue-Idle Prompt",
            Self::DirectionDetail => "Direction Detail",
        }
    }

    pub fn editor_language(self) -> &'static str {
        match self {
            Self::Directions => "toml",
            Self::TaskLedger => "json",
            Self::ResultOutput | Self::QueueIdlePrompt | Self::DirectionDetail => "markdown",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Directions => "directions",
            Self::TaskLedger => "task_ledger",
            Self::ResultOutput => "result_output",
            Self::QueueIdlePrompt => "queue_idle_prompt",
            Self::DirectionDetail => "direction_detail",
        }
    }
}

impl fmt::Display for PlanningAdminFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftLoadRequest {
    pub draft_name: String,
    pub kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub direction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftFileUpdate {
    pub key: PlanningAdminFileKey,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDraftMutationRequest {
    pub draft_name: String,
    pub kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub direction_id: Option<String>,
    #[serde(default)]
    pub files: Vec<PlanningAdminDraftFileUpdate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminValidationIssueView {
    pub severity: String,
    pub file_kind: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminValidationView {
    pub is_valid: bool,
    pub error_count: usize,
    pub warning_count: usize,
    pub issues: Vec<PlanningAdminValidationIssueView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueueTaskView {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueueHeadView {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub rank_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminQueuePreview {
    pub queue_summary: String,
    pub proposal_summary: Option<String>,
    pub queue_head: Option<PlanningAdminQueueHeadView>,
    pub visible_tasks: Vec<PlanningAdminQueueTaskView>,
    pub proposed_tasks: Vec<PlanningAdminQueueTaskView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDraftFileView {
    pub key: PlanningAdminFileKey,
    pub label: String,
    pub active_path: String,
    pub editor_language: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminSessionView {
    pub kind: PlanningAdminDraftKind,
    pub direction_id: Option<String>,
    pub draft_name: String,
    pub draft_directory: String,
    pub editor_heading: String,
    pub return_path: String,
    pub files: Vec<PlanningAdminDraftFileView>,
    pub validation: PlanningAdminValidationView,
    pub queue_preview: Option<PlanningAdminQueuePreview>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDirectionSummaryView {
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_status: String,
    pub needs_attention: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDirectionsSummaryView {
    pub missing_detail_doc_count: usize,
    pub broken_detail_doc_count: usize,
    pub queue_idle_policy: String,
    pub queue_idle_prompt_path: Option<String>,
    pub queue_idle_prompt_status: String,
    pub parse_error: Option<String>,
    pub directions: Vec<PlanningAdminDirectionSummaryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminDoctorSummary {
    pub planning_state: String,
    pub queue_idle_policy: Option<String>,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub health: Option<String>,
    pub issue: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminRuntimeSummary {
    pub workspace_present: bool,
    pub plan_enabled: bool,
    pub preview_status_label: String,
    pub preview_detail: Option<String>,
    pub queue_head: Option<PlanningAdminQueueHeadView>,
    pub visible_tasks: Vec<PlanningAdminQueueTaskView>,
    pub proposed_tasks: Vec<PlanningAdminQueueTaskView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminOverview {
    pub workspace_dir: String,
    pub doctor: PlanningAdminDoctorSummary,
    pub runtime: PlanningAdminRuntimeSummary,
    pub directions: Option<PlanningAdminDirectionsSummaryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminTogglePlanOutcome {
    pub enabled: bool,
    pub doctor: PlanningAdminDoctorSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminResetOutcome {
    pub target: String,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub doctor: PlanningAdminDoctorSummary,
}

#[derive(Clone)]
pub struct PlanningAdminFacadeService {
    workspace_dir: String,
    planning: PlanningServices,
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}

impl PlanningAdminFacadeService {
    pub fn from_planning(
        workspace_dir: impl Into<String>,
        planning: PlanningServices,
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            planning,
            planning_workspace_port,
            planning_validation_service: PlanningValidationService::new(),
            priority_queue_service: PriorityQueueService::new(),
        }
    }

    pub fn new(
        workspace_dir: impl Into<String>,
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> Self {
        let planning = PlanningServices::from_workspace_port(planning_workspace_port.clone());
        Self::from_planning(workspace_dir, planning, planning_workspace_port)
    }

    pub fn workspace_dir(&self) -> &str {
        &self.workspace_dir
    }

    pub fn load_overview(&self) -> Result<PlanningAdminOverview> {
        let doctor = self
            .planning
            .workspace
            .inspect_workspace(self.workspace_dir.as_str());
        let runtime = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(self.workspace_dir.as_str());
        let directions = self
            .planning
            .workspace
            .load_summary(self.workspace_dir.as_str())
            .ok()
            .map(map_directions_summary);

        Ok(PlanningAdminOverview {
            workspace_dir: self.workspace_dir.clone(),
            doctor: map_doctor_report(&doctor),
            runtime: map_runtime_snapshot(&runtime),
            directions,
        })
    }

    pub fn load_runtime_summary(&self) -> Result<PlanningAdminRuntimeSummary> {
        let runtime = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(self.workspace_dir.as_str());
        Ok(map_runtime_snapshot(&runtime))
    }

    pub fn create_draft_session(
        &self,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
    ) -> Result<PlanningAdminSessionView> {
        let draft_name = match kind {
            PlanningAdminDraftKind::FullPlanning | PlanningAdminDraftKind::TaskLedger => {
                self.stage_active_manual_editor_draft()?
            }
            PlanningAdminDraftKind::Directions => {
                self.planning
                    .workspace
                    .stage_editor_session(self.workspace_dir.as_str())?
                    .draft_name
            }
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
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(self.workspace_dir.as_str(), &request.draft_name)?;
        self.build_session_view(request.kind, request.direction_id, loaded)
    }

    pub fn save_draft(
        &self,
        request: PlanningAdminDraftMutationRequest,
    ) -> Result<(PlanningDraftSaveResult, PlanningAdminSessionView)> {
        let visible_files = self.resolve_mutated_visible_files(
            &request.draft_name,
            request.kind,
            request.direction_id.as_deref(),
            &request.files,
        )?;
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
        let visible_files = self.resolve_mutated_visible_files(
            &request.draft_name,
            request.kind,
            request.direction_id.as_deref(),
            &request.files,
        )?;
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

    pub fn set_plan_enabled(&self, enabled: bool) -> Result<PlanningAdminTogglePlanOutcome> {
        self.planning
            .workspace
            .set_plan_enabled(self.workspace_dir.as_str(), enabled)?;
        let doctor = self
            .planning
            .workspace
            .inspect_workspace(self.workspace_dir.as_str());
        Ok(PlanningAdminTogglePlanOutcome {
            enabled,
            doctor: map_doctor_report(&doctor),
        })
    }

    pub fn reset_workspace(
        &self,
        target: PlanningResetTarget,
    ) -> Result<PlanningAdminResetOutcome> {
        let result = self
            .planning
            .workspace
            .reset_workspace(self.workspace_dir.as_str(), target)?;
        let doctor = self
            .planning
            .workspace
            .inspect_workspace(self.workspace_dir.as_str());
        Ok(PlanningAdminResetOutcome {
            target: result.target.label().to_string(),
            rewritten_paths: result.rewritten_paths,
            removed_paths: result.removed_paths,
            doctor: map_doctor_report(&doctor),
        })
    }

    fn resolve_mutated_visible_files(
        &self,
        draft_name: &str,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
        updates: &[PlanningAdminDraftFileUpdate],
    ) -> Result<Vec<PlanningDraftEditorFile>> {
        let session = self.load_draft_session(PlanningAdminDraftLoadRequest {
            draft_name: draft_name.to_string(),
            kind,
            direction_id: direction_id.map(str::to_string),
        })?;
        let update_map = updates
            .iter()
            .map(|update| (update.key, update.body.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut files = Vec::with_capacity(session.files.len());
        for file in session.files {
            files.push(PlanningDraftEditorFile {
                active_path: file.active_path,
                staged_path: format!("{}#{}", draft_name, file.key.label()),
                body: update_map.get(&file.key).cloned().unwrap_or(file.body),
            });
        }
        Ok(files)
    }

    fn build_session_view(
        &self,
        kind: PlanningAdminDraftKind,
        direction_id: Option<String>,
        loaded: PlanningDraftLoadRecord,
    ) -> Result<PlanningAdminSessionView> {
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
        let staged_files = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<BTreeMap<_, _>>();
        let directions_toml = self.load_effective_draft_body(
            &staged_files,
            DIRECTIONS_FILE_PATH,
            PlanningFileKind::Directions,
        )?;
        let task_ledger_json = self.load_effective_draft_body(
            &staged_files,
            TASK_LEDGER_FILE_PATH,
            PlanningFileKind::TaskLedger,
        )?;
        let task_ledger_schema_json = self.load_effective_draft_body(
            &staged_files,
            TASK_LEDGER_SCHEMA_FILE_PATH,
            PlanningFileKind::TaskLedgerSchema,
        )?;
        let result_output_markdown = self.load_effective_draft_body(
            &staged_files,
            RESULT_OUTPUT_FILE_PATH,
            PlanningFileKind::ResultOutput,
        )?;

        let mut result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: &directions_toml,
                    task_ledger_json: &task_ledger_json,
                    task_ledger_schema_json: &task_ledger_schema_json,
                    result_output_markdown: &result_output_markdown,
                });

        if let Some(directions) = result.directions.as_ref() {
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
            match (result.directions.as_ref(), result.task_ledger.as_ref()) {
                (Some(directions), Some(task_ledger)) => self
                    .priority_queue_service
                    .build_snapshot(directions, task_ledger)
                    .ok()
                    .map(map_queue_preview),
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

    fn load_effective_draft_body(
        &self,
        staged_files: &BTreeMap<&str, &str>,
        path: &'static str,
        file_kind: PlanningFileKind,
    ) -> Result<String> {
        if let Some(body) = staged_files.get(path) {
            return Ok((*body).to_string());
        }
        self.planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), path)?
            .ok_or_else(|| missing_core_draft_file_error(path, file_kind))
    }

    fn stage_active_manual_editor_draft(&self) -> Result<String> {
        let directions_toml = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), DIRECTIONS_FILE_PATH)?
            .ok_or_else(|| {
                anyhow!("planning directions are unavailable; initialize planning first")
            })?;
        let task_ledger_json = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), TASK_LEDGER_FILE_PATH)?
            .ok_or_else(|| anyhow!("task-ledger.json is unavailable; initialize planning first"))?;
        let result_output_markdown = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), RESULT_OUTPUT_FILE_PATH)?
            .ok_or_else(|| anyhow!("result-output.md is unavailable; initialize planning first"))?;
        let task_ledger_schema_json = self
            .planning_workspace_port
            .load_optional_planning_file(self.workspace_dir.as_str(), TASK_LEDGER_SCHEMA_FILE_PATH)?
            .ok_or_else(|| {
                anyhow!("task-ledger.schema.json is unavailable; initialize planning first")
            })?;
        let mut files = vec![
            PlanningDraftFileRecord {
                active_path: DIRECTIONS_FILE_PATH.to_string(),
                body: directions_toml.clone(),
            },
            PlanningDraftFileRecord {
                active_path: TASK_LEDGER_FILE_PATH.to_string(),
                body: task_ledger_json,
            },
            PlanningDraftFileRecord {
                active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                body: result_output_markdown,
            },
            PlanningDraftFileRecord {
                active_path: TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
                body: task_ledger_schema_json,
            },
        ];

        let supporting_paths = collect_direction_supporting_paths(&directions_toml);
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

fn collect_direction_supporting_paths(directions_toml: &str) -> Vec<String> {
    let Ok(directions) = toml::from_str::<DirectionCatalogDocument>(directions_toml) else {
        return Vec::new();
    };

    let mut paths = BTreeSet::new();
    let prompt_path = directions.queue_idle.prompt_path.trim();
    if !prompt_path.is_empty() {
        paths.insert(prompt_path.to_string());
    }

    for direction in directions.directions {
        let detail_doc_path = direction.detail_doc_path.trim();
        if !detail_doc_path.is_empty() {
            paths.insert(detail_doc_path.to_string());
        }
    }

    paths.into_iter().collect()
}

#[derive(Debug, Clone)]
struct PlanningAdminDraftValidationSnapshot {
    validation: PlanningAdminValidationView,
    queue_preview: Option<PlanningAdminQueuePreview>,
}

fn missing_core_draft_file_error(path: &'static str, file_kind: PlanningFileKind) -> anyhow::Error {
    anyhow!(
        "draft is missing required {} content at {}",
        match file_kind {
            PlanningFileKind::Directions => "directions",
            PlanningFileKind::TaskLedger => "task-ledger",
            PlanningFileKind::TaskLedgerSchema => "task-ledger schema",
            PlanningFileKind::ResultOutput => "result output",
        },
        path
    )
}

fn file_key_for_kind(
    kind: PlanningAdminDraftKind,
    active_path: &str,
) -> Option<PlanningAdminFileKey> {
    let key = if active_path == DIRECTIONS_FILE_PATH {
        PlanningAdminFileKey::Directions
    } else if active_path == TASK_LEDGER_FILE_PATH {
        PlanningAdminFileKey::TaskLedger
    } else if active_path == RESULT_OUTPUT_FILE_PATH {
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
        PlanningAdminDraftKind::FullPlanning => matches!(
            key,
            PlanningAdminFileKey::Directions
                | PlanningAdminFileKey::TaskLedger
                | PlanningAdminFileKey::ResultOutput
        )
        .then_some(key),
        PlanningAdminDraftKind::Directions => matches!(
            key,
            PlanningAdminFileKey::Directions | PlanningAdminFileKey::QueueIdlePrompt
        )
        .then_some(key),
        PlanningAdminDraftKind::TaskLedger => matches!(
            key,
            PlanningAdminFileKey::TaskLedger | PlanningAdminFileKey::ResultOutput
        )
        .then_some(key),
        PlanningAdminDraftKind::QueueIdlePrompt => matches!(
            key,
            PlanningAdminFileKey::Directions | PlanningAdminFileKey::QueueIdlePrompt
        )
        .then_some(key),
        PlanningAdminDraftKind::DirectionDetail => matches!(
            key,
            PlanningAdminFileKey::Directions | PlanningAdminFileKey::DirectionDetail
        )
        .then_some(key),
    }
}

fn map_doctor_report(report: &PlanningDoctorReport) -> PlanningAdminDoctorSummary {
    PlanningAdminDoctorSummary {
        planning_state: report.planning_state().label().to_string(),
        queue_idle_policy: report.queue_idle_policy().map(str::to_string),
        queue_summary: report.queue_summary().map(str::to_string),
        proposal_summary: report.proposal_summary().map(str::to_string),
        health: report.health().map(str::to_string),
        issue: report.issue().map(str::to_string),
        note: report.note().map(str::to_string),
    }
}

fn map_runtime_snapshot(snapshot: &PlanningRuntimeSnapshot) -> PlanningAdminRuntimeSummary {
    let queue_preview = snapshot.queue_snapshot().cloned().map(map_queue_preview);
    PlanningAdminRuntimeSummary {
        workspace_present: snapshot.workspace_present(),
        plan_enabled: snapshot.plan_enabled(),
        preview_status_label: snapshot.preview_status_label().to_string(),
        preview_detail: snapshot.preview_detail().map(str::to_string),
        queue_head: queue_preview
            .as_ref()
            .and_then(|preview| preview.queue_head.clone()),
        visible_tasks: queue_preview
            .as_ref()
            .map(|preview| preview.visible_tasks.clone())
            .unwrap_or_default(),
        proposed_tasks: queue_preview
            .as_ref()
            .map(|preview| preview.proposed_tasks.clone())
            .unwrap_or_default(),
    }
}

fn map_directions_summary(
    summary: DirectionsMaintenanceSummary,
) -> PlanningAdminDirectionsSummaryView {
    PlanningAdminDirectionsSummaryView {
        missing_detail_doc_count: summary.missing_detail_doc_count,
        broken_detail_doc_count: summary.broken_detail_doc_count,
        queue_idle_policy: summary.queue_idle_policy.label().to_string(),
        queue_idle_prompt_path: summary.queue_idle_prompt_path,
        queue_idle_prompt_status: summary.queue_idle_prompt_status.label().to_string(),
        parse_error: summary.parse_error,
        directions: summary
            .directions
            .into_iter()
            .map(|direction| PlanningAdminDirectionSummaryView {
                id: direction.id,
                title: direction.title,
                detail_doc_path: direction.detail_doc_path,
                detail_doc_status: direction.detail_doc_status.label().to_string(),
                needs_attention: direction.detail_doc_status.needs_attention(),
            })
            .collect(),
    }
}

fn map_validation_report(report: &PlanningValidationReport) -> PlanningAdminValidationView {
    let error_count = report.errors().len();
    let warning_count = report
        .issues
        .iter()
        .filter(|issue| {
            issue.severity != crate::domain::planning::PlanningValidationSeverity::Error
        })
        .count();
    PlanningAdminValidationView {
        is_valid: report.is_valid(),
        error_count,
        warning_count,
        issues: report
            .issues
            .iter()
            .map(|issue| PlanningAdminValidationIssueView {
                severity: match issue.severity {
                    crate::domain::planning::PlanningValidationSeverity::Error => {
                        "error".to_string()
                    }
                    crate::domain::planning::PlanningValidationSeverity::Warning => {
                        "warning".to_string()
                    }
                },
                file_kind: match issue.file_kind {
                    PlanningFileKind::Directions => "directions".to_string(),
                    PlanningFileKind::TaskLedger => "task_ledger".to_string(),
                    PlanningFileKind::TaskLedgerSchema => "task_ledger_schema".to_string(),
                    PlanningFileKind::ResultOutput => "result_output".to_string(),
                },
                code: issue.code.clone(),
                message: issue.message.clone(),
            })
            .collect(),
    }
}

fn map_queue_preview(snapshot: PriorityQueueSnapshot) -> PlanningAdminQueuePreview {
    PlanningAdminQueuePreview {
        queue_summary: match snapshot.next_task.as_ref() {
            Some(task) => format!("now: {}", task.task_title.trim()),
            None => "next task: none".to_string(),
        },
        proposal_summary: snapshot
            .proposed_tasks
            .first()
            .map(|task| task.task_title.trim().to_string()),
        queue_head: snapshot
            .next_task
            .as_ref()
            .map(|task| PlanningAdminQueueHeadView {
                task_id: task.task_id.clone(),
                task_title: task.task_title.clone(),
                direction_id: task.direction_id.clone(),
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at.clone(),
                rank_reasons: task.rank_reasons.clone(),
            }),
        visible_tasks: snapshot
            .visible_tasks(5)
            .into_iter()
            .map(|task| PlanningAdminQueueTaskView {
                task_id: task.task_id,
                task_title: task.task_title,
                direction_id: task.direction_id,
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at,
            })
            .collect(),
        proposed_tasks: snapshot
            .visible_proposed_tasks(5)
            .into_iter()
            .map(|task| PlanningAdminQueueTaskView {
                task_id: task.task_id,
                task_title: task.task_title,
                direction_id: task.direction_id,
                status: task.status.label().to_string(),
                combined_priority: task.combined_priority,
                updated_at: task.updated_at,
            })
            .collect(),
    }
}
