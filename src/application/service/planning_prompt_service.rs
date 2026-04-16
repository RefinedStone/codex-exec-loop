use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning_contract::{
    DIRECTIONS_FILE_PATH, PLAN_OFF_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
    TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PlanningWorkspaceFiles, PriorityQueueSnapshot,
    PriorityQueueTask, QueueIdlePolicy,
};
use crate::domain::text::compact_whitespace_detail;

use super::planning_validation_service::PlanningValidationService;
use super::priority_queue_service::PriorityQueueService;

const MAX_VISIBLE_QUEUE_TASKS: usize = 5;
const MAX_SKIPPED_QUEUE_TASKS: usize = 3;
const MAX_VISIBLE_PROPOSED_TASKS: usize = 3;
const PREVIEW_DETAIL_LIMIT: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueFramingDetails {
    now_detail: String,
    next_detail: String,
    proposed_detail: String,
    blocked_detail: String,
}

#[derive(Clone)]
pub struct PlanningPromptService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
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
    plan_enabled: bool,
    workspace_status: PlanningRuntimeWorkspaceStatus,
    prompt_fragment: Option<String>,
    queue_summary: Option<String>,
    proposal_summary: Option<String>,
    queue_idle_policy: QueueIdlePolicy,
    queue_idle_prompt_path: Option<String>,
    queue_head: Option<PriorityQueueTask>,
    queue_snapshot: Option<PriorityQueueSnapshot>,
    failure_reason: Option<String>,
    auto_followup_pause_reason: Option<String>,
}

impl PlanningRuntimeSnapshot {
    pub fn uninitialized() -> Self {
        Self {
            workspace_present: false,
            plan_enabled: false,
            workspace_status: PlanningRuntimeWorkspaceStatus::Uninitialized,
            prompt_fragment: None,
            queue_summary: None,
            proposal_summary: None,
            queue_idle_policy: QueueIdlePolicy::Stop,
            queue_idle_prompt_path: None,
            queue_head: None,
            queue_snapshot: None,
            failure_reason: None,
            auto_followup_pause_reason: None,
        }
    }

    pub fn invalid(reason: impl Into<String>) -> Self {
        Self {
            workspace_present: true,
            plan_enabled: true,
            workspace_status: PlanningRuntimeWorkspaceStatus::Invalid,
            prompt_fragment: None,
            queue_summary: None,
            proposal_summary: None,
            queue_idle_policy: QueueIdlePolicy::Stop,
            queue_idle_prompt_path: None,
            queue_head: None,
            queue_snapshot: None,
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
            plan_enabled: true,
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
            queue_snapshot: None,
            failure_reason: None,
            auto_followup_pause_reason: None,
        }
    }

