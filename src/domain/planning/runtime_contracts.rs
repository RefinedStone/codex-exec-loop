use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::{
    ParallelModePostTurnQueueSignal, ParallelModeSlotLeaseSnapshot,
};
use crate::domain::planning::{
    PlanningValidationReport, PriorityQueueProjection, PriorityQueueTask, QueueIdlePolicy,
    TaskAuthorityDocument, TaskStatus,
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
    // Accepted planning authority revision observed while this projection was built.
    // This is distinct from the semantic task signature: the revision is the
    // optimistic-concurrency/version fact shared by operator surfaces.
    pub(crate) planning_revision: Option<i64>,
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
            planning_revision: None,
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
            planning_revision: None,
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
            planning_revision: None,
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
            planning_revision: None,
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

    pub fn with_planning_revision(mut self, planning_revision: Option<i64>) -> Self {
        self.planning_revision = planning_revision;
        self
    }

    pub fn workspace_present(&self) -> bool {
        self.workspace_present
    }

    pub fn workspace_status(&self) -> RuntimeWorkspaceStatus {
        self.workspace_status
    }

    pub fn planning_revision(&self) -> Option<i64> {
        self.planning_revision
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
    pub planning_revision: Option<i64>,
    pub task_authority: Option<TaskAuthorityDocument>,
}

impl ExecutionSnapshot {
    pub fn captures_path(path: &str) -> bool {
        canonical_active_planning_file_path(path).is_some()
    }
}

pub const RESULT_OUTPUT_FILE_PATH: &str = ".codex-exec-loop/planning/result-output.md";
pub const ACTIVE_PLANNING_FILE_PATHS: [&str; 1] = [RESULT_OUTPUT_FILE_PATH];

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSnapshotCapture {
    pub workspace_directory: String,
    pub state: TurnSnapshotCaptureState,
    pub parallel_slot_lease: Option<Box<ParallelModeSlotLeaseSnapshot>>,
}

impl TurnSnapshotCapture {
    pub fn ready(workspace_directory: impl Into<String>, snapshot: ExecutionSnapshot) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            state: TurnSnapshotCaptureState::Ready(Box::new(snapshot)),
            parallel_slot_lease: None,
        }
    }

    pub fn capture_failed(workspace_directory: impl Into<String>, message: String) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            state: TurnSnapshotCaptureState::CaptureFailed(message),
            parallel_slot_lease: None,
        }
    }

    pub fn with_parallel_slot_lease(
        mut self,
        parallel_slot_lease: Option<ParallelModeSlotLeaseSnapshot>,
    ) -> Self {
        self.parallel_slot_lease = parallel_slot_lease.map(Box::new);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnSnapshotCaptureState {
    Ready(Box<ExecutionSnapshot>),
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
pub struct ManualPromptCorrelation {
    pub request_id: u64,
    pub generation: u64,
    pub workspace_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptRequest {
    pub correlation: ManualPromptCorrelation,
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
        correlation: ManualPromptCorrelation,
        transcript_text: String,
        runtime_projection: Box<RuntimeProjection>,
        intake: Box<ManualPromptIntakeOutcome>,
    },
    BootstrapReviewRequired {
        correlation: ManualPromptCorrelation,
        transcript_text: String,
        runtime_projection: Box<RuntimeProjection>,
        review: ManualPlanningBootstrapReview,
    },
    BootstrapFailed {
        correlation: ManualPromptCorrelation,
        transcript_text: String,
        runtime_projection: Box<RuntimeProjection>,
        kind: ManualPlanningBootstrapFailureKind,
        reason: String,
    },
    Rejected {
        correlation: ManualPromptCorrelation,
        transcript_text: String,
        runtime_projection: Box<RuntimeProjection>,
        reason: String,
    },
}

impl ManualPromptOutcome {
    pub fn correlation(&self) -> &ManualPromptCorrelation {
        match self {
            ManualPromptOutcome::PromptReady { correlation, .. }
            | ManualPromptOutcome::BootstrapReviewRequired { correlation, .. }
            | ManualPromptOutcome::BootstrapFailed { correlation, .. }
            | ManualPromptOutcome::Rejected { correlation, .. } => correlation,
        }
    }

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

    pub fn transcript_text(&self) -> &str {
        match self {
            ManualPromptOutcome::PromptReady {
                transcript_text, ..
            }
            | ManualPromptOutcome::BootstrapReviewRequired {
                transcript_text, ..
            }
            | ManualPromptOutcome::BootstrapFailed {
                transcript_text, ..
            }
            | ManualPromptOutcome::Rejected {
                transcript_text, ..
            } => transcript_text,
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
    pub continuation_permit: PostTurnContinuationPermit,
}

impl PostTurnRequest {
    pub fn expected_parallel_slot_lease(&self) -> Option<&ParallelModeSlotLeaseSnapshot> {
        self.execution_snapshot_capture
            .as_ref()
            .and_then(|capture| capture.parallel_slot_lease.as_deref())
    }
}

/*
 * A post-turn request captures one automation generation plus a request-local validity token.
 * Operator policy changes and conversation/workspace supersession advance the shared gate, so
 * every older request loses continuation authority. A worker timeout invalidates only clones of
 * its own request token and cannot cancel a newer request captured in the same shared generation.
 */
#[derive(Clone, Default)]
pub struct PostTurnContinuationGate {
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    commit_serialization: std::sync::Arc<std::sync::Mutex<()>>,
}

impl PostTurnContinuationGate {
    pub fn capture(&self) -> PostTurnContinuationPermit {
        use std::sync::atomic::Ordering;

        PostTurnContinuationPermit {
            generation: self.generation.clone(),
            commit_serialization: self.commit_serialization.clone(),
            observed_generation: self.generation.load(Ordering::SeqCst),
            request_valid: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    pub fn advance(&self) -> u64 {
        use std::sync::atomic::Ordering;

        let _guard = self
            .commit_serialization
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }
}

#[derive(Clone)]
pub struct PostTurnContinuationPermit {
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    commit_serialization: std::sync::Arc<std::sync::Mutex<()>>,
    observed_generation: u64,
    request_valid: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl PostTurnContinuationPermit {
    pub fn is_current(&self) -> bool {
        use std::sync::atomic::Ordering;

        self.generation.load(Ordering::SeqCst) == self.observed_generation
            && self.request_valid.load(Ordering::SeqCst)
    }

    pub fn invalidate_if_current(&self) -> bool {
        use std::sync::atomic::Ordering;

        let _guard = self
            .commit_serialization
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.generation.load(Ordering::SeqCst) == self.observed_generation
            && self
                .request_valid
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
    }

    pub fn with_current<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        use std::sync::atomic::Ordering;

        // Keep this critical section limited to bounded host-side commits. An invalidation that
        // linearizes first prevents the operation; one that arrives later waits until the commit
        // finishes, so operator transitions never overlap an accepted authority mutation.
        let _guard = self
            .commit_serialization
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (self.generation.load(Ordering::SeqCst) == self.observed_generation
            && self.request_valid.load(Ordering::SeqCst))
        .then(operation)
    }
}

impl std::fmt::Debug for PostTurnContinuationPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostTurnContinuationPermit")
            .field("observed_generation", &self.observed_generation)
            .field("is_current", &self.is_current())
            .finish()
    }
}

impl PartialEq for PostTurnContinuationPermit {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.generation, &other.generation)
            && self.observed_generation == other.observed_generation
            && std::sync::Arc::ptr_eq(&self.request_valid, &other.request_valid)
    }
}

impl Eq for PostTurnContinuationPermit {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnContext {
    pub thread_id: String,
    pub planning_workspace_directory: String,
    pub latest_user_message: Option<String>,
    pub latest_main_reply: Option<String>,
    pub previous_handoff_task: Option<TaskHandoff>,
    pub current_runtime_projection: RuntimeProjection,
    pub parallel_mode_enabled: bool,
    pub parallel_automation_epoch_id: Option<u64>,
    pub planning_settlement_paused: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueMutationKind {
    Created,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningQueueMutationReceiptEntry {
    pub task_id: String,
    pub task_title: String,
    pub mutation_kind: PlanningQueueMutationKind,
    pub before_status: Option<TaskStatus>,
    pub after_status: TaskStatus,
    pub after_updated_at: String,
    pub unchanged_since_mutation: bool,
}

impl PlanningQueueMutationReceiptEntry {
    pub fn is_created_and_cancellable(&self) -> bool {
        self.mutation_kind == PlanningQueueMutationKind::Created
            && self.unchanged_since_mutation
            && matches!(self.after_status, TaskStatus::Ready | TaskStatus::Proposed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningQueueMutationReceipt {
    pub completed_turn_id: String,
    pub planning_revision: i64,
    pub entries: Vec<PlanningQueueMutationReceiptEntry>,
}

impl PlanningQueueMutationReceipt {
    pub fn created_entries(&self) -> impl Iterator<Item = &PlanningQueueMutationReceiptEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.mutation_kind == PlanningQueueMutationKind::Created)
    }

    pub fn cancellable_created_entries(
        &self,
    ) -> impl Iterator<Item = &PlanningQueueMutationReceiptEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.is_created_and_cancellable())
    }

    pub fn created_batch_is_cancellable(&self) -> bool {
        let created_count = self.created_entries().count();
        created_count > 0 && self.cancellable_created_entries().count() == created_count
    }
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
    pub queue_mutation_receipt: Option<PlanningQueueMutationReceipt>,
}

impl PostTurnProvenance {
    pub fn new(completed_turn_id: String) -> Self {
        Self {
            completed_turn_id,
            handoff_task: None,
            parallel_queue_signal: None,
            queue_mutation_receipt: None,
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

    pub fn with_queue_mutation_receipt(
        mut self,
        queue_mutation_receipt: Option<PlanningQueueMutationReceipt>,
    ) -> Self {
        self.queue_mutation_receipt = queue_mutation_receipt;
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
    pub runtime_projection_workspace_directory: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_projection_prompt_fragment_is_exposed_only_for_ready_projection() {
        let ready_projection = RuntimeProjection::ready(
            "Continue the active task".into(),
            "Queue head ready".into(),
            None,
        );
        assert_eq!(
            ready_projection.prompt_fragment(),
            Some("Continue the active task")
        );

        let invalid_projection = RuntimeProjection::invalid("planning workspace is invalid");
        assert_eq!(invalid_projection.prompt_fragment(), None);
    }

    #[test]
    fn continuation_gate_invalidates_only_previously_captured_permits() {
        let gate = PostTurnContinuationGate::default();
        let old_permit = gate.capture();
        assert!(old_permit.is_current());

        gate.advance();
        assert!(!old_permit.is_current());
        assert!(gate.capture().is_current());
    }

    #[test]
    fn continuation_permit_timeout_invalidation_is_atomic_and_idempotent() {
        let gate = PostTurnContinuationGate::default();
        let permit = gate.capture();
        let permit_clone = permit.clone();
        let newer_request = gate.capture();

        assert_eq!(permit, permit_clone);
        assert_ne!(permit, newer_request);
        assert!(permit.invalidate_if_current());
        assert!(!permit_clone.invalidate_if_current());
        assert!(!permit.is_current());
        assert!(permit_clone.with_current(|| ()).is_none());
        assert!(
            newer_request.is_current(),
            "one worker timeout must not invalidate another request captured from the same gate"
        );
        assert_eq!(newer_request.with_current(|| 7), Some(7));
        gate.advance();
        assert!(newer_request.with_current(|| ()).is_none());
        assert!(gate.capture().is_current());
    }

    #[test]
    fn continuation_invalidation_and_guarded_commit_are_linearly_ordered() {
        let gate = PostTurnContinuationGate::default();
        let permit = gate.capture();
        let worker_permit = permit.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            worker_permit.with_current(|| {
                entered_tx
                    .send(())
                    .expect("test should observe commit entry");
                release_rx
                    .recv()
                    .expect("test should release guarded commit");
            })
        });
        entered_rx
            .recv()
            .expect("guarded commit should acquire serialization first");

        let (advanced_tx, advanced_rx) = std::sync::mpsc::channel();
        let invalidator = std::thread::spawn(move || {
            let generation = gate.advance();
            advanced_tx
                .send(generation)
                .expect("test should observe invalidation");
        });
        assert!(
            advanced_rx
                .recv_timeout(std::time::Duration::from_millis(25))
                .is_err(),
            "invalidation must wait for a commit that linearized first"
        );

        release_tx
            .send(())
            .expect("test should release guarded commit");
        assert_eq!(
            worker.join().expect("guarded commit should finish"),
            Some(())
        );
        assert_eq!(
            advanced_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("invalidation should finish after the commit"),
            1
        );
        invalidator.join().expect("invalidator should finish");
        assert!(!permit.is_current());
        assert_eq!(permit.with_current(|| "must not run"), None);
    }
}
