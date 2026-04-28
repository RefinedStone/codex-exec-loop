use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PlanningWorkspaceFiles, PriorityQueueProjection,
    PriorityQueueTask, QueueIdlePolicy, TaskAuthorityDocument, TaskDefinition,
};

use crate::application::service::planning::runtime::validation::PlanningValidationService;

const MAX_VISIBLE_QUEUE_TASKS: usize = 5;
const MAX_SKIPPED_QUEUE_TASKS: usize = 3;
const MAX_VISIBLE_PROPOSED_TASKS: usize = 3;
const MAX_PROPOSAL_SUMMARY_TITLES: usize = 2;

#[derive(Clone)]
pub struct PlanningPromptService {
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
            return Ok(PlanningRuntimeSnapshot::invalid(format!(
                "planning files incomplete: missing {}",
                missing_paths.join(", ")
            ))
            .with_workspace_present(workspace_present));
        }

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
                return Ok(PlanningRuntimeSnapshot::invalid(format!(
                    "planning queue build failed: {error}"
                ))
                .with_workspace_present(workspace_present));
            }
        };
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
        let queue_summary = build_queue_summary(&queue_projection);
        let proposal_summary = build_proposal_summary(&queue_projection);
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
    let mut missing_paths = Vec::new();
    if workspace_record.result_output_markdown.is_none() {
        missing_paths.push(RESULT_OUTPUT_FILE_PATH);
    }
    missing_paths
}

fn build_prompt_fragment(
    directions: &DirectionCatalogDocument,
    queue_projection: &PriorityQueueProjection,
    result_output_markdown: &str,
) -> String {
    let mut lines = vec![
        "Planning Context".to_string(),
        "".to_string(),
        "Direction Summary".to_string(),
    ];

    for direction in &directions.directions {
        lines.push(format!(
            "- {} | {} | state={}",
            direction.id.trim(),
            direction.title.trim(),
            direction_state_label(direction.state),
        ));
        lines.push(format!("  summary: {}", direction.summary.trim()));
        lines.push(format!(
            "  success_criteria: {}",
            direction.success_criteria.join(" | ")
        ));
        if !direction.scope_hints.is_empty() {
            lines.push(format!(
                "  scope_hints: {}",
                direction.scope_hints.join(" | ")
            ));
        }
        if let Some(detail_doc_path) = trimmed_non_empty(direction.detail_doc_path.as_str()) {
            lines.push(format!("  detail_doc_path: {detail_doc_path}"));
        }
    }

    lines.push(String::new());
    lines.push("Queue Idle Policy".to_string());
    lines.push(format!(
        "- policy: {}",
        directions.queue_idle.policy.label()
    ));
    if let Some(prompt_path) = trimmed_non_empty(directions.queue_idle.prompt_path.as_str()) {
        lines.push(format!("- prompt_path: {prompt_path}"));
    }

    lines.push(String::new());
    lines.push("Queue Summary".to_string());
    match queue_projection.next_task.as_ref() {
        Some(task) => {
            lines.push(format!(
                "- next_task: rank {} | {} | {} | direction={} | status={} | combined_priority={}",
                task.rank,
                task.task_id.trim(),
                task.task_title.trim(),
                task.direction_id.trim(),
                task.status.label(),
                task.combined_priority,
            ));
            lines.push(format!("  rank_reasons: {}", task.rank_reasons.join(" | ")));
        }
        None => lines.push("- next_task: none".to_string()),
    }

    if queue_projection.active_tasks.is_empty() {
        lines.push("- visible_tasks: none".to_string());
    } else {
        let visible_tasks = queue_projection.visible_tasks(MAX_VISIBLE_QUEUE_TASKS);
        lines.push(format!(
            "- visible_tasks: top {} of {}",
            visible_tasks.len(),
            queue_projection.active_tasks.len()
        ));
        for task in visible_tasks {
            lines.push(format!(
                "  - rank {} | {} | {} | direction={} | status={} | combined_priority={}",
                task.rank,
                task.task_id.trim(),
                task.task_title.trim(),
                task.direction_id.trim(),
                task.status.label(),
                task.combined_priority,
            ));
            lines.push(format!(
                "    rank_reasons: {}",
                task.rank_reasons.join(" | ")
            ));
        }
    }

    if !queue_projection.proposed_tasks.is_empty() {
        let proposed_tasks = queue_projection.visible_proposed_tasks(MAX_VISIBLE_PROPOSED_TASKS);
        lines.push(format!(
            "- proposed_tasks: top {} of {} promotable proposals",
            proposed_tasks.len(),
            queue_projection.proposed_tasks.len()
        ));
        for proposed_task in proposed_tasks {
            lines.push(format!(
                "  - proposal rank {} | {} | {} | direction={} | status={} | combined_priority={}",
                proposed_task.rank,
                proposed_task.task_id.trim(),
                proposed_task.task_title.trim(),
                proposed_task.direction_id.trim(),
                proposed_task.status.label(),
                proposed_task.combined_priority,
            ));
            lines.push(format!(
                "    rank_reasons: {}",
                proposed_task.rank_reasons.join(" | ")
            ));
        }
    }

    if !queue_projection.skipped_tasks.is_empty() {
        let skipped_tasks = queue_projection
            .skipped_tasks
            .iter()
            .take(MAX_SKIPPED_QUEUE_TASKS)
            .collect::<Vec<_>>();
        lines.push(format!(
            "- skipped_tasks: showing {} of {}",
            skipped_tasks.len(),
            queue_projection.skipped_tasks.len()
        ));
        for skipped_task in skipped_tasks {
            lines.push(format!(
                "  - {} | {} | direction={} | status={} | reason={}",
                skipped_task.task_id.trim(),
                skipped_task.task_title.trim(),
                skipped_task.direction_id.trim(),
                skipped_task.status.label(),
                skipped_task.reason.trim(),
            ));
        }
    }

    lines.push(String::new());
    lines.push("Task Authority Mutation Contract".to_string());
    lines.push(format!("- Do not edit `{}`.", RESULT_OUTPUT_FILE_PATH));
    lines.push(
        "- New tasks must attach to an existing `direction_id` and include `direction_relation_note`."
            .to_string(),
    );
    lines.push(
        "- Do not write unrelated tasks that cannot be connected to the existing directions."
            .to_string(),
    );
    lines.push(
        "- Task catalog mutations must go through the runtime task authority flow, then queue validation will refresh prompt state."
            .to_string(),
    );

    lines.push(String::new());
    lines.push("Result Output Prompt".to_string());
    if !result_output_markdown.is_empty() {
        lines.push(result_output_markdown.to_string());
    }
    lines.push(String::new());
    lines.push("Runtime Follow-up Proposal Rules".to_string());
    lines.push(
        "- If your final answer offers concrete follow-up options or variants, create each option through the task authority flow as a separate `proposed` task linked to an existing direction."
            .to_string(),
    );
    lines.push(
        "- Use `proposed` only for direction-linked follow-up candidates that should stay out of normal execution until the user explicitly promotes, prioritizes, queues, or executes them."
            .to_string(),
    );
    lines.push(
        "- If `next_task` is `none` but `proposed_tasks` exist and you are told to keep going from the latest answer, move the actionable worklist into normal queue tasks with priorities, keep the remaining queue intact, execute only the single highest-priority executable task in this turn, and then show the remaining queued or proposed work in the final answer."
            .to_string(),
    );
    lines.push(
        "- When the user later asks to prioritize, queue, or execute earlier proposals, update the relevant proposal tasks instead of inventing duplicate tasks."
            .to_string(),
    );

    lines.join("\n")
}

