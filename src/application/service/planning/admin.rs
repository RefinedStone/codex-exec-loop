use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult,
    PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
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
use crate::domain::parallel_mode::ParallelModeQueueItemState;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PlanningFileKind,
    PlanningValidationReport, PlanningWorkspaceFiles, PriorityQueueSnapshot, TaskActor,
    TaskDefinition, TaskLedgerDocument, TaskStatus,
};

const DEFAULT_DIRECTION_ID: &str = "general-workstream";
const GENERATED_DIRECTION_ID_PREFIX: &str = "dir";
const GENERATED_TASK_ID_PREFIX: &str = "task";

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
            Self::TaskLedger => "task catalog",
            Self::QueueIdlePrompt => "queue-idle prompt",
            Self::DirectionDetail => "direction detail",
        }
    }

    pub fn editor_heading(self) -> &'static str {
        match self {
            Self::FullPlanning => "Full Planning Draft",
            Self::Directions => "Directions Draft",
            Self::TaskLedger => "Task Catalog Draft",
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
            Self::TaskLedger => "Task Catalog",
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
pub struct PlanningAdminDirectionManagementView {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub success_criteria_text: String,
    pub scope_hints_text: String,
    pub detail_doc_path: String,
    pub state: String,
    pub task_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminTaskManagementView {
    pub id: String,
    pub direction_id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub base_priority: i32,
    pub dynamic_priority_delta: i32,
    pub priority_reason: String,
    pub depends_on_text: String,
    pub blocked_by_text: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminManagementView {
    pub default_direction_id: String,
    pub directions: Vec<PlanningAdminDirectionManagementView>,
    pub tasks: Vec<PlanningAdminTaskManagementView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDirectionMutationRequest {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub success_criteria_text: String,
    #[serde(default)]
    pub scope_hints_text: String,
    #[serde(default)]
    pub detail_doc_path: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminDirectionDeleteRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminTaskMutationRequest {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub direction_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub base_priority: String,
    #[serde(default)]
    pub dynamic_priority_delta: String,
    #[serde(default)]
    pub priority_reason: String,
    #[serde(default)]
    pub depends_on_text: String,
    #[serde(default)]
    pub blocked_by_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningAdminTaskDeleteRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminCrudOutcome {
    pub notice: String,
    pub management: PlanningAdminManagementView,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanningAdminFileSyncOutcome {
    pub notice: String,
    pub paths: Vec<String>,
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
    planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}

impl PlanningAdminFacadeService {
    pub fn from_planning(
        workspace_dir: impl Into<String>,
        planning: PlanningServices,
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> Self {
        Self::from_planning_with_authority(
            workspace_dir,
            planning,
            planning_workspace_port,
            Arc::new(super::NoopPlanningAuthorityPort::default()),
            Arc::new(NoopPlanningTaskRepositoryPort),
        )
    }

    pub fn from_planning_with_authority(
        workspace_dir: impl Into<String>,
        planning: PlanningServices,
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_authority_port: Arc<dyn PlanningAuthorityPort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            planning,
            planning_workspace_port,
            planning_authority_port,
            planning_task_repository_port,
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

    pub fn load_management_view(&self) -> Result<PlanningAdminManagementView> {
        let documents = self.load_admin_documents()?;
        Ok(map_management_view(
            &documents.directions,
            &documents.task_ledger,
            default_direction_id(&documents.directions)?,
        ))
    }

    pub fn upsert_direction(
        &self,
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let mut documents = self.load_admin_documents()?;
        let direction = direction_from_request(request, &documents.directions)?;
        let id = direction.id.clone();
        let mut updated = false;
        for existing in &mut documents.directions.directions {
            if existing.id.trim() == id {
                *existing = direction.clone();
                updated = true;
                break;
            }
        }
        if !updated {
            documents.directions.directions.push(direction);
        }
        self.commit_admin_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: if updated {
                format!("direction `{id}` updated")
            } else {
                format!("direction `{id}` added")
            },
            management,
        })
    }

    pub fn delete_direction(
        &self,
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let direction_id = normalized_required_id(&request.id, "direction id")?;
        let mut documents = self.load_admin_documents()?;
        if direction_id == DEFAULT_DIRECTION_ID {
            ensure_default_direction(&mut documents.directions)?;
            self.commit_admin_documents(documents)?;
            let management = self.load_management_view()?;
            return Ok(PlanningAdminCrudOutcome {
                notice: format!("default direction `{DEFAULT_DIRECTION_ID}` is retained"),
                management,
            });
        }
        let original_count = documents.directions.directions.len();
        documents
            .directions
            .directions
            .retain(|direction| direction.id.trim() != direction_id);
        if documents.directions.directions.len() == original_count {
            bail!("direction `{direction_id}` was not found");
        }

        let removed_task_ids = documents
            .task_ledger
            .tasks
            .iter()
            .filter(|task| task.direction_id.trim() == direction_id)
            .map(|task| task.id.trim().to_string())
            .collect::<BTreeSet<_>>();
        documents
            .task_ledger
            .tasks
            .retain(|task| task.direction_id.trim() != direction_id);
        remove_task_references(&mut documents.task_ledger, &removed_task_ids);

        let removed_task_count = removed_task_ids.len();
        ensure_default_direction(&mut documents.directions)?;
        self.commit_admin_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: format!(
                "direction `{direction_id}` deleted with {removed_task_count} child tasks"
            ),
            management,
        })
    }

    pub fn upsert_task(
        &self,
        request: PlanningAdminTaskMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let mut documents = self.load_admin_documents()?;
        ensure_default_direction(&mut documents.directions)?;
        let default_direction_id = default_direction_id(&documents.directions)?;
        let task = task_from_request(request, &documents.task_ledger, default_direction_id)?;
        ensure_direction_exists(&documents.directions, &task.direction_id)?;
        let task_id = task.id.clone();
        let mut updated = false;
        for existing in &mut documents.task_ledger.tasks {
            if existing.id.trim() == task_id {
                *existing = task.clone();
                updated = true;
                break;
            }
        }
        if !updated {
            documents.task_ledger.tasks.push(task);
        }
        self.commit_admin_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: if updated {
                format!("task `{task_id}` updated")
            } else {
                format!("task `{task_id}` added")
            },
            management,
        })
    }

    pub fn delete_task(
        &self,
        request: PlanningAdminTaskDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        let task_id = normalized_required_id(&request.id, "task id")?;
        let mut documents = self.load_admin_documents()?;
        let original_count = documents.task_ledger.tasks.len();
        documents
            .task_ledger
            .tasks
            .retain(|task| task.id.trim() != task_id);
        if documents.task_ledger.tasks.len() == original_count {
            bail!("task `{task_id}` was not found");
        }
        remove_task_references(
            &mut documents.task_ledger,
            &BTreeSet::from([task_id.to_string()]),
        );
        self.commit_admin_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: format!("task `{task_id}` deleted"),
            management,
        })
    }

    pub fn export_active_files_for_edit(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.ensure_no_parallel_working("export planning files")?;
        let documents = self.load_admin_documents()?;
        let mut paths = Vec::new();
        write_candidate_file(
            &self.workspace_dir,
            DIRECTIONS_FILE_PATH,
            &toml::to_string_pretty(&documents.directions)?,
            &mut paths,
        )?;
        write_candidate_file(
            &self.workspace_dir,
            TASK_LEDGER_FILE_PATH,
            &serde_json::to_string_pretty(&documents.task_ledger)?,
            &mut paths,
        )?;
        write_candidate_file(
            &self.workspace_dir,
            TASK_LEDGER_SCHEMA_FILE_PATH,
            &documents.task_ledger_schema_json,
            &mut paths,
        )?;
        write_candidate_file(
            &self.workspace_dir,
            RESULT_OUTPUT_FILE_PATH,
            &documents.result_output_markdown,
            &mut paths,
        )?;
        Ok(PlanningAdminFileSyncOutcome {
            notice: format!("exported {} planning files for editing", paths.len()),
            paths,
        })
    }

    pub fn apply_exported_files(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.ensure_no_parallel_working("apply exported planning files")?;
        let directions_result = self
            .planning
            .workspace
            .apply_tracked_directions(self.workspace_dir.as_str())?;
        if !directions_result.validation_report.is_valid() {
            bail!("tracked directions apply failed validation");
        }
        let task_result = self
            .planning
            .workspace
            .apply_tracked_task_ledger(self.workspace_dir.as_str())?;
        if !task_result.validation_report.is_valid() {
            bail!("tracked task catalog apply failed validation");
        }
        let mut paths = directions_result.applied_paths;
        paths.extend(task_result.applied_paths);
        paths.sort();
        paths.dedup();
        Ok(PlanningAdminFileSyncOutcome {
            notice: format!("applied {} exported planning paths", paths.len()),
            paths,
        })
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

    fn load_admin_documents(&self) -> Result<PlanningAdminDocuments> {
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(self.workspace_dir.as_str())?;
        let directions_toml = workspace.directions_toml.ok_or_else(|| {
            anyhow!("planning directions are unavailable; initialize planning first")
        })?;
        let task_ledger_json = workspace
            .task_ledger_json
            .ok_or_else(|| anyhow!("task-ledger.json is unavailable; initialize planning first"))?;
        let task_ledger_schema_json = workspace.task_ledger_schema_json.ok_or_else(|| {
            anyhow!("task-ledger.schema.json is unavailable; initialize planning first")
        })?;
        let result_output_markdown = workspace
            .result_output_markdown
            .ok_or_else(|| anyhow!("result-output.md is unavailable; initialize planning first"))?;
        let task_authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(self.workspace_dir.as_str())?;
        let directions = toml::from_str::<DirectionCatalogDocument>(&directions_toml)
            .context("failed to parse active directions.toml")?;
        let task_ledger = match task_authority_snapshot.as_ref() {
            Some(snapshot) => snapshot.task_ledger.clone(),
            None => serde_json::from_str::<TaskLedgerDocument>(&task_ledger_json)
                .context("failed to parse active task-ledger.json")?,
        };
        Ok(PlanningAdminDocuments {
            directions,
            task_ledger,
            task_ledger_schema_json,
            result_output_markdown,
            observed_planning_revision: task_authority_snapshot
                .as_ref()
                .map(|snapshot| snapshot.planning_revision),
            uses_task_repository: task_authority_snapshot.is_some(),
        })
    }

    fn commit_admin_documents(&self, mut documents: PlanningAdminDocuments) -> Result<()> {
        ensure_default_direction(&mut documents.directions)?;
        if documents.uses_task_repository {
            self.restore_task_referenced_directions_from_tracked_file(&mut documents)?;
        }
        reassign_unresolved_task_directions_to_default(&mut documents)?;

        let directions_toml = toml::to_string_pretty(&documents.directions)?;
        let task_ledger_json = serde_json::to_string_pretty(&documents.task_ledger)?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: &directions_toml,
                    task_ledger_json: &task_ledger_json,
                    task_ledger_schema_json: &documents.task_ledger_schema_json,
                    result_output_markdown: &documents.result_output_markdown,
                });
        if !validation_result.report.is_valid() {
            bail!(
                "planning mutation failed validation: {}",
                validation_result
                    .report
                    .issues
                    .iter()
                    .map(|issue| issue.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        let queue_snapshot = self
            .priority_queue_service
            .build_snapshot(&documents.directions, &documents.task_ledger)
            .context("failed to rebuild planning queue")?;

        if documents.uses_task_repository {
            match self
                .planning_task_repository_port
                .commit_task_authority_snapshot(
                    self.workspace_dir.as_str(),
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: documents.observed_planning_revision,
                        task_ledger: &documents.task_ledger,
                        queue_snapshot: &queue_snapshot,
                    },
                )? {
                PlanningTaskAuthorityCommitResult::Committed { .. } => {}
                PlanningTaskAuthorityCommitResult::Conflict {
                    observed_planning_revision,
                    current_planning_revision,
                } => {
                    bail!(
                        "planning db changed while editing (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                    );
                }
            }
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    self.workspace_dir.as_str(),
                    DIRECTIONS_FILE_PATH,
                    Some(&directions_toml),
                )?;
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    self.workspace_dir.as_str(),
                    RESULT_OUTPUT_FILE_PATH,
                    Some(&documents.result_output_markdown),
                )?;
            return Ok(());
        }

        self.planning_workspace_port.commit_planning_workspace_files(
            self.workspace_dir.as_str(),
            &crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord {
                directions_toml: Some(directions_toml),
                task_ledger_json: Some(task_ledger_json),
                task_ledger_schema_json: Some(documents.task_ledger_schema_json),
                queue_snapshot_json: Some(serde_json::to_string_pretty(&queue_snapshot)?),
                result_output_markdown: Some(documents.result_output_markdown),
            },
        )
    }

    fn restore_task_referenced_directions_from_tracked_file(
        &self,
        documents: &mut PlanningAdminDocuments,
    ) -> Result<()> {
        let existing_direction_ids = documents
            .directions
            .directions
            .iter()
            .map(|direction| direction.id.trim().to_string())
            .collect::<BTreeSet<_>>();
        let missing_direction_ids = documents
            .task_ledger
            .tasks
            .iter()
            .map(|task| task.direction_id.trim().to_string())
            .filter(|direction_id| !direction_id.is_empty())
            .filter(|direction_id| !existing_direction_ids.contains(direction_id))
            .collect::<BTreeSet<_>>();
        if missing_direction_ids.is_empty() {
            return Ok(());
        }

        let tracked_path = Path::new(&self.workspace_dir).join(DIRECTIONS_FILE_PATH);
        let tracked_directions_toml = match fs::read_to_string(&tracked_path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read tracked {}", tracked_path.display()));
            }
        };
        let tracked_directions =
            toml::from_str::<DirectionCatalogDocument>(&tracked_directions_toml)
                .with_context(|| format!("failed to parse tracked {}", tracked_path.display()))?;
        let tracked_by_id = tracked_directions
            .directions
            .into_iter()
            .map(|direction| (direction.id.trim().to_string(), direction))
            .collect::<BTreeMap<_, _>>();

        for direction_id in missing_direction_ids {
            if let Some(direction) = tracked_by_id.get(&direction_id) {
                documents.directions.directions.push(direction.clone());
            }
        }
        Ok(())
    }

    fn ensure_no_parallel_working(&self, action: &str) -> Result<()> {
        let runtime = self
            .planning_authority_port
            .load_runtime_projections(self.workspace_dir.as_str())?;
        if let Some(reason) = describe_parallel_busy(&runtime) {
            bail!("{action} is blocked while parallel work is active: {reason}");
        }
        Ok(())
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

#[derive(Debug, Clone)]
struct PlanningAdminDocuments {
    directions: DirectionCatalogDocument,
    task_ledger: TaskLedgerDocument,
    task_ledger_schema_json: String,
    result_output_markdown: String,
    observed_planning_revision: Option<i64>,
    uses_task_repository: bool,
}

fn map_management_view(
    directions: &DirectionCatalogDocument,
    task_ledger: &TaskLedgerDocument,
    default_direction_id: &str,
) -> PlanningAdminManagementView {
    let mut task_counts = BTreeMap::<String, usize>::new();
    for task in &task_ledger.tasks {
        *task_counts
            .entry(task.direction_id.trim().to_string())
            .or_default() += 1;
    }

    PlanningAdminManagementView {
        default_direction_id: default_direction_id.to_string(),
        directions: directions
            .directions
            .iter()
            .map(|direction| PlanningAdminDirectionManagementView {
                id: direction.id.clone(),
                title: direction.title.clone(),
                summary: direction.summary.clone(),
                success_criteria_text: direction.success_criteria.join("\n"),
                scope_hints_text: direction.scope_hints.join("\n"),
                detail_doc_path: direction.detail_doc_path.clone(),
                state: direction_state_label(direction.state).to_string(),
                task_count: task_counts
                    .get(direction.id.trim())
                    .copied()
                    .unwrap_or_default(),
            })
            .collect(),
        tasks: task_ledger
            .tasks
            .iter()
            .map(|task| PlanningAdminTaskManagementView {
                id: task.id.clone(),
                direction_id: task.direction_id.clone(),
                title: task.title.clone(),
                description: task.description.clone(),
                status: task.status.label().to_string(),
                base_priority: task.base_priority,
                dynamic_priority_delta: task.dynamic_priority_delta,
                priority_reason: task.priority_reason.clone(),
                depends_on_text: task.depends_on.join("\n"),
                blocked_by_text: task.blocked_by.join("\n"),
                updated_at: task.updated_at.clone(),
            })
            .collect(),
    }
}

fn direction_from_request(
    request: PlanningAdminDirectionMutationRequest,
    directions: &DirectionCatalogDocument,
) -> Result<DirectionDefinition> {
    let title = normalized_required_text(&request.title, "direction title")?;
    let id = if request.id.trim().is_empty() {
        generated_unique_id(
            GENERATED_DIRECTION_ID_PREFIX,
            title,
            directions
                .directions
                .iter()
                .map(|direction| direction.id.trim()),
        )
    } else {
        normalized_required_id(&request.id, "direction id")?.to_string()
    };
    let success_criteria = split_lines(&request.success_criteria_text);
    if success_criteria.is_empty() {
        bail!("direction `{id}` requires at least one success criterion");
    }
    Ok(DirectionDefinition {
        id,
        title: title.to_string(),
        summary: non_empty_or(&request.summary, title),
        success_criteria,
        scope_hints: split_lines(&request.scope_hints_text),
        detail_doc_path: request.detail_doc_path.trim().to_string(),
        state: parse_direction_state(&request.state)?,
    })
}

fn task_from_request(
    request: PlanningAdminTaskMutationRequest,
    task_ledger: &TaskLedgerDocument,
    default_direction_id: &str,
) -> Result<TaskDefinition> {
    let title = normalized_required_text(&request.title, "task title")?;
    let id = if request.id.trim().is_empty() {
        generated_unique_id(
            GENERATED_TASK_ID_PREFIX,
            title,
            task_ledger.tasks.iter().map(|task| task.id.trim()),
        )
    } else {
        normalized_required_id(&request.id, "task id")?.to_string()
    };
    let now = Utc::now().to_rfc3339();
    let existing = task_ledger
        .tasks
        .iter()
        .find(|task| task.id.trim() == id.as_str())
        .cloned();
    let direction_id = if request.direction_id.trim().is_empty() {
        default_direction_id.to_string()
    } else {
        normalized_required_id(&request.direction_id, "direction id")?.to_string()
    };
    let base_priority = parse_i32_or_default(&request.base_priority, 10, "base priority")?;
    let dynamic_priority_delta =
        parse_i32_or_default(&request.dynamic_priority_delta, 0, "dynamic priority delta")?;
    Ok(TaskDefinition {
        id,
        direction_id,
        direction_relation_note: existing
            .as_ref()
            .map(|task| task.direction_relation_note.clone())
            .unwrap_or_default(),
        title: title.to_string(),
        description: non_empty_or(&request.description, title),
        status: parse_task_status(&request.status)?,
        base_priority,
        dynamic_priority_delta,
        priority_reason: request.priority_reason.trim().to_string(),
        depends_on: split_references(&request.depends_on_text),
        blocked_by: split_references(&request.blocked_by_text),
        created_by: existing
            .as_ref()
            .map(|task| task.created_by)
            .unwrap_or(TaskActor::User),
        last_updated_by: TaskActor::User,
        source_turn_id: existing.and_then(|task| task.source_turn_id),
        updated_at: now,
    })
}

fn ensure_default_direction(directions: &mut DirectionCatalogDocument) -> Result<()> {
    if directions
        .directions
        .iter()
        .any(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
    {
        return Ok(());
    }
    directions.directions.push(default_direction_definition()?);
    Ok(())
}

fn default_direction_definition() -> Result<DirectionDefinition> {
    let artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    let directions = toml::from_str::<DirectionCatalogDocument>(&artifacts.directions_toml)
        .context("failed to parse bootstrap default directions")?;
    directions
        .directions
        .into_iter()
        .find(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
        .ok_or_else(|| anyhow!("bootstrap default direction `{DEFAULT_DIRECTION_ID}` is missing"))
}

fn reassign_unresolved_task_directions_to_default(
    documents: &mut PlanningAdminDocuments,
) -> Result<()> {
    ensure_default_direction(&mut documents.directions)?;
    let direction_ids = documents
        .directions
        .directions
        .iter()
        .map(|direction| direction.id.trim().to_string())
        .collect::<BTreeSet<_>>();
    for task in &mut documents.task_ledger.tasks {
        if !direction_ids.contains(task.direction_id.trim()) {
            task.direction_id = DEFAULT_DIRECTION_ID.to_string();
        }
    }
    Ok(())
}

fn parse_direction_state(raw: &str) -> Result<DirectionState> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "active" => Ok(DirectionState::Active),
        "paused" => Ok(DirectionState::Paused),
        "done" => Ok(DirectionState::Done),
        other => bail!("unknown direction state `{other}`"),
    }
}

fn parse_task_status(raw: &str) -> Result<TaskStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "ready" => Ok(TaskStatus::Ready),
        "blocked" => Ok(TaskStatus::Blocked),
        "in_progress" => Ok(TaskStatus::InProgress),
        "done" => Ok(TaskStatus::Done),
        "cancelled" => Ok(TaskStatus::Cancelled),
        "awaiting_user" => Ok(TaskStatus::AwaitingUser),
        "proposed" => Ok(TaskStatus::Proposed),
        other => bail!("unknown task status `{other}`"),
    }
}

