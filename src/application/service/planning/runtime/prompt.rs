/*
 * This module is the runtime snapshot boundary for planning.  It reads the
 * operator-facing workspace files, joins them with the DB-backed direction/task
 * authority, validates the combined view, and lowers it into a compact
 * PlanningRuntimeSnapshot consumed by policy.rs, facade.rs, TUI overlays, and
 * auto-follow prompt assembly.
 *
 * The key design point is that runtime readers still use a file-shaped
 * validation contract, but task authority is no longer trusted from an operator
 * file.  It is serialized from the accepted DB snapshot and then passed through
 * the same validator so older validation and prompt-fragment code can remain
 * narrow while the source of truth stays authoritative.
 */
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningWorkspaceFiles, PriorityQueueProjection, PriorityQueueTask,
    QueueIdlePolicy, TaskAuthorityDocument, TaskDefinition,
};
use anyhow::{Context, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod fragment;

use self::fragment::{build_prompt_fragment, trimmed_non_empty};

const MAX_PROPOSAL_SUMMARY_TITLES: usize = 2;

#[derive(Clone)]
pub struct PlanningPromptService {
    // Workspace and repository ports are kept separate because runtime prompt
    // loading combines two authority planes: operator-authored markdown files
    // and DB-accepted planning authority.
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    authority_seed_service: PlanningAuthoritySeedService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningRuntimeWorkspaceStatus {
    Uninitialized,
    Invalid,
    ReadyNoTask,
    ReadyWithTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeSnapshot {
    /*
     * A snapshot is intentionally immutable outside this module.  Policy and UI
     * code should observe derived facts, not recompute whether a workspace is
     * invalid, actionable, repeated, or just proposal-only.  Keeping fields
     * private preserves the relationship between status, queue head, prompt
     * fragment, failure text, and authority signatures.
     */
    workspace_present: bool,
    workspace_status: PlanningRuntimeWorkspaceStatus,
    prompt_fragment: Option<String>,
    queue_summary: Option<String>,
    proposal_summary: Option<String>,
    queue_idle_policy: QueueIdlePolicy,
    queue_idle_prompt_path: Option<String>,
    queue_head: Option<PriorityQueueTask>,
    queue_projection: Option<PriorityQueueProjection>,
    task_authority_signature: Option<u64>,
    queue_head_task_signature: Option<u64>,
    failure_reason: Option<String>,
    auto_followup_pause_reason: Option<String>,
}

impl PlanningRuntimeSnapshot {
    // No files means planning has not started; this is different from a partial
    // workspace, which should be repairable and therefore invalid.
    pub fn uninitialized() -> Self {
        Self {
            workspace_present: false,
            workspace_status: PlanningRuntimeWorkspaceStatus::Uninitialized,
            prompt_fragment: None,
            queue_summary: None,
            proposal_summary: None,
            queue_idle_policy: QueueIdlePolicy::Stop,
            queue_idle_prompt_path: None,
            queue_head: None,
            queue_projection: None,
            task_authority_signature: None,
            queue_head_task_signature: None,
            failure_reason: None,
            auto_followup_pause_reason: None,
        }
    }

    // Invalid snapshots still mark the workspace as present by default so TUI
    // repair/doctor flows can explain a broken planning workspace instead of
    // hiding planning as inactive.
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self {
            workspace_present: true,
            workspace_status: PlanningRuntimeWorkspaceStatus::Invalid,
            prompt_fragment: None,
            queue_summary: None,
            proposal_summary: None,
            queue_idle_policy: QueueIdlePolicy::Stop,
            queue_idle_prompt_path: None,
            queue_head: None,
            queue_projection: None,
            task_authority_signature: None,
            queue_head_task_signature: None,
            failure_reason: Some(reason.into()),
            auto_followup_pause_reason: None,
        }
    }

    pub fn ready(
        prompt_fragment: String,
        queue_summary: String,
        queue_head: Option<PriorityQueueTask>,
    ) -> Self {
        Self::ready_with_details(prompt_fragment, queue_summary, None, queue_head)
    }

    // Test and projection callers can build a ready snapshot without retaining
    // the full queue projection.  Runtime loading uses the richer constructor
    // below so detailed TUI surfaces can inspect active/proposed/skipped tasks.
    pub fn ready_with_details(
        prompt_fragment: String,
        queue_summary: String,
        proposal_summary: Option<String>,
        queue_head: Option<PriorityQueueTask>,
    ) -> Self {
        Self {
            workspace_present: true,
            workspace_status: if queue_head.is_some() {
                PlanningRuntimeWorkspaceStatus::ReadyWithTask
            } else {
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            },
            prompt_fragment: Some(prompt_fragment),
            queue_summary: Some(queue_summary),
            proposal_summary,
            queue_idle_policy: QueueIdlePolicy::Stop,
            queue_idle_prompt_path: None,
            queue_head,
            queue_projection: None,
            task_authority_signature: None,
            queue_head_task_signature: None,
            failure_reason: None,
            auto_followup_pause_reason: None,
        }
    }

    pub fn ready_with_queue_projection(
        prompt_fragment: String,
        queue_summary: String,
        proposal_summary: Option<String>,
        queue_head: Option<PriorityQueueTask>,
        queue_projection: PriorityQueueProjection,
    ) -> Self {
        /*
         * This is the full-fidelity ready constructor.  The queue head determines
         * whether policy may generate a continuation, while the full projection
         * remains available for UI panes that need more than a single summary.
         */
        Self {
            workspace_present: true,
            workspace_status: if queue_head.is_some() {
                PlanningRuntimeWorkspaceStatus::ReadyWithTask
            } else {
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            },
            prompt_fragment: Some(prompt_fragment),
            queue_summary: Some(queue_summary),
            proposal_summary,
            queue_idle_policy: QueueIdlePolicy::Stop,
            queue_idle_prompt_path: None,
            queue_head,
            queue_projection: Some(queue_projection),
            task_authority_signature: None,
            queue_head_task_signature: None,
            failure_reason: None,
            auto_followup_pause_reason: None,
        }
    }

    pub fn with_queue_idle_policy(
        mut self,
        policy: QueueIdlePolicy,
        prompt_path: Option<String>,
    ) -> Self {
        self.queue_idle_policy = policy;
        self.queue_idle_prompt_path = prompt_path;
        self
    }

    pub fn with_workspace_present(mut self, present: bool) -> Self {
        self.workspace_present = present;
        self
    }

    pub fn workspace_present(&self) -> bool {
        self.workspace_present
    }

    pub fn workspace_status(&self) -> PlanningRuntimeWorkspaceStatus {
        self.workspace_status
    }

    pub fn prompt_fragment(&self) -> Option<&str> {
        self.prompt_fragment.as_deref()
    }

    pub fn queue_summary(&self) -> Option<&str> {
        self.queue_summary.as_deref()
    }

    pub fn proposal_summary(&self) -> Option<&str> {
        self.proposal_summary.as_deref()
    }

    pub fn queue_head(&self) -> Option<&PriorityQueueTask> {
        self.queue_head.as_ref()
    }

    pub fn queue_idle_policy(&self) -> QueueIdlePolicy {
        self.queue_idle_policy
    }

    pub fn queue_idle_prompt_path(&self) -> Option<&str> {
        self.queue_idle_prompt_path.as_deref()
    }

    pub fn queue_projection(&self) -> Option<&PriorityQueueProjection> {
        self.queue_projection.as_ref()
    }

    // Signatures are coarse change detectors for repeat-queue safeguards.  They
    // are not persistence identifiers; they only let runtime orchestration tell
    // whether accepted authority or the handed-off task changed between turns.
    pub fn task_authority_signature(&self) -> Option<u64> {
        self.task_authority_signature
    }

    pub fn queue_head_task_signature(&self) -> Option<u64> {
        self.queue_head_task_signature
    }

    pub fn failure_reason(&self) -> Option<&str> {
        self.failure_reason.as_deref()
    }

    pub fn auto_followup_pause_reason(&self) -> Option<&str> {
        self.auto_followup_pause_reason.as_deref()
    }

    pub fn with_auto_followup_pause_reason(&self, reason: impl Into<String>) -> Self {
        let mut snapshot = self.clone();
        snapshot.auto_followup_pause_reason = Some(reason.into());
        snapshot
    }

    #[cfg(test)]
    pub(crate) fn with_test_signatures(
        &self,
        task_authority_signature: Option<u64>,
        queue_head_task_signature: Option<u64>,
    ) -> Self {
        let mut snapshot = self.clone();
        snapshot.task_authority_signature = task_authority_signature;
        snapshot.queue_head_task_signature = queue_head_task_signature;
        snapshot
    }

    pub fn preview_status_label(&self) -> &'static str {
        match self.workspace_status {
            PlanningRuntimeWorkspaceStatus::Uninitialized => "inactive",
            PlanningRuntimeWorkspaceStatus::Invalid => "blocked",
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
            | PlanningRuntimeWorkspaceStatus::ReadyWithTask => "ready",
        }
    }

    pub fn preview_detail(&self) -> Option<&str> {
        // Preview text uses the highest operator-actionable detail first:
        // repeated-head pause, validation failure, live queue, then proposals.
        self.auto_followup_pause_reason()
            .or_else(|| self.failure_reason())
            .or_else(|| self.queue_summary())
            .or_else(|| self.proposal_summary())
    }

    pub fn blocks_auto_followup(&self) -> bool {
        self.workspace_status == PlanningRuntimeWorkspaceStatus::Invalid
            || self.auto_followup_pause_reason.is_some()
    }

    pub fn has_actionable_queue_head(&self) -> bool {
        self.workspace_status == PlanningRuntimeWorkspaceStatus::ReadyWithTask
            && self.auto_followup_pause_reason.is_none()
    }

    pub fn has_proposal_candidates(&self) -> bool {
        self.proposal_summary.is_some()
    }
}

impl PlanningPromptService {
    #[cfg(test)]
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
            Arc::new(crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort),
        )
    }

    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            authority_seed_service: PlanningAuthoritySeedService::new(
                planning_workspace_port.clone(),
                planning_task_repository_port.clone(),
                planning_validation_service.clone(),
                priority_queue_service.clone(),
            ),
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
            planning_task_repository_port,
        }
    }

    pub fn load_runtime_snapshot(&self, workspace_dir: &str) -> Result<PlanningRuntimeSnapshot> {
        /*
         * This is the runtime planning read pipeline.  It collapses every
         * recoverable planning problem into an invalid snapshot instead of an
         * application error, because the TUI and repair services need a stable
         * object that can explain incomplete files, validation failures, or
         * queue construction failures.
         */
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let workspace_present = workspace_record.has_any_files();
        if !workspace_present {
            return Ok(PlanningRuntimeSnapshot::uninitialized());
        }

        let missing_paths = missing_workspace_paths(&workspace_record);
        if !missing_paths.is_empty() {
            /*
             * Partial operator files mean planning was started but cannot be
             * trusted.  Treating this as invalid keeps repair/doctor guidance
             * visible instead of silently reverting to an inactive state.
             */
            return Ok(PlanningRuntimeSnapshot::invalid(format!(
                "planning files incomplete: missing {}",
                missing_paths.join(", ")
            ))
            .with_workspace_present(workspace_present));
        }

        // Runtime validation uses accepted DB authority, not task-ledger files,
        // but the validator still expects a file-shaped workspace bundle.
        let task_authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "planning task authority is unavailable; initialize or repair the planning database"
                )
            })?;
        let direction_authority_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "planning direction authority is unavailable; initialize or repair the planning database"
                )
            })?;
        let authority_task_authority_json =
            serde_json::to_string(&task_authority_snapshot.task_authority)
                .context("failed to serialize task authority ledger")?;
        let workspace_files = workspace_record_to_files(
            &workspace_record,
            &direction_authority_snapshot.directions,
            &authority_task_authority_json,
        );

        let mut validation_result = self
            .planning_validation_service
            .validate_workspace_files(workspace_files);
        if let Some(directions) = validation_result.directions.as_ref() {
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| {
                        self.planning_workspace_port
                            .load_optional_planning_file(workspace_dir, path)
                            .ok()
                            .flatten()
                            .is_some()
                    },
                    &mut validation_result.report,
                );
        }

        if !validation_result.is_valid() {
            let first_error = validation_result
                .report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning validation failed".to_string());
            return Ok(PlanningRuntimeSnapshot::invalid(format!(
                "planning validation failed: {first_error}"
            ))
            .with_workspace_present(workspace_present));
        }

        let directions = validation_result
            .directions
            .expect("valid planning directions should be available");
        let task_authority = validation_result
            .task_authority
            .expect("valid planning task ledger should be available");
        let stored_queue_projection = Some(task_authority_snapshot.queue_projection);
        let current_queue_projection = match self
            .priority_queue_service
            .build_projection(&directions, &task_authority)
        {
            Ok(queue_projection) => queue_projection,
            Err(error) => {
                /*
                 * Queue construction can still reject a validated ledger if
                 * execution preconditions are inconsistent.  Surface that as an
                 * invalid planning snapshot so repair paths get a concrete
                 * reason instead of an unhandled runtime failure.
                 */
                return Ok(PlanningRuntimeSnapshot::invalid(format!(
                    "planning queue build failed: {error}"
                ))
                .with_workspace_present(workspace_present));
            }
        };

        // Prefer the stored projection when it still matches the live rebuild.
        // This preserves repository-provided ordering metadata while avoiding
        // stale projections after authority changes.
        let queue_projection = match stored_queue_projection {
            Some(stored_queue_projection)
                if stored_queue_projection == current_queue_projection =>
            {
                stored_queue_projection
            }
            _ => current_queue_projection,
        };
        let result_output_markdown = workspace_record
            .result_output_markdown
            .as_deref()
            .expect("complete planning workspace should include result output");
        let queue_summary = queue_projection.queue_summary();
        let proposal_summary = queue_projection.proposal_summary(MAX_PROPOSAL_SUMMARY_TITLES);
        let prompt_fragment =
            build_prompt_fragment(&directions, &queue_projection, result_output_markdown);

        let queue_idle_prompt_path =
            trimmed_non_empty(directions.queue_idle.prompt_path.as_str()).map(str::to_string);
        let task_authority_signature = normalized_task_authority_signature(&task_authority);
        let queue_head_task_signature = queue_projection
            .next_task
            .as_ref()
            .and_then(|queue_head| {
                task_authority
                    .tasks
                    .iter()
                    .find(|task| task.id.trim() == queue_head.task_id.trim())
            })
            .map(normalized_task_signature);

        Ok(PlanningRuntimeSnapshot {
            workspace_present,
            workspace_status: if queue_projection.next_task.is_some() {
                PlanningRuntimeWorkspaceStatus::ReadyWithTask
            } else {
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            },
            prompt_fragment: Some(prompt_fragment),
            queue_summary: Some(queue_summary),
            proposal_summary,
            queue_idle_policy: directions.queue_idle.policy,
            queue_idle_prompt_path,
            queue_head: queue_projection.next_task.clone(),
            queue_projection: Some(queue_projection),
            task_authority_signature: Some(task_authority_signature),
            queue_head_task_signature,
            failure_reason: None,
            auto_followup_pause_reason: None,
        })
    }
}

