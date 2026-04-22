use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::shared::contract::{
    DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH, canonical_active_planning_file_path,
};
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::domain::planning::{PlanningWorkspaceFiles, TaskDefinition, TaskLedgerDocument};

use crate::application::service::planning::runtime::validation::PlanningValidationService;

#[derive(Clone)]
pub struct PlanningReconciliationService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningExecutionSnapshot {
    pub directions_toml: Option<String>,
    pub task_ledger_json: Option<String>,
    pub task_ledger_schema_json: Option<String>,
    pub result_output_markdown: Option<String>,
    pub queue_snapshot_json: Option<String>,
}

impl PlanningExecutionSnapshot {
    pub fn captures_path(path: &str) -> bool {
        canonical_active_planning_file_path(path).is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueSnapshotAction {
    RebuiltFromAcceptedPlanning,
    RestoredFromExecutionSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProtectedFileRestoration {
    pub relative_path: &'static str,
    pub archived_candidate_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningReconciliationResult {
    pub notices: Vec<String>,
    pub restored_protected_files: Vec<PlanningProtectedFileRestoration>,
    pub rejected_task_ledger: bool,
    pub rejected_archive_path: Option<String>,
    pub queue_snapshot_action: Option<PlanningQueueSnapshotAction>,
    pub repair_request: Option<PlanningRepairRequest>,
    pub auto_followup_block_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRepairRequest {
    pub failure_summary: String,
    pub validation_errors: Vec<String>,
    pub directions_toml: String,
    pub task_ledger_schema_json: String,
    pub accepted_task_ledger_json: String,
    pub rejected_task_ledger_json: Option<String>,
    pub rejected_archive_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRepairPromptHandoff<'a> {
    pub task_id: &'a str,
    pub task_title: &'a str,
    pub updated_at: &'a str,
    pub status_label: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningRepairRetryReason {
    TaskLedgerUnchanged,
    TaskLedgerStillInvalid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PlanningChangeSet {
    directions_changed: bool,
    task_ledger_changed: bool,
    task_ledger_schema_changed: bool,
    result_output_changed: bool,
    queue_snapshot_changed: bool,
}

impl PlanningChangeSet {
    fn from_paths(paths: &[String]) -> Self {
        let mut change_set = Self::default();
        for path in paths {
            match canonical_active_planning_file_path(path) {
                Some(DIRECTIONS_FILE_PATH) => change_set.directions_changed = true,
                Some(TASK_LEDGER_FILE_PATH) => change_set.task_ledger_changed = true,
                Some(TASK_LEDGER_SCHEMA_FILE_PATH) => change_set.task_ledger_schema_changed = true,
                Some(RESULT_OUTPUT_FILE_PATH) => change_set.result_output_changed = true,
                Some(QUEUE_SNAPSHOT_FILE_PATH) => change_set.queue_snapshot_changed = true,
                _ => {}
            }
        }
        change_set
    }

    fn has_relevant_changes(self) -> bool {
        self.directions_changed
            || self.task_ledger_changed
            || self.task_ledger_schema_changed
            || self.result_output_changed
            || self.queue_snapshot_changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReconciledPlanningWorkspaceFiles {
    directions_toml: String,
    task_ledger_schema_json: String,
    result_output_markdown: String,
}

#[derive(Debug, Clone, Copy)]
struct ProtectedFileRestoreRequest<'a> {
    workspace_dir: &'a str,
    turn_id: &'a str,
    relative_path: &'static str,
    current_body: Option<&'a str>,
    execution_snapshot_body: Option<&'a str>,
    changed: bool,
}

impl PlanningReconciliationService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
        }
    }

    pub fn load_execution_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningExecutionSnapshot> {
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_candidate_files(workspace_dir)?;

        Ok(PlanningExecutionSnapshot {
            directions_toml: workspace_record.directions_toml,
            task_ledger_json: workspace_record.task_ledger_json,
            task_ledger_schema_json: workspace_record.task_ledger_schema_json,
            result_output_markdown: workspace_record.result_output_markdown,
            queue_snapshot_json: workspace_record.queue_snapshot_json,
        })
    }

    pub fn reconcile_after_turn(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        let change_set = PlanningChangeSet::from_paths(changed_planning_file_paths);
        if !change_set.has_relevant_changes() {
            return Ok(PlanningReconciliationResult::default());
        }

        let mut result = PlanningReconciliationResult::default();
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;

        let reconciled_workspace = self.restore_protected_workspace_files(
            workspace_dir,
            turn_id,
            &workspace_record,
            execution_snapshot,
            change_set,
            &mut result,
        )?;

        if change_set.task_ledger_changed {
            self.reconcile_task_ledger(
                workspace_dir,
                turn_id,
                &workspace_record,
                execution_snapshot,
                &reconciled_workspace,
                &mut result,
            )?;
        } else if change_set.queue_snapshot_changed {
            self.restore_queue_snapshot(
                workspace_record.queue_snapshot_json.as_deref(),
                execution_snapshot,
                &mut result,
            )?;
        }

        if !change_set.task_ledger_changed
            && (change_set.queue_snapshot_changed || !result.restored_protected_files.is_empty())
        {
            self.planning_workspace_port
                .commit_planning_workspace_files(
                    workspace_dir,
                    &execution_snapshot_to_workspace_record(execution_snapshot),
                )?;
        }

        Ok(result)
    }

    fn restore_protected_workspace_files(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        workspace_record: &PlanningWorkspaceLoadRecord,
        execution_snapshot: &PlanningExecutionSnapshot,
        change_set: PlanningChangeSet,
        result: &mut PlanningReconciliationResult,
    ) -> Result<ReconciledPlanningWorkspaceFiles> {
        Ok(ReconciledPlanningWorkspaceFiles {
            directions_toml: self.restore_protected_file(
                ProtectedFileRestoreRequest {
                    workspace_dir,
                    turn_id,
                    relative_path: DIRECTIONS_FILE_PATH,
                    current_body: workspace_record.directions_toml.as_deref(),
                    execution_snapshot_body: execution_snapshot.directions_toml.as_deref(),
                    changed: change_set.directions_changed,
                },
                result,
            )?,
            task_ledger_schema_json: self.restore_protected_file(
                ProtectedFileRestoreRequest {
                    workspace_dir,
                    turn_id,
                    relative_path: TASK_LEDGER_SCHEMA_FILE_PATH,
                    current_body: workspace_record.task_ledger_schema_json.as_deref(),
                    execution_snapshot_body: execution_snapshot.task_ledger_schema_json.as_deref(),
                    changed: change_set.task_ledger_schema_changed,
                },
                result,
            )?,
            result_output_markdown: self.restore_protected_file(
                ProtectedFileRestoreRequest {
                    workspace_dir,
                    turn_id,
                    relative_path: RESULT_OUTPUT_FILE_PATH,
                    current_body: workspace_record.result_output_markdown.as_deref(),
                    execution_snapshot_body: execution_snapshot.result_output_markdown.as_deref(),
                    changed: change_set.result_output_changed,
                },
                result,
            )?,
        })
    }

    fn restore_protected_file(
        &self,
        request: ProtectedFileRestoreRequest<'_>,
        result: &mut PlanningReconciliationResult,
    ) -> Result<String> {
        if !request.changed {
            return Ok(request.current_body.unwrap_or_default().to_string());
        }

        if request.current_body == request.execution_snapshot_body {
            return Ok(request
                .execution_snapshot_body
                .unwrap_or_default()
                .to_string());
        }

        let archived_candidate_path = self.archive_changed_candidate(
            request.workspace_dir,
            request.turn_id,
            request.relative_path,
            request.current_body,
            request.execution_snapshot_body,
        )?;

        result
            .restored_protected_files
            .push(PlanningProtectedFileRestoration {
                relative_path: request.relative_path,
                archived_candidate_path: archived_candidate_path.clone(),
            });
        result.notices.push(format!(
            "planning reconciliation restored protected {}",
            request.relative_path
        ));
        if let Some(archived_candidate_path) = archived_candidate_path.as_deref() {
            result.notices.push(format!(
                "planning reconciliation archived protected-file candidate at {archived_candidate_path}"
            ));
        }

        Ok(request
            .execution_snapshot_body
            .unwrap_or_default()
            .to_string())
    }

    fn archive_changed_candidate(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        relative_path: &str,
        current_body: Option<&str>,
        execution_snapshot_body: Option<&str>,
    ) -> Result<Option<String>> {
        let Some(current_body) = current_body else {
            return Ok(None);
        };
        if Some(current_body) == execution_snapshot_body {
            return Ok(None);
        }

        self.planning_workspace_port
            .archive_rejected_planning_file(workspace_dir, turn_id, relative_path, current_body)
            .map(Some)
    }

    fn reconcile_task_ledger(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        workspace_record: &PlanningWorkspaceLoadRecord,
        execution_snapshot: &PlanningExecutionSnapshot,
        reconciled_workspace: &ReconciledPlanningWorkspaceFiles,
        result: &mut PlanningReconciliationResult,
    ) -> Result<()> {
        let task_ledger_candidate = workspace_record
            .task_ledger_json
            .as_deref()
            .unwrap_or_default();
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: &reconciled_workspace.directions_toml,
                    task_ledger_json: task_ledger_candidate,
                    task_ledger_schema_json: &reconciled_workspace.task_ledger_schema_json,
                    result_output_markdown: &reconciled_workspace.result_output_markdown,
                });

        if validation_result.is_valid() {
            let directions = validation_result.directions.as_ref().ok_or_else(|| {
                anyhow!("planning validation reported success without parsed directions.toml")
            })?;
            let task_ledger = validation_result.task_ledger.as_ref().ok_or_else(|| {
                anyhow!("planning validation reported success without parsed task-ledger.json")
            })?;
            let queue_snapshot = self
                .priority_queue_service
                .build_snapshot(directions, task_ledger)
                .map_err(|error| {
                    anyhow!("planning validation passed but queue build failed: {error}")
                })?;
            let queue_snapshot_json = serde_json::to_string_pretty(&queue_snapshot)
                .context("failed to serialize queue snapshot")?;
            let mut committed_record = execution_snapshot_to_workspace_record(execution_snapshot);
            committed_record.task_ledger_json = workspace_record.task_ledger_json.clone();
            committed_record.queue_snapshot_json = Some(queue_snapshot_json);
            self.planning_workspace_port
                .commit_planning_workspace_files(workspace_dir, &committed_record)?;
            result.queue_snapshot_action =
                Some(PlanningQueueSnapshotAction::RebuiltFromAcceptedPlanning);
            result.notices.push(
                "planning reconciliation accepted task-ledger.json and rebuilt queue.snapshot.json"
                    .to_string(),
            );
            return Ok(());
        }

        if let Some(task_ledger_json) = workspace_record.task_ledger_json.as_deref() {
            let archive_path = self
                .planning_workspace_port
                .archive_rejected_planning_file(
                    workspace_dir,
                    turn_id,
                    TASK_LEDGER_FILE_PATH,
                    task_ledger_json,
                )?;
            result.rejected_archive_path = Some(archive_path);
        }

        self.restore_queue_snapshot(
            workspace_record.queue_snapshot_json.as_deref(),
            execution_snapshot,
            result,
        )?;
        self.planning_workspace_port
            .commit_planning_workspace_files(
                workspace_dir,
                &execution_snapshot_to_workspace_record(execution_snapshot),
            )?;
        let validation_errors = validation_error_summaries(&validation_result);
        result.rejected_task_ledger = true;
        result.repair_request = Some(PlanningRepairRequest {
            failure_summary: validation_errors
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown validation failure".to_string()),
            validation_errors,
            directions_toml: reconciled_workspace.directions_toml.clone(),
            task_ledger_schema_json: reconciled_workspace.task_ledger_schema_json.clone(),
            accepted_task_ledger_json: execution_snapshot
                .task_ledger_json
                .clone()
                .unwrap_or_default(),
            rejected_task_ledger_json: workspace_record.task_ledger_json.clone(),
            rejected_archive_path: result.rejected_archive_path.clone(),
        });
        result.notices.push(format!(
            "planning reconciliation rejected task-ledger.json and restored the last accepted ledger ({})",
            first_validation_error_summary(&validation_result)
        ));
        if let Some(rejected_archive_path) = result.rejected_archive_path.as_deref() {
            result.notices.push(format!(
                "planning reconciliation archived rejected task-ledger at {rejected_archive_path}"
            ));
        }

        Ok(())
    }

    fn restore_queue_snapshot(
        &self,
        current_queue_snapshot_json: Option<&str>,
        execution_snapshot: &PlanningExecutionSnapshot,
        result: &mut PlanningReconciliationResult,
    ) -> Result<()> {
        if current_queue_snapshot_json == execution_snapshot.queue_snapshot_json.as_deref() {
            return Ok(());
        }

        result.queue_snapshot_action =
            Some(PlanningQueueSnapshotAction::RestoredFromExecutionSnapshot);
        result.notices.push(
            "planning reconciliation restored queue.snapshot.json to the last accepted state"
                .to_string(),
        );
        Ok(())
    }
}

fn execution_snapshot_to_workspace_record(
    execution_snapshot: &PlanningExecutionSnapshot,
) -> PlanningWorkspaceLoadRecord {
    PlanningWorkspaceLoadRecord {
        directions_toml: execution_snapshot.directions_toml.clone(),
        task_ledger_json: execution_snapshot.task_ledger_json.clone(),
        task_ledger_schema_json: execution_snapshot.task_ledger_schema_json.clone(),
        queue_snapshot_json: execution_snapshot.queue_snapshot_json.clone(),
        result_output_markdown: execution_snapshot.result_output_markdown.clone(),
    }
}

fn first_validation_error_summary(
    validation_result: &crate::domain::planning::PlanningValidationResult,
) -> String {
    validation_error_summaries(validation_result)
        .into_iter()
        .next()
        .unwrap_or_else(|| "unknown validation failure".to_string())
}

fn validation_error_summaries(
    validation_result: &crate::domain::planning::PlanningValidationResult,
) -> Vec<String> {
    validation_result
        .report
        .errors()
        .into_iter()
        .map(|issue| issue.message.clone())
        .collect()
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PlanningRepairPromptContext {
    accepted_heading: Option<String>,
    accepted_excerpt: Option<String>,
    rejected_heading: Option<String>,
    rejected_excerpt: Option<String>,
}

pub fn build_planning_repair_prompt(
    request: &PlanningRepairRequest,
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
    attempt_number: usize,
    max_attempts: usize,
    retry_reason: Option<PlanningRepairRetryReason>,
) -> String {
    let mut lines = vec![
        "대리인입니다.".to_string(),
        format!("planning repair {attempt_number}/{max_attempts} 입니다."),
        "이전 턴에서 `task-ledger.json` 후보가 validation을 통과하지 못했습니다.".to_string(),
        "이번 턴에서는 `.codex-exec-loop/planning/task-ledger.json` 하나만 고치세요.".to_string(),
        "- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, `queue.snapshot.json` 은 수정하지 마세요.".to_string(),
        "- 현재 작업공간에는 마지막 accepted `task-ledger.json` 이 이미 복원돼 있습니다."
            .to_string(),
        "- 아래 validation 오류를 모두 해결하는 유효한 JSON으로 다시 작성하세요.".to_string(),
        "- 기존 direction frame 밖의 관련 없는 새 작업은 추가하지 마세요.".to_string(),
    ];

    if let Some(retry_reason) = retry_reason {
        lines.push(format!("- 추가 지시: {}", retry_reason.instruction()));
    }

    if let Some(previous_handoff) = previous_handoff {
        lines.push(String::new());
        lines.push("직전에 main session으로 넘긴 task:".to_string());
        lines.push(format!("- task_id: {}", previous_handoff.task_id));
        lines.push(format!("- title: {}", previous_handoff.task_title));
        lines.push(format!("- updated_at: {}", previous_handoff.updated_at));
        lines.push(format!("- status: {}", previous_handoff.status_label));
        lines.push(
            "- 같은 task를 유지하려면 그 task 자체가 바뀌었다는 근거가 ledger에 있어야 합니다."
                .to_string(),
        );
    }

    lines.push(String::new());
    lines.push(format!("Failure summary: {}", request.failure_summary));
    lines.push(String::new());
    lines.push("Validation errors:".to_string());
    for error in &request.validation_errors {
        lines.push(format!("- {error}"));
    }
    if let Some(rejected_archive_path) = request.rejected_archive_path.as_deref() {
        lines.push(format!("- rejected archive: {rejected_archive_path}"));
    }

    lines.push(String::new());
    lines.push("Accepted directions (`directions.toml`):".to_string());
    lines.push(prompt_code_block(
        "toml",
        truncate_prompt_section(&request.directions_toml, 4_000).as_str(),
    ));

    lines.push(String::new());
    lines.push("Allowed schema (`task-ledger.schema.json`):".to_string());
    lines.push(prompt_code_block(
        "json",
        truncate_prompt_section(&request.task_ledger_schema_json, 4_000).as_str(),
    ));

    let prompt_context = build_planning_repair_prompt_context(request, previous_handoff);
    let accepted_excerpt = prompt_context
        .accepted_excerpt
        .clone()
        .unwrap_or_else(|| truncate_prompt_section(&request.accepted_task_ledger_json, 4_000));

    lines.push(String::new());
    lines.push(
        prompt_context.accepted_heading.unwrap_or_else(|| {
            "Current accepted `task-ledger.json` (restored on disk):".to_string()
        }),
    );
    lines.push(prompt_code_block("json", &accepted_excerpt));

    if let Some(rejected_task_ledger_json) = request
        .rejected_task_ledger_json
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let rejected_excerpt = prompt_context
            .rejected_excerpt
            .clone()
            .unwrap_or_else(|| truncate_prompt_section(rejected_task_ledger_json, 4_000));
        lines.push(String::new());
        lines.push(
            prompt_context
                .rejected_heading
                .unwrap_or_else(|| "Rejected candidate excerpt:".to_string()),
        );
        lines.push(prompt_code_block("json", &rejected_excerpt));
    }

    lines.push(String::new());
    lines.push(
        "수정이 끝나면 무엇을 고쳤는지 짧게 요약하세요. 더 이상 고칠 것이 없어도 `DONE` 만 단독으로 출력하지 말고 이유를 설명하세요."
            .to_string(),
    );

    lines.join("\n")
}

fn build_planning_repair_prompt_context(
    request: &PlanningRepairRequest,
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
) -> PlanningRepairPromptContext {
    let accepted_task_ledger = parse_task_ledger_document(&request.accepted_task_ledger_json);
    let rejected_task_ledger = request
        .rejected_task_ledger_json
        .as_deref()
        .and_then(parse_task_ledger_document);
    let Some(accepted_task_ledger) = accepted_task_ledger.as_ref() else {
        return PlanningRepairPromptContext::default();
    };

    let focus_ids = collect_focus_task_ids(
        accepted_task_ledger,
        rejected_task_ledger.as_ref(),
        &request.validation_errors,
        previous_handoff,
    );
    if focus_ids.is_empty() {
        return PlanningRepairPromptContext::default();
    }

    PlanningRepairPromptContext {
        accepted_heading: Some(
            "Current accepted `task-ledger.json` focus (current handoff + validation context):"
                .to_string(),
        ),
        accepted_excerpt: serialize_focused_task_ledger_excerpt(accepted_task_ledger, &focus_ids),
        rejected_heading: rejected_task_ledger
            .as_ref()
            .map(|_| "Rejected candidate focus (changed tasks + validation context):".to_string()),
        rejected_excerpt: rejected_task_ledger
            .as_ref()
            .and_then(|task_ledger| serialize_focused_task_ledger_excerpt(task_ledger, &focus_ids)),
    }
}

fn parse_task_ledger_document(body: &str) -> Option<TaskLedgerDocument> {
    serde_json::from_str(body).ok()
}

fn collect_focus_task_ids(
    accepted_task_ledger: &TaskLedgerDocument,
    rejected_task_ledger: Option<&TaskLedgerDocument>,
    validation_errors: &[String],
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
) -> BTreeSet<String> {
    let mut focus_ids = BTreeSet::new();
    if let Some(previous_handoff) = previous_handoff {
        let task_id = previous_handoff.task_id.trim();
        if !task_id.is_empty() {
            focus_ids.insert(task_id.to_string());
        }
    }

    let mut known_task_ids = accepted_task_ledger
        .tasks
        .iter()
        .map(|task| task.id.trim().to_string())
        .collect::<BTreeSet<_>>();
    if let Some(rejected_task_ledger) = rejected_task_ledger {
        known_task_ids.extend(
            rejected_task_ledger
                .tasks
                .iter()
                .map(|task| task.id.trim().to_string()),
        );
        focus_ids.extend(changed_task_ids(accepted_task_ledger, rejected_task_ledger));
    }

    for validation_error in validation_errors {
        for task_id in &known_task_ids {
            if validation_error.contains(task_id) {
                focus_ids.insert(task_id.clone());
            }
        }
    }

    expand_related_task_ids(&mut focus_ids, accepted_task_ledger);
    if let Some(rejected_task_ledger) = rejected_task_ledger {
        expand_related_task_ids(&mut focus_ids, rejected_task_ledger);
    }

    focus_ids
}

fn changed_task_ids(
    accepted_task_ledger: &TaskLedgerDocument,
    rejected_task_ledger: &TaskLedgerDocument,
) -> BTreeSet<String> {
    let accepted_task_map = accepted_task_ledger
        .tasks
        .iter()
        .map(|task| (task.id.trim(), task))
        .collect::<HashMap<_, _>>();
    let rejected_task_map = rejected_task_ledger
        .tasks
        .iter()
        .map(|task| (task.id.trim(), task))
        .collect::<HashMap<_, _>>();
    let all_task_ids = accepted_task_map
        .keys()
        .copied()
        .chain(rejected_task_map.keys().copied())
        .collect::<BTreeSet<_>>();
    let mut changed_task_ids = BTreeSet::new();

    for task_id in all_task_ids {
        match (
            accepted_task_map.get(task_id),
            rejected_task_map.get(task_id),
        ) {
            (Some(accepted_task), Some(rejected_task))
                if normalized_task_definition(accepted_task)
                    != normalized_task_definition(rejected_task) =>
            {
                changed_task_ids.insert(task_id.to_string());
            }
            (None, Some(_)) | (Some(_), None) => {
                changed_task_ids.insert(task_id.to_string());
            }
            _ => {}
        }
    }

    changed_task_ids
}

fn normalized_task_definition(task: &TaskDefinition) -> TaskDefinition {
    let mut normalized_task = task.clone();
    normalized_task.depends_on.sort();
    normalized_task.blocked_by.sort();
    normalized_task
}

fn expand_related_task_ids(focus_ids: &mut BTreeSet<String>, task_ledger: &TaskLedgerDocument) {
    let seed_ids = focus_ids.clone();
    for task in &task_ledger.tasks {
        let task_id = task.id.trim();
        let directly_related = seed_ids.contains(task_id)
            || task
                .depends_on
                .iter()
                .any(|dependency_id| seed_ids.contains(dependency_id.trim()))
            || task
                .blocked_by
                .iter()
                .any(|blocker_id| seed_ids.contains(blocker_id.trim()));
        if !directly_related {
            continue;
        }

        focus_ids.insert(task_id.to_string());
        for dependency_id in &task.depends_on {
            let dependency_id = dependency_id.trim();
            if !dependency_id.is_empty() {
                focus_ids.insert(dependency_id.to_string());
            }
        }
        for blocker_id in &task.blocked_by {
            let blocker_id = blocker_id.trim();
            if !blocker_id.is_empty() {
                focus_ids.insert(blocker_id.to_string());
            }
        }
    }
}

fn serialize_focused_task_ledger_excerpt(
    task_ledger: &TaskLedgerDocument,
    focus_ids: &BTreeSet<String>,
) -> Option<String> {
    let focused_tasks = task_ledger
        .tasks
        .iter()
        .filter(|task| focus_ids.contains(task.id.trim()))
        .cloned()
        .collect::<Vec<_>>();
    if focused_tasks.is_empty() {
        return None;
    }

    serde_json::to_string_pretty(&TaskLedgerDocument {
        version: task_ledger.version,
        tasks: focused_tasks,
    })
    .ok()
}

fn prompt_code_block(language: &str, body: &str) -> String {
    format!("```{language}\n{body}\n```")
}

fn truncate_prompt_section(body: &str, max_chars: usize) -> String {
    let body = body.trim();
    if body.chars().count() <= max_chars {
        return body.to_string();
    }

    let truncated = body.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n... [truncated]")
}

impl PlanningRepairRetryReason {
    fn instruction(self) -> &'static str {
        match self {
            Self::TaskLedgerUnchanged => {
                "직전 repair 시도에서 `task-ledger.json` 이 바뀌지 않았습니다. 이번 턴에서는 그 파일을 반드시 다시 작성하세요."
            }
            Self::TaskLedgerStillInvalid => {
                "직전 repair 시도에서 `task-ledger.json` 을 수정했지만 여전히 유효하지 않습니다. 이번 턴에서는 validation 오류를 모두 해결하도록 그 파일을 다시 작성하세요."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{
        PlanningExecutionSnapshot, PlanningQueueSnapshotAction, PlanningReconciliationService,
        PlanningRepairPromptHandoff, PlanningRepairRetryReason, build_planning_repair_prompt,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning::authoring::bootstrap::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::application::service::planning::shared::contract::{
        DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
        TASK_LEDGER_SCHEMA_FILE_PATH,
    };
    use crate::application::service::priority_queue_service::PriorityQueueService;

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    fn write_bootstrap_workspace(workspace_dir: &str) -> PlanningExecutionSnapshot {
        let planning_dir = Path::new(workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let directions =
            toml::from_str(&artifacts.directions_toml).expect("bootstrap directions should parse");
        let task_ledger = serde_json::from_str(&artifacts.task_ledger_json)
            .expect("bootstrap task ledger should parse");
        let queue_snapshot = PriorityQueueService::new()
            .build_snapshot(&directions, &task_ledger)
            .expect("bootstrap queue snapshot should build");
        let queue_snapshot_json =
            serde_json::to_string_pretty(&queue_snapshot).expect("queue snapshot should serialize");
        fs::write(
            planning_dir.join("directions.toml"),
            &artifacts.directions_toml,
        )
        .expect("directions should write");
        fs::write(
            planning_dir.join("task-ledger.json"),
            &artifacts.task_ledger_json,
        )
        .expect("task ledger should write");
        fs::write(
            planning_dir.join("task-ledger.schema.json"),
            &artifacts.task_ledger_schema_json,
        )
        .expect("schema should write");
        fs::write(
            planning_dir.join("queue.snapshot.json"),
            &queue_snapshot_json,
        )
        .expect("queue snapshot should write");
        fs::write(
            planning_dir.join("result-output.md"),
            &artifacts.result_output_markdown,
        )
        .expect("result output should write");

        PlanningExecutionSnapshot {
            directions_toml: Some(artifacts.directions_toml),
            task_ledger_json: Some(artifacts.task_ledger_json),
            task_ledger_schema_json: Some(artifacts.task_ledger_schema_json),
            result_output_markdown: Some(artifacts.result_output_markdown),
            queue_snapshot_json: Some(queue_snapshot_json),
        }
    }

    fn service() -> PlanningReconciliationService {
        PlanningReconciliationService::new(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
    }

    use std::sync::Arc;

    #[test]
    fn valid_task_ledger_change_rebuilds_queue_snapshot() {
        let workspace_dir = create_temp_workspace("planning-reconcile-valid");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let valid_task_ledger = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": [
                {
                    "id": "task-1",
                    "direction_id": "example-direction",
                    "direction_relation_note": "implements the active example direction",
                    "title": "Do the thing",
                    "description": "Implement the next queued step.",
                    "status": "ready",
                    "base_priority": 10,
                    "dynamic_priority_delta": 0,
                    "priority_reason": "",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": "turn-1",
                    "updated_at": "2026-04-09T10:00:00Z"
                }
            ]
        }))
        .expect("valid task ledger should serialize");
        fs::write(
            Path::new(&workspace_dir).join(".codex-exec-loop/planning/task-ledger.json"),
            valid_task_ledger,
        )
        .expect("task ledger candidate should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-1",
                &[TASK_LEDGER_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let queue_snapshot = fs::read_to_string(
            Path::new(&workspace_dir).join(".codex-exec-loop/planning/queue.snapshot.json"),
        )
        .expect("queue snapshot should exist");

        assert_eq!(
            result.queue_snapshot_action,
            Some(PlanningQueueSnapshotAction::RebuiltFromAcceptedPlanning)
        );
        assert!(!result.rejected_task_ledger);
        assert!(queue_snapshot.contains("\"task_id\": \"task-1\""));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn invalid_task_ledger_change_is_archived_and_restored() {
        let workspace_dir = create_temp_workspace("planning-reconcile-invalid");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(
            planning_dir.join("queue.snapshot.json"),
            "{\"next_task\":\"broken\"}",
        )
        .expect("mutated queue snapshot should write");
        fs::write(planning_dir.join("task-ledger.json"), "{ invalid json")
            .expect("invalid task ledger should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-2",
                &[
                    TASK_LEDGER_FILE_PATH.to_string(),
                    QUEUE_SNAPSHOT_FILE_PATH.to_string(),
                ],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_task_ledger = fs::read_to_string(planning_dir.join("task-ledger.json"))
            .expect("restored task ledger should be readable");
        let restored_queue_snapshot = fs::read_to_string(planning_dir.join("queue.snapshot.json"))
            .expect("restored queue snapshot should be readable");

        assert!(result.rejected_task_ledger);
        assert!(result.rejected_archive_path.is_some());
        assert!(result.repair_request.is_some());
        assert_eq!(
            result.queue_snapshot_action,
            Some(PlanningQueueSnapshotAction::RestoredFromExecutionSnapshot)
        );
        assert_eq!(
            restored_task_ledger,
            execution_snapshot
                .task_ledger_json
                .expect("execution snapshot should keep the accepted task ledger")
        );
        assert_eq!(
            restored_queue_snapshot,
            execution_snapshot
                .queue_snapshot_json
                .expect("execution snapshot should keep the accepted queue snapshot")
        );
        assert!(
            Path::new(
                result
                    .rejected_archive_path
                    .as_deref()
                    .expect("archive path should be present")
            )
            .exists()
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn repair_prompt_includes_validation_errors_and_rejected_excerpt() {
        let prompt = build_planning_repair_prompt(
            &super::PlanningRepairRequest {
                failure_summary: "failed to parse task-ledger.json: expected value".to_string(),
                validation_errors: vec![
                    "failed to parse task-ledger.json: expected value".to_string(),
                    "task-ledger.schema.json must not be blank".to_string(),
                ],
                directions_toml: "version = 1".to_string(),
                task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
                accepted_task_ledger_json: "{\"version\":1,\"tasks\":[]}".to_string(),
                rejected_task_ledger_json: Some("{ invalid json".to_string()),
                rejected_archive_path: Some(
                    "/tmp/workspace/.codex-exec-loop/planning/rejected/turn-1/task-ledger.json"
                        .to_string(),
                ),
            },
            None,
            1,
            2,
            Some(PlanningRepairRetryReason::TaskLedgerStillInvalid),
        );

        assert!(prompt.contains("planning repair 1/2"));
        assert!(prompt.contains("failed to parse task-ledger.json"));
        assert!(prompt.contains("rejected archive"));
        assert!(prompt.contains("Rejected candidate excerpt"));
        assert!(prompt.contains("수정했지만 여전히 유효하지 않습니다"));
    }

    #[test]
    fn repair_prompt_surfaces_previous_handoff_and_changed_task_context_from_large_ledger() {
        let filler_tasks = (0..40)
            .map(|index| {
                json!({
                    "id": format!("filler-task-{index:02}"),
                    "direction_id": "example-direction",
                    "direction_relation_note": format!("Filler relation note {index}"),
                    "title": format!("Filler task {index}"),
                    "description": "This filler task makes the accepted ledger large enough that naive truncation would hide the real repair targets.".repeat(3),
                    "status": "done",
                    "base_priority": 10,
                    "dynamic_priority_delta": 0,
                    "priority_reason": "",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T00:00:00Z"
                })
            })
            .collect::<Vec<_>>();
        let accepted_task_ledger_json = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": filler_tasks.iter().cloned().chain([
                json!({
                    "id": "context-first-bridge-adapter-attachment-event-reuse",
                    "direction_id": "context-first-architecture-and-doc-coherence",
                    "direction_relation_note": "Current queue head before repair.",
                    "title": "Reuse attachment event and profiles across remaining bridge adapters",
                    "description": "Carry the same attachment truth through remaining bridge adapters.",
                    "status": "ready",
                    "base_priority": 87,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Current top executable task.",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:30:00Z"
                }),
                json!({
                    "id": "terminal-bridge-local-spike-readiness-gate",
                    "direction_id": "terminal-agent-bridge-research-and-capability-boundary",
                    "direction_relation_note": "Immediate gate before implementation.",
                    "title": "Gate tmux local-attach spike on capability audit and evidence",
                    "description": "Hold the local spike until evidence exists.",
                    "status": "blocked",
                    "base_priority": 89,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Research gate remains closed.",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:50:00Z"
                })
            ]).collect::<Vec<_>>()
        }))
        .expect("accepted task ledger should serialize");
        let rejected_task_ledger_json = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": filler_tasks.into_iter().chain([
                json!({
                    "id": "context-first-bridge-adapter-attachment-event-reuse",
                    "direction_id": "context-first-architecture-and-doc-coherence",
                    "direction_relation_note": "Repair candidate incorrectly left the old queue head untouched.",
                    "title": "Reuse attachment event and profiles across remaining bridge adapters",
                    "description": "Carry the same attachment truth through remaining bridge adapters.",
                    "status": "ready",
                    "base_priority": 87,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Current top executable task.",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:30:00Z"
                }),
                json!({
                    "id": "terminal-bridge-local-spike-readiness-gate",
                    "direction_id": "terminal-agent-bridge-research-and-capability-boundary",
                    "direction_relation_note": "Immediate gate before implementation.",
                    "title": "Gate tmux local-attach spike on capability audit and evidence",
                    "description": "Hold the local spike until evidence exists.",
                    "status": "blocked",
                    "base_priority": 89,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Research gate remains closed.",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:50:00Z"
                }),
                json!({
                    "id": "terminal-bridge-primary-implementation-slice",
                    "direction_id": "terminal-agent-bridge-research-and-capability-boundary",
                    "direction_relation_note": "First real implementation slice.",
                    "title": "Implement the first real terminal bridge slice",
                    "description": "Start the first real implementation slice.",
                    "status": "blocked",
                    "base_priority": 90,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Implementation waits for readiness gate.",
                    "depends_on": ["terminal-bridge-local-spike-readiness-gate"],
                    "blocked_by": ["terminal-bridge-local-spike-readiness-gate"],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:50:00Z"
                })
            ]).collect::<Vec<_>>()
        }))
        .expect("rejected task ledger should serialize");
        let prompt = build_planning_repair_prompt(
            &super::PlanningRepairRequest {
                failure_summary: "task terminal-bridge-primary-implementation-slice cannot list terminal-bridge-local-spike-readiness-gate in both depends_on and blocked_by".to_string(),
                validation_errors: vec![
                    "task terminal-bridge-primary-implementation-slice cannot list terminal-bridge-local-spike-readiness-gate in both depends_on and blocked_by".to_string(),
                ],
                directions_toml: "version = 1".to_string(),
                task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
                accepted_task_ledger_json,
                rejected_task_ledger_json: Some(rejected_task_ledger_json),
                rejected_archive_path: Some(
                    "/tmp/workspace/.codex-exec-loop/planning/rejected/turn-1/task-ledger.json"
                        .to_string(),
                ),
            },
            Some(PlanningRepairPromptHandoff {
                task_id: "context-first-bridge-adapter-attachment-event-reuse",
                task_title: "Reuse attachment event and profiles across remaining bridge adapters",
                updated_at: "2026-04-22T23:30:00Z",
                status_label: "ready",
            }),
            1,
            2,
            Some(PlanningRepairRetryReason::TaskLedgerStillInvalid),
        );

        assert!(prompt.contains("직전에 main session으로 넘긴 task:"));
        assert!(prompt.contains("Current accepted `task-ledger.json` focus"));
        assert!(prompt.contains("Rejected candidate focus"));
        assert!(prompt.contains("context-first-bridge-adapter-attachment-event-reuse"));
        assert!(prompt.contains("terminal-bridge-primary-implementation-slice"));
        assert!(prompt.contains("terminal-bridge-local-spike-readiness-gate"));
    }

