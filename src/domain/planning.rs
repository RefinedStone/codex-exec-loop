use serde::{Deserialize, Serialize};

pub const PLANNING_FORMAT_VERSION: u32 = 1;
pub const PLANNING_OFFICIAL_COMPLETION_REFRESH_CONTRACT_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkspaceState {
    Uninitialized,
    Authoring,
    Ready,
    Executing,
    Repairing,
    BlockedInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningAuthorityLocation {
    pub workspace_root: String,
    pub canonical_repo_root: String,
    pub runtime_dir: String,
    pub authority_store_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAuthorityShadowStoreSyncState {
    Bootstrapped,
    InSync,
    Resynced,
}

impl PlanningAuthorityShadowStoreSyncState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bootstrapped => "bootstrapped",
            Self::InSync => "in_sync",
            Self::Resynced => "resynced",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningAuthorityShadowStoreInspection {
    pub location: PlanningAuthorityLocation,
    pub sync_state: PlanningAuthorityShadowStoreSyncState,
    pub mirrored_document_count: usize,
    pub parity_issue_count: usize,
    pub parity_issue_examples: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningFileKind {
    Directions,
    TaskLedger,
    TaskLedgerSchema,
    ResultOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningValidationSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningValidationIssue {
    pub severity: PlanningValidationSeverity,
    pub file_kind: PlanningFileKind,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanningValidationReport {
    pub issues: Vec<PlanningValidationIssue>,
}

impl PlanningValidationReport {
    pub fn new() -> Self {
        Self { issues: Vec::new() }
    }

    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == PlanningValidationSeverity::Error)
    }

    pub fn is_valid(&self) -> bool {
        !self.has_errors()
    }

    pub fn push_error(
        &mut self,
        file_kind: PlanningFileKind,
        code: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.issues.push(PlanningValidationIssue {
            severity: PlanningValidationSeverity::Error,
            file_kind,
            code: code.into(),
            message: message.into(),
        });
    }

    pub fn push_warning(
        &mut self,
        file_kind: PlanningFileKind,
        code: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.issues.push(PlanningValidationIssue {
            severity: PlanningValidationSeverity::Warning,
            file_kind,
            code: code.into(),
            message: message.into(),
        });
    }

    pub fn errors(&self) -> Vec<&PlanningValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == PlanningValidationSeverity::Error)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionCatalogDocument {
    pub version: u32,
    #[serde(default)]
    pub queue_idle: QueueIdleConfig,
    pub directions: Vec<DirectionDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionDefinition {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub scope_hints: Vec<String>,
    #[serde(default)]
    pub detail_doc_path: String,
    pub state: DirectionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueIdleConfig {
    #[serde(default)]
    pub policy: QueueIdlePolicy,
    #[serde(default)]
    pub prompt_path: String,
}

impl Default for QueueIdleConfig {
    fn default() -> Self {
        Self {
            policy: QueueIdlePolicy::Stop,
            prompt_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueIdlePolicy {
    #[default]
    Stop,
    ReviewAndEnqueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionState {
    Active,
    Paused,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLedgerDocument {
    pub version: u32,
    #[serde(default)]
    pub tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDefinition {
    pub id: String,
    pub direction_id: String,
    #[serde(default)]
    pub direction_relation_note: String,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub base_priority: i32,
    #[serde(default)]
    pub dynamic_priority_delta: i32,
    #[serde(default)]
    pub priority_reason: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    pub created_by: TaskActor,
    pub last_updated_by: TaskActor,
    #[serde(default)]
    pub source_turn_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Ready,
    Blocked,
    InProgress,
    Done,
    Cancelled,
    AwaitingUser,
    Proposed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskActor {
    User,
    Llm,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriorityQueueSnapshot {
    pub next_task: Option<PriorityQueueTask>,
    pub active_tasks: Vec<PriorityQueueTask>,
    pub proposed_tasks: Vec<PriorityQueueTask>,
    pub skipped_tasks: Vec<PriorityQueueSkippedTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriorityQueueTask {
    pub rank: usize,
    pub task_id: String,
    pub direction_id: String,
    pub direction_title: String,
    pub task_title: String,
    pub status: TaskStatus,
    pub combined_priority: i32,
    pub updated_at: String,
    pub rank_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriorityQueueSkippedTask {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: TaskStatus,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningRefreshContractKind {
    OfficialCompletion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningOfficialCompletionRefreshPayload {
    pub agent_id: String,
    pub task_id: String,
    pub task_title: String,
    pub branch_name: String,
    pub worktree_path: String,
    pub commit_sha: String,
    pub validation_summary: String,
    pub final_response_summary: String,
    #[serde(default)]
    pub final_response_text: Option<String>,
    #[serde(default)]
    pub failure_context: Option<String>,
    pub completed_at: String,
}

impl PlanningOfficialCompletionRefreshPayload {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_id: impl Into<String>,
        task_id: impl Into<String>,
        task_title: impl Into<String>,
        branch_name: impl Into<String>,
        worktree_path: impl Into<String>,
        commit_sha: impl Into<String>,
        validation_summary: impl Into<String>,
        final_response_summary: impl Into<String>,
        final_response_text: Option<String>,
        failure_context: Option<String>,
        completed_at: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            task_id: task_id.into(),
            task_title: task_title.into(),
            branch_name: branch_name.into(),
            worktree_path: worktree_path.into(),
            commit_sha: commit_sha.into(),
            validation_summary: validation_summary.into(),
            final_response_summary: final_response_summary.into(),
            final_response_text,
            failure_context,
            completed_at: completed_at.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningOfficialCompletionRefreshContract {
    pub version: u32,
    pub refresh_kind: PlanningRefreshContractKind,
    pub root_turn_id: String,
    pub refresh_order: u64,
    pub completion: PlanningOfficialCompletionRefreshPayload,
}

impl PlanningOfficialCompletionRefreshContract {
    pub fn new(
        root_turn_id: impl Into<String>,
        refresh_order: u64,
        completion: PlanningOfficialCompletionRefreshPayload,
    ) -> Self {
        Self {
            version: PLANNING_OFFICIAL_COMPLETION_REFRESH_CONTRACT_VERSION,
            refresh_kind: PlanningRefreshContractKind::OfficialCompletion,
            root_turn_id: root_turn_id.into(),
            refresh_order,
            completion,
        }
    }
}

impl DirectionState {
    pub fn allows_queue_execution(self) -> bool {
        self == Self::Active
    }
}

impl QueueIdlePolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::ReviewAndEnqueue => "review_and_enqueue",
        }
    }
}

impl TaskStatus {
    pub fn queue_readiness_rank(self) -> Option<u8> {
        match self {
            Self::InProgress => Some(0),
            Self::Ready => Some(1),
            Self::Blocked | Self::Done | Self::Cancelled | Self::AwaitingUser | Self::Proposed => {
                None
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Blocked => "blocked",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::AwaitingUser => "awaiting_user",
            Self::Proposed => "proposed",
        }
    }

    pub fn is_dependency_complete(self) -> bool {
        self == Self::Done
    }

    pub fn clears_blocker(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::AwaitingUser)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PLANNING_OFFICIAL_COMPLETION_REFRESH_CONTRACT_VERSION,
        PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
        PlanningRefreshContractKind,
    };

    #[test]
    fn official_completion_refresh_contract_round_trips_as_versioned_json() {
        let contract = PlanningOfficialCompletionRefreshContract::new(
            "turn-42",
            7,
            PlanningOfficialCompletionRefreshPayload::new(
                "agent-2",
                "task-9",
                "Official completion pipeline 구현",
                "akra-agent/slot-1/official-completion",
                "/tmp/slot-1",
                "abc123def456",
                "cargo test passed",
                "official completion lifecycle wired",
                Some("Implemented official completion reporting.".to_string()),
                None,
                "2026-04-17T09:10:00Z",
            ),
        );

        let serialized =
            serde_json::to_string_pretty(&contract).expect("contract should serialize");
        let restored: PlanningOfficialCompletionRefreshContract =
            serde_json::from_str(&serialized).expect("contract should deserialize");

        assert_eq!(
            restored.version,
            PLANNING_OFFICIAL_COMPLETION_REFRESH_CONTRACT_VERSION
        );
        assert_eq!(
            restored.refresh_kind,
            PlanningRefreshContractKind::OfficialCompletion
        );
        assert_eq!(restored.root_turn_id, "turn-42");
        assert_eq!(restored.refresh_order, 7);
        assert_eq!(restored.completion.task_id, "task-9");
        assert_eq!(
            restored.completion.final_response_text.as_deref(),
            Some("Implemented official completion reporting.")
        );
    }
}

impl TaskDefinition {
    pub fn requires_relation_note(&self) -> bool {
        self.created_by == TaskActor::Llm || self.last_updated_by == TaskActor::Llm
    }

    pub fn combined_priority(&self) -> i32 {
        self.base_priority + self.dynamic_priority_delta
    }
}

#[derive(Debug, Clone)]
pub struct PlanningWorkspaceFiles<'a> {
    pub directions_toml: &'a str,
    pub task_ledger_json: &'a str,
    pub task_ledger_schema_json: &'a str,
    pub result_output_markdown: &'a str,
}

#[derive(Debug, Clone)]
pub struct PlanningValidationResult {
    pub directions: Option<DirectionCatalogDocument>,
    pub task_ledger: Option<TaskLedgerDocument>,
    pub report: PlanningValidationReport,
}

impl PlanningValidationResult {
    pub fn is_valid(&self) -> bool {
        self.report.is_valid()
    }
}

impl PriorityQueueSnapshot {
    pub fn visible_tasks(&self, limit: usize) -> Vec<PriorityQueueTask> {
        self.active_tasks.iter().take(limit).cloned().collect()
    }

    pub fn visible_proposed_tasks(&self, limit: usize) -> Vec<PriorityQueueTask> {
        self.proposed_tasks.iter().take(limit).cloned().collect()
    }
}