fn default_direction_id(directions: &DirectionCatalogDocument) -> Result<&str> {
    if let Some(direction) = directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
    {
        return Ok(direction.id.trim());
    }
    directions
        .directions
        .iter()
        .find(|direction| direction.state == DirectionState::Active)
        .or_else(|| directions.directions.first())
        .map(|direction| direction.id.trim())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow!("at least one direction is required"))
}

fn ensure_direction_exists(
    directions: &DirectionCatalogDocument,
    direction_id: &str,
) -> Result<()> {
    if directions
        .directions
        .iter()
        .any(|direction| direction.id.trim() == direction_id.trim())
    {
        return Ok(());
    }
    bail!("direction `{}` does not exist", direction_id.trim())
}

fn normalized_required_id<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    if value.contains(char::is_whitespace) || value.contains('/') || value.contains('\\') {
        bail!("{label} `{value}` must not contain whitespace or path separators");
    }
    Ok(value)
}

fn normalized_required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn generated_unique_id<'a>(
    prefix: &str,
    title: &str,
    existing_ids: impl IntoIterator<Item = &'a str>,
) -> String {
    let existing = existing_ids.into_iter().collect::<BTreeSet<_>>();
    let slug = slugify_title(title);
    let base = format!("{prefix}-{slug}");
    if !existing.contains(base.as_str()) {
        return base;
    }

    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!("numeric suffix search should eventually find an unused id")
}

