use serde::{Deserialize, Serialize};

pub const PLANNING_FORMAT_VERSION: u32 = 1;
pub const DIRECTIONS_FILE_PATH: &str = ".codex-exec-loop/planning/directions.toml";
pub const TASK_LEDGER_FILE_PATH: &str = ".codex-exec-loop/planning/task-ledger.json";
pub const TASK_LEDGER_SCHEMA_FILE_PATH: &str = ".codex-exec-loop/planning/task-ledger.schema.json";
pub const QUEUE_SNAPSHOT_FILE_PATH: &str = ".codex-exec-loop/planning/queue.snapshot.json";
pub const RESULT_OUTPUT_FILE_PATH: &str = ".codex-exec-loop/planning/result-output.md";
pub const PLANNING_DRAFTS_DIRECTORY: &str = ".codex-exec-loop/planning/drafts";
pub const PLANNING_REJECTED_DIRECTORY: &str = ".codex-exec-loop/planning/rejected";
pub const ACTIVE_PLANNING_FILE_PATHS: [&str; 5] = [
    DIRECTIONS_FILE_PATH,
    TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH,
    QUEUE_SNAPSHOT_FILE_PATH,
    RESULT_OUTPUT_FILE_PATH,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkspaceState {
    Uninitialized,
    Authoring,
    Ready,
    Executing,
    Repairing,
    BlockedInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningFileKind {
    Directions,
    TaskLedger,
    TaskLedgerSchema,
    ResultOutput,
}

impl PlanningFileKind {
    pub fn path(self) -> &'static str {
        match self {
            Self::Directions => DIRECTIONS_FILE_PATH,
            Self::TaskLedger => TASK_LEDGER_FILE_PATH,
            Self::TaskLedgerSchema => TASK_LEDGER_SCHEMA_FILE_PATH,
            Self::ResultOutput => RESULT_OUTPUT_FILE_PATH,
        }
    }
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
    pub state: DirectionState,
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

impl DirectionState {
    pub fn allows_queue_execution(self) -> bool {
        self == Self::Active
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

pub fn canonical_active_planning_file_path(path: &str) -> Option<&'static str> {
    let normalized = path.replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");

    ACTIVE_PLANNING_FILE_PATHS
        .iter()
        .copied()
        .find(|candidate| {
            normalized
                .strip_suffix(candidate)
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('/'))
        })
}

#[cfg(test)]
mod tests {
    use super::{
        DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
        canonical_active_planning_file_path,
    };

    #[test]
    fn canonical_active_planning_file_path_matches_relative_and_absolute_paths() {
        assert_eq!(
            canonical_active_planning_file_path(DIRECTIONS_FILE_PATH),
            Some(DIRECTIONS_FILE_PATH)
        );
        assert_eq!(
            canonical_active_planning_file_path("./.codex-exec-loop/planning/task-ledger.json"),
            Some(TASK_LEDGER_FILE_PATH)
        );
        assert_eq!(
            canonical_active_planning_file_path(
                "/tmp/workspace/.codex-exec-loop/planning/result-output.md"
            ),
            Some(RESULT_OUTPUT_FILE_PATH)
        );
        assert_eq!(
            canonical_active_planning_file_path(
                r"C:\workspace\.codex-exec-loop\planning\task-ledger.json"
            ),
            Some(TASK_LEDGER_FILE_PATH)
        );
        assert!(canonical_active_planning_file_path("src/main.rs").is_none());
    }
}