fn normalized_task_authority_signature(task_authority: &TaskAuthorityDocument) -> u64 {
    // Task ordering and dependency vector ordering should not make repeat-turn
    // detection think accepted authority changed.  Normalize before hashing.
    let mut normalized_ledger = task_authority.clone();
    normalized_ledger
        .tasks
        .sort_by(|left, right| left.id.cmp(&right.id));
    for task in &mut normalized_ledger.tasks {
        task.depends_on.sort();
        task.blocked_by.sort();
    }

    normalized_json_signature(&normalized_ledger)
}

fn normalized_task_signature(task: &TaskDefinition) -> u64 {
    normalized_json_signature(&task.normalized())
}

fn normalized_json_signature<T>(value: &T) -> u64
where
    T: serde::Serialize,
{
    let json = serde_json::to_string(value)
        .expect("valid planning state should serialize into a signature");
    let mut hasher = DefaultHasher::new();
    json.hash(&mut hasher);
    hasher.finish()
}

fn workspace_record_to_files<'a>(
    workspace_record: &'a PlanningWorkspaceLoadRecord,
    directions: &'a DirectionCatalogDocument,
    task_authority_json: &'a str,
) -> PlanningWorkspaceFiles<'a> {
    // This adapter keeps the validator's existing file-shaped input while
    // substituting DB-backed authority for the removed task-authority file.
    PlanningWorkspaceFiles {
        directions,
        task_authority_json,
        result_output_markdown: workspace_record
            .result_output_markdown
            .as_deref()
            .expect("complete planning workspace should include result output"),
    }
}