fn slugify_title(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "item".to_string()
    } else {
        slug
    }
}

fn parse_i32_or_default(raw: &str, default: i32, label: &str) -> Result<i32> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(default);
    }
    raw.parse::<i32>()
        .with_context(|| format!("{label} must be an integer"))
}

fn split_lines(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn split_references(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{generated_unique_id, slugify_title};

    #[test]
    fn slugify_title_preserves_unicode_alphanumerics() {
        assert_eq!(slugify_title("한글 작업 1"), "한글-작업-1");
    }

    #[test]
    fn generated_unique_id_keeps_unicode_title_identity() {
        let existing = ["task-한글-작업", "task-한글-작업-2"];

        assert_eq!(
            generated_unique_id("task", "한글 작업", existing),
            "task-한글-작업-3"
        );
    }
}

fn remove_task_references(
    task_ledger: &mut TaskLedgerDocument,
    removed_task_ids: &BTreeSet<String>,
) {
    for task in &mut task_ledger.tasks {
        task.depends_on
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
        task.blocked_by
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
    }
}

fn direction_state_label(state: DirectionState) -> &'static str {
    match state {
        DirectionState::Active => "active",
        DirectionState::Paused => "paused",
        DirectionState::Done => "done",
    }
}

fn describe_parallel_busy(runtime: &PlanningAuthorityRuntimeProjectionSnapshot) -> Option<String> {
    if let Some(lease) = runtime.slot_leases.values().find(|lease| {
        matches!(
            lease.state,
            crate::domain::parallel_mode::ParallelModeSlotLeaseState::Leased
                | crate::domain::parallel_mode::ParallelModeSlotLeaseState::Running
                | crate::domain::parallel_mode::ParallelModeSlotLeaseState::CleanupPending
        )
    }) {
        return Some(format!(
            "slot {} is {} for task {}",
            lease.slot_id,
            lease.state.label(),
            lease.task_id
        ));
    }
    if let Some(record) = runtime
        .distributor_queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
    {
        let state = match record.queue_state {
            ParallelModeQueueItemState::Idle => "idle",
            ParallelModeQueueItemState::Queued => "queued",
            ParallelModeQueueItemState::Pushing => "pushing",
            ParallelModeQueueItemState::PrPending => "pr pending",
            ParallelModeQueueItemState::MergePending => "merge pending",
            ParallelModeQueueItemState::Integrating => "integrating",
            ParallelModeQueueItemState::Cleaning => "cleaning",
            ParallelModeQueueItemState::Done => "done",
            ParallelModeQueueItemState::Blocked => "blocked",
            ParallelModeQueueItemState::Failed => "failed",
        };
        return Some(format!(
            "distributor item {} is {} for task {}",
            record.queue_item_id, state, record.task_id
        ));
    }
    None
}

fn write_candidate_file(
    workspace_dir: &str,
    relative_path: &str,
    body: &str,
    written_paths: &mut Vec<String>,
) -> Result<()> {
    let path = Path::new(workspace_dir).join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    written_paths.push(relative_path.to_string());
    Ok(())
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
            PlanningFileKind::TaskLedger => "task catalog compatibility file",
            PlanningFileKind::TaskLedgerSchema => "task catalog compatibility schema",
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