    pub fn ready_with_queue_snapshot(
        prompt_fragment: String,
        queue_summary: String,
        proposal_summary: Option<String>,
        queue_head: Option<PriorityQueueTask>,
        queue_snapshot: PriorityQueueSnapshot,
    ) -> Self {
        Self {
            workspace_present: true,
            plan_enabled: true,
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
            queue_snapshot: Some(queue_snapshot),
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

    pub fn with_plan_enabled(mut self, enabled: bool) -> Self {
        self.plan_enabled = enabled;
        self
    }

    pub fn with_workspace_present(mut self, present: bool) -> Self {
        self.workspace_present = present;
        self
    }

    pub fn workspace_present(&self) -> bool {
        self.workspace_present
    }

    pub fn plan_enabled(&self) -> bool {
        self.plan_enabled
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

    pub fn queue_snapshot(&self) -> Option<&PriorityQueueSnapshot> {
        self.queue_snapshot.as_ref()
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

    pub fn preview_status_label(&self) -> &'static str {
        if !self.plan_enabled {
            return "waiting";
        }
        if self.auto_followup_pause_reason.is_some() {
            return "paused";
        }
        match self.workspace_status {
            PlanningRuntimeWorkspaceStatus::Uninitialized => "waiting",
            PlanningRuntimeWorkspaceStatus::Invalid => "blocked",
            PlanningRuntimeWorkspaceStatus::ReadyNoTask => {
                if self.proposal_summary.is_some() {
                    "review needed"
                } else {
                    "waiting"
                }
            }
            PlanningRuntimeWorkspaceStatus::ReadyWithTask => "ready",
        }
    }

    pub fn preview_detail(&self) -> Option<String> {
        self.preview_detail_with_limit(PREVIEW_DETAIL_LIMIT)
    }

    pub fn preview_detail_with_limit(&self, max_detail_len: usize) -> Option<String> {
        self.auto_followup_pause_reason()
            .or_else(|| self.failure_reason())
            .map(|detail| compact_whitespace_detail(detail, max_detail_len))
            .or_else(|| self.compact_queue_framing_summary(max_detail_len))
            .or_else(|| self.compact_proposal_summary_detail(max_detail_len))
    }

    pub fn compact_queue_framing_summary(&self, max_detail_len: usize) -> Option<String> {
        build_queue_framing_summary(self, max_detail_len)
    }

    pub fn compact_proposal_summary_detail(&self, max_detail_len: usize) -> Option<String> {
        self.proposal_summary()
            .map(|summary| compact_proposal_summary_detail(summary, max_detail_len))
    }

    pub fn blocks_auto_followup(&self) -> bool {
        (self.workspace_present && !self.plan_enabled)
            || self.workspace_status == PlanningRuntimeWorkspaceStatus::Invalid
            || self.auto_followup_pause_reason.is_some()
    }

    pub fn has_actionable_queue_head(&self) -> bool {
        self.plan_enabled
            && self.workspace_status == PlanningRuntimeWorkspaceStatus::ReadyWithTask
            && self.auto_followup_pause_reason.is_none()
    }

    pub fn has_proposal_candidates(&self) -> bool {
        self.proposal_summary.is_some()
    }
}

impl PlanningPromptService {
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

    pub fn load_runtime_snapshot(&self, workspace_dir: &str) -> Result<PlanningRuntimeSnapshot> {
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let workspace_present = workspace_record.has_any_files();

        if !workspace_present {
            return Ok(PlanningRuntimeSnapshot::uninitialized());
        }
        let plan_enabled = self
            .planning_workspace_port
            .load_optional_planning_file(workspace_dir, PLAN_OFF_FILE_PATH)?
            .is_none();

        let missing_paths = missing_workspace_paths(&workspace_record);
        if !missing_paths.is_empty() {
            return Ok(PlanningRuntimeSnapshot::invalid(format!(
                "planning files incomplete: missing {}",
                missing_paths.join(", ")
            ))
            .with_workspace_present(workspace_present)
            .with_plan_enabled(plan_enabled));
        }

        let workspace_files = workspace_record_to_files(&workspace_record);
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
            .with_workspace_present(workspace_present)
            .with_plan_enabled(plan_enabled));
        }

        let directions = validation_result
            .directions
            .expect("valid planning directions should be available");
        let task_ledger = validation_result
            .task_ledger
            .expect("valid planning task ledger should be available");
        let queue_snapshot = match self
            .priority_queue_service
            .build_snapshot(&directions, &task_ledger)
        {
            Ok(queue_snapshot) => queue_snapshot,
            Err(error) => {
                return Ok(PlanningRuntimeSnapshot::invalid(format!(
                    "planning queue build failed: {error}"
                ))
                .with_workspace_present(workspace_present)
                .with_plan_enabled(plan_enabled));
            }
        };
        let result_output_markdown = workspace_record
            .result_output_markdown
            .as_deref()
            .expect("complete planning workspace should include result output");
        let queue_summary = build_queue_summary(&queue_snapshot);
        let proposal_summary = build_proposal_summary(&queue_snapshot);
        let prompt_fragment =
            build_prompt_fragment(&directions, &queue_snapshot, result_output_markdown);
        let queue_idle_prompt_path =
            trimmed_non_empty(directions.queue_idle.prompt_path.as_str()).map(str::to_string);

        Ok(PlanningRuntimeSnapshot {
            workspace_present,
            plan_enabled,
            workspace_status: if queue_snapshot.next_task.is_some() {
                PlanningRuntimeWorkspaceStatus::ReadyWithTask
            } else {
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            },
            prompt_fragment: Some(prompt_fragment),
            queue_summary: Some(queue_summary),
            proposal_summary,
            queue_idle_policy: directions.queue_idle.policy,
            queue_idle_prompt_path,
            queue_head: queue_snapshot.next_task.clone(),
            queue_snapshot: Some(queue_snapshot),
            failure_reason: None,
            auto_followup_pause_reason: None,
        })
    }
}

fn workspace_record_to_files(
    workspace_record: &PlanningWorkspaceLoadRecord,
) -> PlanningWorkspaceFiles<'_> {
    PlanningWorkspaceFiles {
        directions_toml: workspace_record
            .directions_toml
            .as_deref()
            .expect("complete planning workspace should include directions"),
        task_ledger_json: workspace_record
            .task_ledger_json
            .as_deref()
            .expect("complete planning workspace should include task ledger"),
        task_ledger_schema_json: workspace_record
            .task_ledger_schema_json
            .as_deref()
            .expect("complete planning workspace should include task-ledger schema"),
        result_output_markdown: workspace_record
            .result_output_markdown
            .as_deref()
            .expect("complete planning workspace should include result output"),
    }
}