    #[test]
    fn changed_directions_are_restored_from_execution_snapshot() {
        let workspace_dir = create_temp_workspace("planning-reconcile-directions");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(
            planning_dir.join("directions.toml"),
            "version = 1\n[[directions]]\nid = \"mutated\"\ntitle = \"Mutated\"\nsummary = \"mutated\"\nsuccess_criteria = [\"mutated\"]\nstate = \"active\"\n",
        )
            .expect("mutated directions should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-3",
                &[DIRECTIONS_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_directions = fs::read_to_string(planning_dir.join("directions.toml"))
            .expect("restored directions should be readable");

        assert!(!result.rejected_task_ledger);
        assert_eq!(
            restored_directions,
            execution_snapshot
                .directions_toml
                .expect("execution snapshot should keep the accepted directions")
        );
        assert_eq!(result.restored_protected_files.len(), 1);
        assert_eq!(
            result.restored_protected_files[0].relative_path,
            DIRECTIONS_FILE_PATH
        );
        assert!(
            result.restored_protected_files[0]
                .archived_candidate_path
                .as_deref()
                .is_some()
        );
        assert_eq!(result.queue_snapshot_action, None);

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn task_ledger_acceptance_uses_restored_schema_baseline() {
        let workspace_dir = create_temp_workspace("planning-reconcile-schema-restore");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        let valid_task_ledger = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": [
                {
                    "id": "task-restore-schema",
                    "direction_id": "example-direction",
                    "direction_relation_note": "implements the active example direction",
                    "title": "Do the thing",
                    "description": "Implement the next queued step.",
                    "status": "ready",
                    "base_priority": 10,
                    "dynamic_priority_delta": 0,
                    "priority_reason": "",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": "turn-restore-schema",
                    "updated_at": "2026-04-09T10:00:00Z"
                }
            ]
        }))
        .expect("valid task ledger should serialize");
        fs::write(planning_dir.join("task-ledger.schema.json"), "")
            .expect("mutated schema should write");
        fs::write(planning_dir.join("task-ledger.json"), valid_task_ledger)
            .expect("task ledger candidate should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-schema-restore",
                &[
                    TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
                    TASK_LEDGER_FILE_PATH.to_string(),
                ],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_schema = fs::read_to_string(planning_dir.join("task-ledger.schema.json"))
            .expect("restored schema should read");
        let queue_snapshot = fs::read_to_string(planning_dir.join("queue.snapshot.json"))
            .expect("rebuilt queue snapshot should read");

        assert_eq!(
            restored_schema,
            execution_snapshot
                .task_ledger_schema_json
                .expect("execution snapshot should keep the accepted task-ledger schema")
        );
        assert_eq!(
            result.queue_snapshot_action,
            Some(PlanningQueueSnapshotAction::RebuiltFromAcceptedPlanning)
        );
        assert!(!result.rejected_task_ledger);
        assert!(queue_snapshot.contains("\"task_id\": \"task-restore-schema\""));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn queue_snapshot_change_without_task_ledger_change_is_restored() {
        let workspace_dir = create_temp_workspace("planning-reconcile-queue-only");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(
            planning_dir.join("queue.snapshot.json"),
            "{\"next_task\":\"stale\"}",
        )
        .expect("mutated queue snapshot should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-queue-only",
                &[QUEUE_SNAPSHOT_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_queue_snapshot = fs::read_to_string(planning_dir.join("queue.snapshot.json"))
            .expect("restored queue snapshot should read");

        assert_eq!(
            result.queue_snapshot_action,
            Some(PlanningQueueSnapshotAction::RestoredFromExecutionSnapshot)
        );
        assert_eq!(
            restored_queue_snapshot,
            execution_snapshot
                .queue_snapshot_json
                .expect("execution snapshot should keep the accepted queue snapshot")
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn absolute_queue_snapshot_path_is_canonicalized_for_change_detection() {
        let workspace_dir = create_temp_workspace("planning-reconcile-absolute-queue");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(
            planning_dir.join("queue.snapshot.json"),
            "{\"next_task\":\"stale\"}",
        )
        .expect("mutated queue snapshot should write");

        let absolute_queue_snapshot_path = planning_dir
            .join("queue.snapshot.json")
            .display()
            .to_string();
        assert!(PlanningExecutionSnapshot::captures_path(
            absolute_queue_snapshot_path.as_str()
        ));

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-absolute-queue",
                &[absolute_queue_snapshot_path],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_queue_snapshot = fs::read_to_string(planning_dir.join("queue.snapshot.json"))
            .expect("restored queue snapshot should read");

        assert_eq!(
            result.queue_snapshot_action,
            Some(PlanningQueueSnapshotAction::RestoredFromExecutionSnapshot)
        );
        assert_eq!(
            restored_queue_snapshot,
            execution_snapshot
                .queue_snapshot_json
                .expect("execution snapshot should keep the accepted queue snapshot")
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