fn missing_workspace_paths(workspace_record: &PlanningWorkspaceLoadRecord) -> Vec<&'static str> {
    // Direction and task authority now come from DB snapshots, so only
    // operator-maintained files are reported as missing workspace paths.
    let mut missing_paths = Vec::new();
    if workspace_record.result_output_markdown.is_none() {
        missing_paths.push(RESULT_OUTPUT_FILE_PATH);
    }
    missing_paths
}

#[cfg(test)]
mod tests {
    use super::{missing_workspace_paths, workspace_record_to_files};
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
    use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
    use crate::application::service::planning::shared::prompt_sections::runtime_task_authority_contract_rules;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig,
    };

    #[test]
    fn missing_workspace_paths_only_reports_operator_files() {
        let record = PlanningWorkspaceLoadRecord {
            result_output_markdown: None,
        };

        assert_eq!(
            missing_workspace_paths(&record),
            vec![RESULT_OUTPUT_FILE_PATH]
        );
    }

    #[test]
    fn task_authority_contract_uses_db_authority_source_of_truth() {
        let rules = runtime_task_authority_contract_rules().join("\n");

        assert!(rules.contains("accepted DB authority"));
        assert!(!rules.contains("task-ledger.json"));
        assert!(!rules.contains(".codex-exec-loop/runtime/exports/*"));
    }

    #[test]
    fn workspace_record_combines_db_task_authority_with_operator_files() {
        let record = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("# Result Output Prompt".to_string()),
        };
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General workstream".to_string(),
                summary: "default".to_string(),
                success_criteria: vec!["done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        };

        let files = workspace_record_to_files(&record, &directions, "{\"version\":1,\"tasks\":[]}");

        assert_eq!(files.directions, &directions);
        assert_eq!(files.task_authority_json, "{\"version\":1,\"tasks\":[]}");
        assert_eq!(files.result_output_markdown, "# Result Output Prompt");
    }
}