fn missing_workspace_paths(workspace_record: &PlanningWorkspaceLoadRecord) -> Vec<&'static str> {
    let mut missing_paths = Vec::new();
    if workspace_record.directions_toml.is_none() {
        missing_paths.push(DIRECTIONS_FILE_PATH);
    }
    if workspace_record.task_ledger_json.is_none() {
        missing_paths.push(TASK_LEDGER_FILE_PATH);
    }
    if workspace_record.task_ledger_schema_json.is_none() {
        missing_paths.push(TASK_LEDGER_SCHEMA_FILE_PATH);
    }
    if workspace_record.result_output_markdown.is_none() {
        missing_paths.push(RESULT_OUTPUT_FILE_PATH);
    }
    missing_paths
}

fn build_prompt_fragment(
    directions: &DirectionCatalogDocument,
    queue_snapshot: &PriorityQueueSnapshot,
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
    match queue_snapshot.next_task.as_ref() {
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

    if queue_snapshot.active_tasks.is_empty() {
        lines.push("- visible_tasks: none".to_string());
    } else {
        let visible_tasks = queue_snapshot.visible_tasks(MAX_VISIBLE_QUEUE_TASKS);
        lines.push(format!(
            "- visible_tasks: top {} of {}",
            visible_tasks.len(),
            queue_snapshot.active_tasks.len()
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

    if !queue_snapshot.proposed_tasks.is_empty() {
        let proposed_tasks = queue_snapshot.visible_proposed_tasks(MAX_VISIBLE_PROPOSED_TASKS);
        lines.push(format!(
            "- proposed_tasks: top {} of {} promotable proposals",
            proposed_tasks.len(),
            queue_snapshot.proposed_tasks.len()
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

    if !queue_snapshot.skipped_tasks.is_empty() {
        let skipped_tasks = queue_snapshot
            .skipped_tasks
            .iter()
            .take(MAX_SKIPPED_QUEUE_TASKS)
            .collect::<Vec<_>>();
        lines.push(format!(
            "- skipped_tasks: showing {} of {}",
            skipped_tasks.len(),
            queue_snapshot.skipped_tasks.len()
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
    lines.push("Task Ledger Mutation Contract".to_string());
    lines.push(format!("- You may edit only `{}`.", TASK_LEDGER_FILE_PATH));
    lines.push(format!(
        "- Do not edit `{}`, `{}`, `{}`, or `{}`.",
        DIRECTIONS_FILE_PATH,
        TASK_LEDGER_SCHEMA_FILE_PATH,
        QUEUE_SNAPSHOT_FILE_PATH,
        RESULT_OUTPUT_FILE_PATH,
    ));
    lines.push(
        "- New tasks must attach to an existing `direction_id` and include `direction_relation_note`."
            .to_string(),
    );
    lines.push(
        "- Do not write unrelated tasks that cannot be connected to the existing directions."
            .to_string(),
    );
    lines.push(
        "- Keep `task-ledger.json` valid JSON that satisfies the checked-in schema.".to_string(),
    );

    lines.push(String::new());
    lines.push("Result Output Prompt".to_string());
    if !result_output_markdown.is_empty() {
        lines.push(result_output_markdown.to_string());
    }
    lines.push(String::new());
    lines.push("Runtime Follow-up Proposal Rules".to_string());
    lines.push(
        "- If your final answer offers concrete follow-up options or variants, also add each option to `task-ledger.json` as a separate `proposed` task linked to an existing direction."
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

fn build_queue_summary(queue_snapshot: &PriorityQueueSnapshot) -> String {
    let now_detail = queue_snapshot
        .next_task
        .as_ref()
        .map(|task| {
            compact_queue_task_summary(task.task_title.as_str(), 1, 1, PREVIEW_DETAIL_LIMIT)
        })
        .unwrap_or_else(|| "none".to_string());

    let remaining_tasks = queue_snapshot
        .next_task
        .as_ref()
        .map(|current| {
            queue_snapshot
                .active_tasks
                .iter()
                .filter(|task| task.task_id != current.task_id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| queue_snapshot.active_tasks.iter().collect::<Vec<_>>());
    let next_detail = remaining_tasks
        .first()
        .map(|task| {
            compact_queue_task_summary(
                task.task_title.as_str(),
                remaining_tasks.len(),
                1,
                PREVIEW_DETAIL_LIMIT,
            )
        })
        .unwrap_or_else(|| "none".to_string());

    let proposed_detail = queue_snapshot
        .proposed_tasks
        .first()
        .map(|task| {
            compact_queue_task_summary(
                task.task_title.as_str(),
                queue_snapshot.proposed_tasks.len(),
                1,
                PREVIEW_DETAIL_LIMIT,
            )
        })
        .unwrap_or_else(|| "none".to_string());

    let blocked_detail = queue_snapshot
        .skipped_tasks
        .first()
        .map(|task| {
            compact_skipped_queue_task_summary(
                task,
                queue_snapshot.skipped_tasks.len(),
                PREVIEW_DETAIL_LIMIT,
            )
        })
        .unwrap_or_else(|| "none".to_string());

    format!(
        "now: {now_detail}  |  next: {next_detail}  |  proposed: {proposed_detail}  |  blocked: {blocked_detail}"
    )
}

fn build_proposal_summary(queue_snapshot: &PriorityQueueSnapshot) -> Option<String> {
    queue_snapshot.proposed_tasks.first().map(|task| {
        compact_queue_task_summary(
            task.task_title.as_str(),
            queue_snapshot.proposed_tasks.len(),
            1,
            PREVIEW_DETAIL_LIMIT,
        )
    })
}

fn build_queue_framing_summary(
    snapshot: &PlanningRuntimeSnapshot,
    max_detail_len: usize,
) -> Option<String> {
    build_queue_framing_details(snapshot, max_detail_len).map(|details| {
        format!(
            "now: {}  |  next: {}  |  proposed: {}  |  blocked: {}",
            details.now_detail,
            details.next_detail,
            details.proposed_detail,
            details.blocked_detail
        )
    })
}

fn build_queue_framing_details(
    snapshot: &PlanningRuntimeSnapshot,
    max_detail_len: usize,
) -> Option<QueueFramingDetails> {
    let queue_snapshot = snapshot.queue_snapshot();
    let has_queue_context = snapshot.workspace_present()
        || snapshot.queue_head().is_some()
        || snapshot.queue_summary().is_some()
        || snapshot.proposal_summary().is_some()
        || queue_snapshot.is_some();
    if !has_queue_context {
        return None;
    }

    let mut details = QueueFramingDetails {
        now_detail: "none".to_string(),
        next_detail: "none".to_string(),
        proposed_detail: "none".to_string(),
        blocked_detail: "none".to_string(),
    };

    if let Some(queue_snapshot) = queue_snapshot {
        details.now_detail = queue_snapshot
            .next_task
            .as_ref()
            .or_else(|| snapshot.queue_head())
            .map(|task| compact_queue_task_summary(task.task_title.as_str(), 1, 1, max_detail_len))
            .unwrap_or_else(|| "none".to_string());

        let remaining_tasks = queue_snapshot
            .next_task
            .as_ref()
            .map(|current| {
                queue_snapshot
                    .active_tasks
                    .iter()
                    .filter(|task| task.task_id != current.task_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| queue_snapshot.active_tasks.iter().collect::<Vec<_>>());
        details.next_detail = remaining_tasks
            .first()
            .map(|task| {
                compact_queue_task_summary(
                    task.task_title.as_str(),
                    remaining_tasks.len(),
                    1,
                    max_detail_len,
                )
            })
            .unwrap_or_else(|| "none".to_string());

        details.proposed_detail = queue_snapshot
            .proposed_tasks
            .first()
            .map(|task| {
                compact_queue_task_summary(
                    task.task_title.as_str(),
                    queue_snapshot.proposed_tasks.len(),
                    1,
                    max_detail_len,
                )
            })
            .or_else(|| snapshot.compact_proposal_summary_detail(max_detail_len))
            .unwrap_or_else(|| "none".to_string());

        details.blocked_detail = queue_snapshot
            .skipped_tasks
            .first()
            .map(|task| {
                compact_skipped_queue_task_summary(
                    task,
                    queue_snapshot.skipped_tasks.len(),
                    max_detail_len,
                )
            })
            .unwrap_or_else(|| "none".to_string());

        return Some(details);
    }

    if let Some(queue_head) = snapshot.queue_head() {
        details.now_detail =
            compact_queue_task_summary(queue_head.task_title.as_str(), 1, 1, max_detail_len);
    }

    if let Some(queue_summary) = snapshot.queue_summary() {
        apply_queue_framing_details(&mut details, queue_summary, max_detail_len);
    }

    if let Some(proposal_summary) = snapshot.proposal_summary() {
        details.proposed_detail = compact_proposal_summary_detail(proposal_summary, max_detail_len);
    }

    Some(details)
}

fn apply_queue_framing_details(
    details: &mut QueueFramingDetails,
    summary: &str,
    max_detail_len: usize,
) {
    for segment in summary.split("  |  ") {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(detail) = trimmed.strip_prefix("now: ") {
            details.now_detail = compact_whitespace_detail(detail, max_detail_len);
            continue;
        }
        if let Some(detail) = trimmed.strip_prefix("next: ") {
            details.next_detail = compact_whitespace_detail(detail, max_detail_len);
            continue;
        }
        if let Some(detail) = trimmed.strip_prefix("proposed: ") {
            details.proposed_detail = compact_whitespace_detail(detail, max_detail_len);
            continue;
        }
        if let Some(detail) = trimmed.strip_prefix("blocked: ") {
            details.blocked_detail = compact_whitespace_detail(detail, max_detail_len);
            continue;
        }
    }
}

fn compact_queue_task_summary(
    task_title: &str,
    total_count: usize,
    shown_count: usize,
    max_detail_len: usize,
) -> String {
    let mut summary = compact_whitespace_detail(task_title.trim(), max_detail_len);
    let hidden_count = total_count.saturating_sub(shown_count);
    if hidden_count > 0 {
        summary.push_str(&format!(" (+{hidden_count} more)"));
    }
    summary
}

fn compact_skipped_queue_task_summary(
    task: &crate::domain::planning::PriorityQueueSkippedTask,
    total_count: usize,
    max_detail_len: usize,
) -> String {
    let title = compact_whitespace_detail(task.task_title.as_str(), max_detail_len);
    let reason = compact_whitespace_detail(task.reason.as_str(), max_detail_len);
    let mut summary = format!("{title} ({reason})");
    let hidden_count = total_count.saturating_sub(1);
    if hidden_count > 0 {
        summary.push_str(&format!(" (+{hidden_count} more)"));
    }
    summary
}

fn compact_proposal_summary_detail(summary: &str, max_detail_len: usize) -> String {
    compact_whitespace_detail(summary, max_detail_len)
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
    use std::sync::Arc;

    use anyhow::Result;

    use super::{PlanningPromptService, PlanningRuntimeWorkspaceStatus};
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning_bootstrap_service::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning_contract::{
        DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
        TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
    };
    use crate::application::service::planning_validation_service::PlanningValidationService;
    use crate::application::service::priority_queue_service::PriorityQueueService;

    #[derive(Default)]
    struct FakePlanningWorkspacePort {
        load_record: PlanningWorkspaceLoadRecord,
    }

    impl PlanningWorkspacePort for FakePlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[crate::application::port::outbound::planning_workspace_port::PlanningDraftFileRecord],
        ) -> Result<
            crate::application::port::outbound::planning_workspace_port::PlanningDraftStageRecord,
        > {
            unreachable!("staging is not used in planning prompt service tests")
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(self.load_record.clone())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            let body = match relative_path {
                DIRECTIONS_FILE_PATH => self.load_record.directions_toml.clone(),
                TASK_LEDGER_FILE_PATH => self.load_record.task_ledger_json.clone(),
                TASK_LEDGER_SCHEMA_FILE_PATH => self.load_record.task_ledger_schema_json.clone(),
                QUEUE_SNAPSHOT_FILE_PATH => self.load_record.queue_snapshot_json.clone(),
                RESULT_OUTPUT_FILE_PATH => self.load_record.result_output_markdown.clone(),
                _ => None,
            };
            Ok(body)
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<
            crate::application::port::outbound::planning_workspace_port::PlanningDraftLoadRecord,
        > {
            unreachable!("draft loads are not used in planning prompt service tests")
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            unreachable!("draft replacement is not used in planning prompt service tests")
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
            _body: Option<&str>,
        ) -> Result<()> {
            unreachable!("file replacement is not used in planning prompt service tests")
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            unreachable!("archive writes are not used in planning prompt service tests")
        }
    }

    fn sample_service(load_record: PlanningWorkspaceLoadRecord) -> PlanningPromptService {
        PlanningPromptService::new(
            Arc::new(FakePlanningWorkspacePort { load_record }),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
    }

    #[test]
    fn missing_all_planning_files_keeps_runtime_snapshot_uninitialized() {
        let result = sample_service(PlanningWorkspaceLoadRecord::default())
            .load_runtime_snapshot("/tmp/workspace")
            .expect("planning runtime snapshot should load");

        assert_eq!(
            result.workspace_status(),
            PlanningRuntimeWorkspaceStatus::Uninitialized
        );
        assert!(!result.blocks_auto_followup());
        assert!(!result.has_actionable_queue_head());
    }

    #[test]
    fn partial_planning_workspace_blocks_auto_followup() {
        let bootstrap_artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(bootstrap_artifacts.directions_toml),
            task_ledger_json: Some(bootstrap_artifacts.task_ledger_json),
            task_ledger_schema_json: None,
            queue_snapshot_json: None,
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_runtime_snapshot("/tmp/workspace")
        .expect("planning runtime snapshot should load");

        assert_eq!(
            result.workspace_status(),
            PlanningRuntimeWorkspaceStatus::Invalid
        );
        let reason = result
            .failure_reason()
            .expect("partial workspace should capture a failure reason");
        assert!(reason.contains("task-ledger.schema.json"));
        assert!(result.blocks_auto_followup());
    }

    #[test]
    fn valid_planning_workspace_without_queue_head_is_ready_no_task() {
        let bootstrap_artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(bootstrap_artifacts.directions_toml),
            task_ledger_json: Some(bootstrap_artifacts.task_ledger_json),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json),
            queue_snapshot_json: None,
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_runtime_snapshot("/tmp/workspace")
        .expect("planning runtime snapshot should load");

        assert_eq!(
            result.workspace_status(),
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
        );
        assert_eq!(result.queue_head(), None);
        assert_eq!(
            result.queue_summary(),
            Some("now: none  |  next: none  |  proposed: none  |  blocked: none")
        );
        assert_eq!(result.proposal_summary(), None);
        assert!(!result.has_actionable_queue_head());
        assert!(!result.blocks_auto_followup());
    }

    #[test]
    fn proposed_followups_are_surfaceable_when_no_executable_queue_head_exists() {
        let bootstrap_artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(bootstrap_artifacts.directions_toml),
            task_ledger_json: Some(
                r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-followup-1",
      "direction_id": "example-direction",
      "direction_relation_note": "The answer offered a concrete next-step variant under the current direction.",
      "title": "Draft a sushi-chef roadmap",
      "description": "Persist the offered roadmap option as a follow-up candidate.",
      "status": "proposed",
      "base_priority": 30,
      "dynamic_priority_delta": 0,
      "priority_reason": "Suggested follow-up option from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-09T09:00:00Z"
    }
  ]
}"#
                .to_string(),
            ),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json),
            queue_snapshot_json: None,
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_runtime_snapshot("/tmp/workspace")
        .expect("planning runtime snapshot should load");

        assert_eq!(
            result.workspace_status(),
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
        );
        assert_eq!(
            result.queue_summary(),
            Some(
                "now: none  |  next: none  |  proposed: Draft a sushi-chef roadmap  |  blocked: none"
            )
        );
        assert_eq!(
            result.proposal_summary(),
            Some("Draft a sushi-chef roadmap")
        );
        let prompt_fragment = result
            .prompt_fragment()
            .expect("valid workspace should expose a prompt fragment");
        assert!(prompt_fragment.contains("proposed_tasks: top 1 of 1 promotable proposals"));
        assert!(
            prompt_fragment
                .contains("proposal rank 1 | task-followup-1 | Draft a sushi-chef roadmap")
        );
        assert!(prompt_fragment.contains("combined_priority=30"));
        assert!(prompt_fragment.contains("Runtime Follow-up Proposal Rules"));
        assert!(
            prompt_fragment
                .contains("move the actionable worklist into normal queue tasks with priorities")
        );
        assert!(result.has_proposal_candidates());
    }

    #[test]
    fn preview_detail_uses_canonical_queue_and_proposal_summaries() {
        let snapshot = super::PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: Draft roadmap  |  next: none  |  proposed: Draft checklist (+1 more)  |  blocked: none".to_string(),
            Some("Draft checklist (+1 more)".to_string()),
            None,
        );

        assert_eq!(
            snapshot.preview_detail().as_deref(),
            Some(
                "now: Draft roadmap  |  next: none  |  proposed: Draft checklist (+1 more)  |  blocked: none"
            )
        );
    }

    #[test]
    fn non_promotable_proposals_do_not_surface_as_proposal_candidates() {
        let bootstrap_artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(
                r#"
version = 1

[[directions]]
id = "example-direction"
title = "Example direction"
summary = "Keep the product moving"
success_criteria = ["done"]
scope_hints = ["stay focused"]
state = "paused"
"#
                .to_string(),
            ),
            task_ledger_json: Some(
                r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-followup-1",
      "direction_id": "example-direction",
      "direction_relation_note": "The answer offered a concrete next-step variant under the current direction.",
      "title": "Draft a sushi-chef roadmap",
      "description": "Persist the offered roadmap option as a follow-up candidate.",
      "status": "proposed",
      "base_priority": 30,
      "dynamic_priority_delta": 4,
      "priority_reason": "Suggested follow-up option from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-09T09:00:00Z"
    }
  ]
}"#
                .to_string(),
            ),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json),
            queue_snapshot_json: None,
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_runtime_snapshot("/tmp/workspace")
        .expect("planning runtime snapshot should load");

        assert_eq!(result.proposal_summary(), None);
        assert!(!result.has_proposal_candidates());
        let prompt_fragment = result
            .prompt_fragment()
            .expect("valid workspace should expose a prompt fragment");
        assert!(prompt_fragment.contains("skipped_tasks: showing 1 of 1"));
        assert!(prompt_fragment.contains("direction example-direction is paused"));
    }

    #[test]
    fn valid_planning_workspace_builds_prompt_fragment_with_queue_context() {
        let bootstrap_artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(bootstrap_artifacts.directions_toml),
            task_ledger_json: Some(
                r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "title": "Implement the next slice",
      "description": "Move the planning work forward.",
      "status": "ready",
      "base_priority": 8,
      "dynamic_priority_delta": 2,
      "priority_reason": "user requested this next",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-09T09:00:00Z"
    }
  ]
}"#
                .to_string(),
            ),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json),
            queue_snapshot_json: None,
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_runtime_snapshot("/tmp/workspace")
        .expect("planning runtime snapshot should load");

        assert_eq!(
            result.workspace_status(),
            PlanningRuntimeWorkspaceStatus::ReadyWithTask
        );
        let prompt_fragment = result
            .prompt_fragment()
            .expect("valid workspace should expose a prompt fragment");
        assert!(prompt_fragment.contains("Planning Context"));
        assert!(prompt_fragment.contains("Direction Summary"));
        assert!(prompt_fragment.contains("Queue Summary"));
        assert!(prompt_fragment.contains("task-1"));
        assert!(prompt_fragment.contains("Result Output Prompt"));
        assert!(prompt_fragment.contains("Runtime Follow-up Proposal Rules"));
        assert!(
            result
                .queue_summary()
                .expect("valid workspace should expose a queue summary")
                .contains("now: Implement the next slice")
        );
        assert_eq!(
            result
                .queue_head()
                .expect("valid workspace should expose the queue head")
                .task_id,
            "task-1"
        );
        assert!(result.has_actionable_queue_head());
    }

    #[test]
    fn invalid_planning_workspace_blocks_auto_followup_with_validation_reason() {
        let bootstrap_artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let result = sample_service(PlanningWorkspaceLoadRecord {
            directions_toml: Some(bootstrap_artifacts.directions_toml),
            task_ledger_json: Some(
                r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1"
    }
  ]
}"#
                .to_string(),
            ),
            task_ledger_schema_json: Some(bootstrap_artifacts.task_ledger_schema_json),
            queue_snapshot_json: None,
            result_output_markdown: Some(bootstrap_artifacts.result_output_markdown),
        })
        .load_runtime_snapshot("/tmp/workspace")
        .expect("planning runtime snapshot should load");

        assert_eq!(
            result.workspace_status(),
            PlanningRuntimeWorkspaceStatus::Invalid
        );
        let reason = result
            .failure_reason()
            .expect("invalid planning workspace should expose a failure reason");
        assert!(reason.contains("planning validation failed"));
    }
}
