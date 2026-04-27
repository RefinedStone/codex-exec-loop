use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

mod documents;
mod draft_session;
mod projection;

use crate::application::port::outbound::planning_authority_port::{
    NoopPlanningAuthorityPort, PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::{
    DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH,
};
use crate::application::service::planning::{
    PlanningDraftPromoteResult, PlanningDraftSaveResult, PlanningResetTarget, PlanningServices,
};
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::domain::parallel_mode::ParallelModeQueueItemState;

use self::documents::{
    default_direction_id, direction_from_request, ensure_default_direction,
    ensure_direction_exists, normalized_required_id, remove_task_references, task_from_request,
};
use self::projection::{
    map_directions_summary, map_doctor_report, map_management_view, map_runtime_snapshot,
};

const DEFAULT_DIRECTION_ID: &str = documents::DEFAULT_DIRECTION_ID;

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
            Arc::new(NoopPlanningAuthorityPort::default()),
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

    fn ensure_no_parallel_working(&self, action: &str) -> Result<()> {
        let runtime = self
            .planning_authority_port
            .load_runtime_projections(self.workspace_dir.as_str())?;
        if let Some(reason) = describe_parallel_busy(&runtime) {
            bail!("{action} is blocked while parallel work is active: {reason}");
        }
        Ok(())
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