fn build_queue_summary(queue_projection: &PriorityQueueProjection) -> String {
    match queue_projection.next_task.as_ref() {
        Some(task) => format!(
            "next task: rank {} / {} / {} / priority {}",
            task.rank,
            task.task_id.trim(),
            task.task_title.trim(),
            task.combined_priority,
        ),
        None => "queue idle: no executable planning task".to_string(),
    }
}

fn build_proposal_summary(queue_projection: &PriorityQueueProjection) -> Option<String> {
    if queue_projection.proposed_tasks.is_empty() {
        return None;
    }

    let task_titles = queue_projection
        .proposed_tasks
        .iter()
        .map(|task| task.task_title.trim())
        .filter(|title| !title.is_empty())
        .take(MAX_PROPOSAL_SUMMARY_TITLES)
        .collect::<Vec<_>>();
    let remaining_count = queue_projection
        .proposed_tasks
        .len()
        .saturating_sub(task_titles.len());
    let title_segment = if task_titles.is_empty() {
        String::new()
    } else {
        let mut segment = format!(": {}", task_titles.join(" | "));
        if remaining_count > 0 {
            segment.push_str(&format!(" | +{remaining_count} more"));
        }
        segment
    };

    Some(format!(
        "{} promotable follow-up proposal{} available{}",
        queue_projection.proposed_tasks.len(),
        if queue_projection.proposed_tasks.len() == 1 {
            ""
        } else {
            "s"
        },
        title_segment,
    ))
}

fn trimmed_non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn direction_state_label(state: DirectionState) -> &'static str {
    match state {
        DirectionState::Active => "active",
        DirectionState::Paused => "paused",
        DirectionState::Done => "done",
    }
}

#[cfg(test)]
mod tests {
    use super::{missing_workspace_paths, workspace_record_to_files};
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
    use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
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
