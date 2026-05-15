use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;
use crate::domain::planning::{
    PlanningValidationReport, PriorityQueueProjection, PriorityQueueTask, QueueIdlePolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeWorkspaceStatus {
    Uninitialized,
    Invalid,
    ReadyNoTask,
    ReadyWithTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProjection {
    pub(crate) workspace_present: bool,
    pub(crate) workspace_status: RuntimeWorkspaceStatus,
    pub(crate) prompt_fragment: Option<String>,
    pub(crate) queue_summary: Option<String>,
    pub(crate) proposal_summary: Option<String>,
    pub(crate) queue_idle_policy: QueueIdlePolicy,
    pub(crate) queue_idle_prompt_path: Option<String>,
    pub(crate) queue_head: Option<PriorityQueueTask>,
    pub(crate) queue_projection: Option<PriorityQueueProjection>,
    pub(crate) task_authority_signature: Option<u64>,
    pub(crate) queue_head_task_signature: Option<u64>,
    pub(crate) failure_reason: Option<String>,
    pub(crate) auto_follow_pause_reason: Option<String>,
}

impl RuntimeProjection {
    pub fn uninitialized() -> Self {
        Self {
            workspace_present: false,
            workspace_status: RuntimeWorkspaceStatus::Uninitialized,
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
            auto_follow_pause_reason: None,
        }
    }

    pub fn invalid(reason: impl Into<String>) -> Self {
        Self {
            workspace_present: true,
            workspace_status: RuntimeWorkspaceStatus::Invalid,
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
            auto_follow_pause_reason: None,
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
                RuntimeWorkspaceStatus::ReadyWithTask
            } else {
                RuntimeWorkspaceStatus::ReadyNoTask
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
            auto_follow_pause_reason: None,
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
                RuntimeWorkspaceStatus::ReadyWithTask
            } else {
                RuntimeWorkspaceStatus::ReadyNoTask
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
            auto_follow_pause_reason: None,
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

    pub fn workspace_status(&self) -> RuntimeWorkspaceStatus {
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

    pub fn auto_follow_pause_reason(&self) -> Option<&str> {
        self.auto_follow_pause_reason.as_deref()
    }

    pub fn with_auto_follow_pause_reason(&self, reason: impl Into<String>) -> Self {
        let mut projection = self.clone();
        projection.auto_follow_pause_reason = Some(reason.into());
        projection
    }

    #[cfg(test)]
    pub(crate) fn with_test_signatures(
        &self,
        task_authority_signature: Option<u64>,
        queue_head_task_signature: Option<u64>,
    ) -> Self {
        let mut projection = self.clone();
        projection.task_authority_signature = task_authority_signature;
        projection.queue_head_task_signature = queue_head_task_signature;
        projection
    }

    pub fn preview_status_label(&self) -> &'static str {
        match self.workspace_status {
            RuntimeWorkspaceStatus::Uninitialized => "inactive",
            RuntimeWorkspaceStatus::Invalid => "blocked",
            RuntimeWorkspaceStatus::ReadyNoTask | RuntimeWorkspaceStatus::ReadyWithTask => "ready",
        }
    }

    pub fn preview_detail(&self) -> Option<&str> {
        self.auto_follow_pause_reason()
            .or_else(|| self.failure_reason())
            .or_else(|| self.queue_summary())
            .or_else(|| self.proposal_summary())
    }

    pub fn blocks_auto_follow(&self) -> bool {
        self.workspace_status == RuntimeWorkspaceStatus::Invalid
            || self.auto_follow_pause_reason.is_some()
    }

    pub fn has_actionable_queue_head(&self) -> bool {
        self.workspace_status == RuntimeWorkspaceStatus::ReadyWithTask
            && self.auto_follow_pause_reason.is_none()
    }

    pub fn has_proposal_candidates(&self) -> bool {
        self.proposal_summary.is_some()
    }

    pub fn queue_is_drained(&self) -> bool {
        if self.workspace_status != RuntimeWorkspaceStatus::ReadyNoTask
            || self.queue_head.is_some()
            || self.has_proposal_candidates()
        {
            return false;
        }
        self.queue_projection.as_ref().is_none_or(|projection| {
            projection.active_tasks.is_empty()
                && projection.proposed_tasks.is_empty()
                && projection.skipped_tasks.iter().all(|task| {
                    matches!(
                        task.status,
                        crate::domain::planning::TaskStatus::Done
                            | crate::domain::planning::TaskStatus::Cancelled
                    )
                })
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionSnapshot {
    pub result_output_markdown: Option<String>,
}

impl ExecutionSnapshot {
    pub fn captures_path(path: &str) -> bool {
        canonical_active_planning_file_path(path).is_some()
    }
}

const RESULT_OUTPUT_FILE_PATH: &str = ".codex-exec-loop/planning/result-output.md";
const ACTIVE_PLANNING_FILE_PATHS: [&str; 1] = [RESULT_OUTPUT_FILE_PATH];

fn canonical_active_planning_file_path(path: &str) -> Option<&'static str> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSnapshotCapture {
    pub workspace_directory: String,
    pub state: TurnSnapshotCaptureState,
}

impl TurnSnapshotCapture {
    pub fn ready(workspace_directory: impl Into<String>, snapshot: ExecutionSnapshot) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            state: TurnSnapshotCaptureState::Ready(snapshot),
        }
    }

    pub fn capture_failed(workspace_directory: impl Into<String>, message: String) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            state: TurnSnapshotCaptureState::CaptureFailed(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnSnapshotCaptureState {
    Ready(ExecutionSnapshot),
    CaptureFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskHandoff {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub status_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainSessionHandoff {
    pub prompt: String,
    pub transcript_text: String,
    pub task: TaskHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubSessionHandoff {
    pub prompt: String,
    pub developer_instructions: String,
    pub service_name: String,
    pub task: TaskHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeQueuedAutoFollowPrompt {
    pub prompt: String,
    pub transcript_text: String,
    pub handoff_task: Option<TaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptRequest {
    pub workspace_directory: String,
    pub raw_prompt: String,
    pub parent_thread_id: Option<String>,
    pub parent_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPlanningBootstrapReview {
    pub draft_name: String,
    pub staged_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualPlanningBootstrapFailureKind {
    Stage,
    Promote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualPromptOutcome {
    PromptReady {
        transcript_text: String,
        runtime_projection: Box<RuntimeProjection>,
        intake: Box<ManualPromptIntakeOutcome>,
    },
    BootstrapReviewRequired {
        transcript_text: String,
        runtime_projection: Box<RuntimeProjection>,
        review: ManualPlanningBootstrapReview,
    },
    BootstrapFailed {
        transcript_text: String,
        runtime_projection: Box<RuntimeProjection>,
        kind: ManualPlanningBootstrapFailureKind,
        reason: String,
    },
    Rejected {
        transcript_text: String,
        runtime_projection: Box<RuntimeProjection>,
        reason: String,
    },
}

impl ManualPromptOutcome {
    pub fn runtime_projection(&self) -> &RuntimeProjection {
        match self {
            ManualPromptOutcome::PromptReady {
                runtime_projection, ..
            }
            | ManualPromptOutcome::BootstrapReviewRequired {
                runtime_projection, ..
            }
            | ManualPromptOutcome::BootstrapFailed {
                runtime_projection, ..
            }
            | ManualPromptOutcome::Rejected {
                runtime_projection, ..
            } => runtime_projection,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptIntakeRequest {
    pub workspace_directory: String,
    pub raw_prompt: String,
    pub legacy_source_turn_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub parent_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptMainSessionHandoff {
    pub prompt: String,
    pub transcript_text: String,
    pub task: Option<TaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualPromptIntakeOutcome {
    TaskCommitted {
        committed_task_id: String,
        committed_planning_revision: i64,
        handoff: ManualPromptMainSessionHandoff,
    },
    TaskUpdated {
        updated_task_id: String,
        committed_planning_revision: i64,
        handoff: ManualPromptMainSessionHandoff,
    },
    Rejected {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnRequest {
    pub context: PostTurnContext,
    pub workspace_directory: String,
    pub completed_turn_id: String,
    pub changed_planning_file_paths: Vec<String>,
    pub execution_snapshot_capture: Option<TurnSnapshotCapture>,
    pub planning_worker_panel_state: PlanningWorkerPanelState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnContext {
    pub thread_id: String,
    pub planning_workspace_directory: String,
    pub latest_user_message: Option<String>,
    pub latest_main_reply: Option<String>,
    pub previous_handoff_task: Option<TaskHandoff>,
    pub current_runtime_projection: RuntimeProjection,
    pub continuation_paused: bool,
    pub can_queue_next: bool,
    pub stop_keyword: String,
    pub stop_keyword_matched: bool,
    pub no_file_changes_stop_matched: bool,
    pub mode_label: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlanningWorkerStatus {
    #[default]
    Idle,
    RefreshRunning,
    RefreshSucceeded,
    RefreshFailed,
    RepairRunning,
    RepairSucceeded,
    RepairFailed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningWorkerPanelState {
    pub status: PlanningWorkerStatus,
    pub last_operation_label: Option<String>,
    pub last_summary: Option<String>,
    pub last_rejected_summary: Option<String>,
    pub last_queue_summary: Option<String>,
    pub last_notice_detail: Option<String>,
    pub last_prompt: Option<String>,
    pub last_response: Option<String>,
    pub last_host_detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnOutcome {
    pub provenance: PostTurnProvenance,
    pub runtime_projection: RuntimeProjection,
    pub planning_repair_state: Option<PostTurnPlanningRepairState>,
    pub runtime_notices: Vec<String>,
    pub action: PostTurnContinuationAction,
    pub operator_alerts: Vec<OperatorAlert>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnPlanningRepairState {
    pub attempts_used: usize,
    pub max_attempts: usize,
    pub latest_request: PlanningRepairRequestSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRepairRequestSnapshot {
    pub failure_summary: String,
    pub validation_errors: Vec<String>,
    pub direction_authority_json: String,
    pub accepted_task_authority_json: String,
    pub rejected_task_authority_json: Option<String>,
    pub result_output_markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnProvenance {
    pub completed_turn_id: String,
    pub handoff_task: Option<TaskHandoff>,
    pub parallel_queue_signal: Option<ParallelModePostTurnQueueSignal>,
}

impl PostTurnProvenance {
    pub fn new(completed_turn_id: String) -> Self {
        Self {
            completed_turn_id,
            handoff_task: None,
            parallel_queue_signal: None,
        }
    }

    pub fn with_handoff_task(mut self, handoff_task: Option<TaskHandoff>) -> Self {
        self.handoff_task = handoff_task;
        self
    }

    pub fn with_parallel_queue_signal(
        mut self,
        parallel_queue_signal: Option<ParallelModePostTurnQueueSignal>,
    ) -> Self {
        self.parallel_queue_signal = parallel_queue_signal;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnQueuedPrompt {
    pub prompt: String,
    pub mode_label: String,
    pub transcript_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostTurnContinuationAction {
    QueueAutoPrompt(Box<PostTurnQueuedPrompt>),
    SkipAutoFollow {
        reason: PostTurnAutoFollowSkipReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostTurnAutoFollowSkipReason {
    PostTurnContinuationPaused,
    LimitReached,
    NoAgentReply,
    StopKeywordMatched,
    NoFileChanges,
    PlanningBlocked,
    PlanningQueueIdlePolicyStop,
    PlanningQueueHeadRequired,
    PlanningQueueDrained,
    PlanningRepeatedQueueHead,
    ParallelSessionCompleted,
    PostTurnEvaluationTimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, allow(dead_code))]
pub struct PostTurnExecution {
    pub thread_id: String,
    pub completed_turn_id: String,
    pub evaluation: PostTurnOutcome,
    pub planning_worker_panel_state: PlanningWorkerPanelState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelTurnHandoff {
    pub task_id: String,
    pub task_title: String,
}

impl ParallelTurnHandoff {
    pub fn new(task_id: impl Into<String>, task_title: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            task_title: task_title.into(),
        }
    }
}
