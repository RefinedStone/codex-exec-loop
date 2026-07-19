use std::time::Duration;

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityMutationKind, PlanningTaskAuthorityMutationRecord,
};
use crate::application::service::parallel_mode::{
    ParallelModeOfficialCompletionReport, turn::ParallelModeTurnService,
};
use crate::application::service::planning::{
    PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON, PlanningPostTurnAutoFollowDecision,
    PlanningPostTurnAutoFollowRequest, PlanningPostTurnAutoFollowSkipReason,
    PlanningPostTurnQueueRefreshFinalizationEvent, PlanningPostTurnQueueRefreshFinalizationRequest,
    PlanningPostTurnQueueRefreshPreparation, PlanningPostTurnQueueRefreshPreparationRequest,
    PlanningPostTurnReconciliationRequest, PlanningQueueAuthoritySnapshot,
    PlanningRuntimeProjection, PlanningServices, PlanningTaskHandoff,
};
use crate::application::service::post_turn_decision::{
    PostTurnAutoFollowStopReason, PostTurnDecision as ApplicationPostTurnDecision,
    decide_parallel_official_completion_post_turn,
};
use crate::diagnostics::event_log;
use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;
use crate::domain::planning::{
    OriginSessionKind, PlanningQueueMutationKind, PlanningQueueMutationReceipt,
    PlanningQueueMutationReceiptEntry, PlanningWorkerPanelState as DomainPlanningWorkerPanelState,
    PlanningWorkerStatus as DomainPlanningWorkerStatus,
    PostTurnAutoFollowSkipReason as DomainPostTurnAutoFollowSkipReason,
    PostTurnContext as DomainPostTurnContext,
    PostTurnContinuationAction as DomainPostTurnContinuationAction,
    PostTurnExecution as DomainPostTurnExecution, PostTurnOutcome as DomainPostTurnOutcome,
    PostTurnPlanningRepairState as DomainPostTurnPlanningRepairState,
    PostTurnProvenance as DomainPostTurnProvenance,
    PostTurnQueuedPrompt as DomainPostTurnQueuedPrompt, PostTurnRequest as DomainPostTurnRequest,
    TaskDefinition, TurnSnapshotCaptureState,
};
use serde_json::json;

pub(crate) const POST_TURN_EVALUATION_TIMEOUT: Duration = Duration::from_secs(600);
const POST_TURN_CANCELLATION_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(2);
#[path = "post_turn_evaluation/logging.rs"]
mod logging;
#[path = "post_turn_evaluation/official_completion.rs"]
mod official_completion;
#[path = "post_turn_evaluation/planning_worker_panel.rs"]
mod planning_worker_panel;
#[path = "post_turn_evaluation/repair.rs"]
mod repair;
use self::planning_worker_panel::planning_worker_queue_summary;
use logging::{
    PostTurnWorkerLogContext, planning_worker_refresh_skipped_detail, post_turn_action_decision,
    post_turn_action_log_detail, post_turn_event_detail,
};

// Post-turn evaluation is the handoff between a completed Codex turn and the
// planning/parallel-mode continuation that may schedule the next prompt. The
// executor owns a cloned service set so production can run it off the UI thread
// while tests run the same sequence synchronously.
#[derive(Clone)]
pub struct PostTurnEvaluationService {
    planning_feature: PlanningServices,
    parallel_mode_turn_service: ParallelModeTurnService,
}

impl PostTurnEvaluationService {
    pub fn new(
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
    ) -> Self {
        Self {
            planning_feature,
            parallel_mode_turn_service,
        }
    }

    pub fn evaluate(&self, request: PostTurnEvaluationRequest) -> PostTurnEvaluationExecution {
        let executor = PostTurnEvaluationExecutor::new(
            self.planning_feature.clone(),
            self.parallel_mode_turn_service.clone(),
            request.planning_worker_panel_state.clone(),
        );
        executor.run(&request.context, &request)
    }

    pub fn evaluate_with_timeout(
        &self,
        request: PostTurnEvaluationRequest,
        timeout: Duration,
    ) -> PostTurnEvaluationExecution {
        let (execution_tx, execution_rx) = std::sync::mpsc::channel();
        let timeout_context = request.context.clone();
        let timeout_request = request.clone();
        let service = self.clone();
        let evaluator = std::thread::spawn(move || {
            let execution = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                service.evaluate(request)
            }))
            .map_err(|_| "post-turn evaluation worker panicked".to_string());
            let _ = execution_tx.send(execution);
        });
        match execution_rx.recv_timeout(timeout) {
            Ok(Ok(execution)) => {
                let _ = evaluator.join();
                execution
            }
            Ok(Err(message)) => {
                timeout_request.continuation_permit.invalidate_if_current();
                let _ = evaluator.join();
                post_turn_evaluation_failure_execution(&timeout_context, &timeout_request, message)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                timeout_request.continuation_permit.invalidate_if_current();
                settle_post_turn_evaluator(evaluator, POST_TURN_CANCELLATION_SETTLEMENT_TIMEOUT);
                post_turn_evaluation_timeout_execution(&timeout_context, &timeout_request, timeout)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                timeout_request.continuation_permit.invalidate_if_current();
                let _ = evaluator.join();
                post_turn_evaluation_failure_execution(
                    &timeout_context,
                    &timeout_request,
                    "post-turn evaluation worker disconnected before returning a result"
                        .to_string(),
                )
            }
        }
    }
}

fn settle_post_turn_evaluator(worker: std::thread::JoinHandle<()>, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while !worker.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if worker.is_finished() {
        let _ = worker.join();
    }
}

// Post-turn evaluation is the handoff between a completed Codex turn and the
// planning/parallel-mode continuation that may schedule the next prompt.
pub type PostTurnEvaluationRequest = DomainPostTurnRequest;
pub type PostTurnEvaluationContext = DomainPostTurnContext;
pub type PlanningWorkerStatus = DomainPlanningWorkerStatus;
pub type PlanningWorkerPanelState = DomainPlanningWorkerPanelState;
pub type PostTurnEvaluationOutcome = DomainPostTurnOutcome;
pub type PostTurnPlanningRepairState = DomainPostTurnPlanningRepairState;
pub type PostTurnEvaluationProvenance = DomainPostTurnProvenance;
pub type PostTurnQueuedPrompt = DomainPostTurnQueuedPrompt;
pub type PostTurnContinuationAction = DomainPostTurnContinuationAction;
pub type PostTurnAutoFollowSkipReason = DomainPostTurnAutoFollowSkipReason;

fn post_turn_log_context<'a>(
    context: &'a PostTurnEvaluationContext,
    request: &'a PostTurnEvaluationRequest,
) -> PostTurnWorkerLogContext<'a> {
    PostTurnWorkerLogContext::new(
        context.thread_id.as_str(),
        request.completed_turn_id.as_str(),
        request.workspace_directory.as_str(),
    )
}

fn previous_handoff_task(context: &PostTurnEvaluationContext) -> Option<&PlanningTaskHandoff> {
    context.previous_handoff_task.as_ref()
}

#[derive(Debug, Clone)]
struct HiddenPlanningRepairOutcome {
    runtime_projection: PlanningRuntimeProjection,
    resolved: bool,
}
#[derive(Debug, Clone)]
struct PlanningQueueRefreshOutcome {
    runtime_projection: PlanningRuntimeProjection,
}

fn queue_receipt_baseline(
    planning_feature: &PlanningServices,
    request: &PostTurnEvaluationRequest,
) -> Option<PlanningQueueAuthoritySnapshot> {
    let captured = request
        .execution_snapshot_capture
        .as_ref()
        .filter(|capture| capture.workspace_directory == request.workspace_directory)
        .and_then(|capture| match &capture.state {
            TurnSnapshotCaptureState::Ready(snapshot) => snapshot
                .planning_revision
                .zip(snapshot.task_authority.clone())
                .map(
                    |(planning_revision, task_authority)| PlanningQueueAuthoritySnapshot {
                        planning_revision,
                        tasks: task_authority.tasks,
                    },
                ),
            TurnSnapshotCaptureState::CaptureFailed(_) => None,
        });
    captured.or_else(|| {
        planning_feature
            .queue
            .load_authority_snapshot(&request.workspace_directory)
            .ok()
    })
}

fn build_queue_mutation_receipt(
    completed_thread_id: &str,
    completed_turn_id: &str,
    before_planning_revision: i64,
    mut mutations: Vec<PlanningTaskAuthorityMutationRecord>,
    after: PlanningQueueAuthoritySnapshot,
) -> PlanningQueueMutationReceipt {
    let mut entries = Vec::new();
    let mut entry_indexes = std::collections::BTreeMap::new();
    let mut expected_tasks = std::collections::BTreeMap::<String, TaskDefinition>::new();
    let mut attributed_task_ids = std::collections::BTreeSet::new();
    let mut contaminated_task_ids = std::collections::BTreeSet::new();
    mutations.sort_by_key(|mutation| mutation.planning_revision);
    for mutation in mutations {
        if mutation.planning_revision <= before_planning_revision {
            continue;
        }
        if !task_mutation_belongs_to_turn(&mutation, completed_thread_id, completed_turn_id) {
            if attributed_task_ids.contains(&mutation.task_id) {
                contaminated_task_ids.insert(mutation.task_id);
            }
            continue;
        }
        attributed_task_ids.insert(mutation.task_id.clone());
        let (Some(after_status), Some(after_updated_at), Some(after_task)) = (
            mutation.after_status,
            mutation.after_updated_at,
            mutation.after_task,
        ) else {
            contaminated_task_ids.insert(mutation.task_id);
            continue;
        };
        expected_tasks.insert(mutation.task_id.clone(), after_task);
        if let Some(index) = entry_indexes.get(&mutation.task_id).copied() {
            let entry: &mut PlanningQueueMutationReceiptEntry = &mut entries[index];
            entry.task_title = mutation.task_title;
            entry.after_status = after_status;
            entry.after_updated_at = after_updated_at;
        } else {
            entry_indexes.insert(mutation.task_id.clone(), entries.len());
            entries.push(PlanningQueueMutationReceiptEntry {
                task_id: mutation.task_id,
                task_title: mutation.task_title,
                mutation_kind: match mutation.mutation_kind {
                    PlanningTaskAuthorityMutationKind::Created => {
                        PlanningQueueMutationKind::Created
                    }
                    PlanningTaskAuthorityMutationKind::Updated => {
                        PlanningQueueMutationKind::Updated
                    }
                    PlanningTaskAuthorityMutationKind::Removed => unreachable!(),
                },
                before_status: mutation.before_status,
                after_status,
                after_updated_at,
                unchanged_since_mutation: false,
            });
        }
    }
    for entry in &mut entries {
        entry.unchanged_since_mutation = !contaminated_task_ids.contains(&entry.task_id)
            && expected_tasks.get(&entry.task_id).is_some_and(|expected| {
                after
                    .tasks
                    .iter()
                    .any(|task| task.id == entry.task_id && task == expected)
            });
    }
    PlanningQueueMutationReceipt {
        completed_turn_id: completed_turn_id.to_string(),
        planning_revision: after.planning_revision,
        entries,
    }
}

fn task_mutation_belongs_to_turn(
    mutation: &PlanningTaskAuthorityMutationRecord,
    completed_thread_id: &str,
    completed_turn_id: &str,
) -> bool {
    let completed_thread_id = completed_thread_id.trim();
    let completed_turn_id = completed_turn_id.trim();
    if completed_thread_id.is_empty() || completed_turn_id.is_empty() {
        return false;
    }
    match mutation.provenance.origin_session_kind {
        Some(OriginSessionKind::Main) => {
            provenance_id_matches(
                mutation.provenance.thread_id.as_deref(),
                completed_thread_id,
            ) && provenance_id_matches(mutation.provenance.turn_id.as_deref(), completed_turn_id)
        }
        Some(OriginSessionKind::Planner) => {
            provenance_id_matches(
                mutation.provenance.parent_thread_id.as_deref(),
                completed_thread_id,
            ) && provenance_id_matches(
                mutation.provenance.parent_turn_id.as_deref(),
                completed_turn_id,
            )
        }
        Some(
            OriginSessionKind::ManualIntake
            | OriginSessionKind::Parallel
            | OriginSessionKind::System,
        ) => false,
        None => false,
    }
}

fn provenance_id_matches(actual: Option<&str>, expected: &str) -> bool {
    actual.is_some_and(|actual| actual.trim() == expected)
}
#[derive(Debug, Clone)]
struct OfficialCompletionRefreshOutcome {
    runtime_projection_workspace_directory: String,
    runtime_projection: PlanningRuntimeProjection,
    runtime_notices: Vec<String>,
}

impl OfficialCompletionRefreshOutcome {
    fn for_turn_workspace(
        request: &PostTurnEvaluationRequest,
        runtime_projection: PlanningRuntimeProjection,
        runtime_notices: Vec<String>,
    ) -> Self {
        Self {
            runtime_projection_workspace_directory: request.workspace_directory.clone(),
            runtime_projection,
            runtime_notices,
        }
    }

    fn for_planning_workspace(
        planning_workspace_directory: &str,
        runtime_projection: PlanningRuntimeProjection,
        runtime_notices: Vec<String>,
    ) -> Self {
        Self {
            runtime_projection_workspace_directory: planning_workspace_directory.to_string(),
            runtime_projection,
            runtime_notices,
        }
    }
}
#[derive(Debug)]
enum OfficialCompletionCapture {
    NotApplicable,
    Captured(Box<ParallelModeOfficialCompletionReport>),
    Failed { detail: String },
}
#[derive(Debug, Clone)]
struct PostTurnDecision {
    action: PostTurnContinuationAction,
    provenance: PostTurnEvaluationProvenance,
    operator_alerts: Vec<OperatorAlert>,
}
impl PostTurnDecision {
    fn from_action(completed_turn_id: String, action: PostTurnContinuationAction) -> Self {
        let operator_alerts = operator_alerts_for_action(&action);
        Self {
            action,
            provenance: PostTurnEvaluationProvenance::new(completed_turn_id),
            operator_alerts,
        }
    }

    fn from_action_with_provenance(
        action: PostTurnContinuationAction,
        provenance: PostTurnEvaluationProvenance,
    ) -> Self {
        let operator_alerts = operator_alerts_for_action(&action);
        Self {
            action,
            provenance,
            operator_alerts,
        }
    }

    fn from_application_decision(
        completed_turn_id: String,
        decision: ApplicationPostTurnDecision,
    ) -> Self {
        Self {
            action: PostTurnContinuationAction::SkipAutoFollow {
                reason: auto_follow_skip_reason_from_post_turn(decision.auto_follow_stop_reason),
            },
            provenance: PostTurnEvaluationProvenance::new(completed_turn_id)
                .with_parallel_queue_signal(decision.parallel_queue_signal),
            operator_alerts: decision.operator_alerts,
        }
    }
}
pub type PostTurnEvaluationExecution = DomainPostTurnExecution;
#[derive(Clone)]
struct PostTurnEvaluationExecutor {
    planning_feature: PlanningServices,
    parallel_mode_turn_service: ParallelModeTurnService,
    planning_worker_panel_state: PlanningWorkerPanelState,
}
impl PostTurnEvaluationExecutor {
    fn new(
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        planning_worker_panel_state: PlanningWorkerPanelState,
    ) -> Self {
        Self {
            planning_feature,
            parallel_mode_turn_service,
            planning_worker_panel_state,
        }
    }

    // The execution order is deliberate: protect planning files first, repair
    // only when continuation can act on the result, finish official parallel
    // completions before planning queue refreshes, then derive the action
    // from the final runtime projection.
    #[tracing::instrument(level = "trace", skip(self, context, request))]
    fn run(
        mut self,
        context: &PostTurnEvaluationContext,
        request: &PostTurnEvaluationRequest,
    ) -> PostTurnEvaluationExecution {
        let planning_workspace_directory = context.planning_workspace_directory.as_str();
        let mut receipt_baseline = (!context.planning_settlement_paused
            && request.continuation_permit.is_current())
        .then(|| queue_receipt_baseline(&self.planning_feature, request))
        .flatten();
        event_log::emit_lazy("post_turn_evaluation_started", || {
            post_turn_event_detail(
                post_turn_log_context(context, request),
                "post_turn",
                "started",
                Some("evaluate"),
                Some(&context.current_runtime_projection),
                [
                    (
                        "planning_workspace_directory",
                        json!(planning_workspace_directory),
                    ),
                    (
                        "changed_planning_file_count",
                        json!(request.changed_planning_file_paths.len()),
                    ),
                    (
                        "post_turn_continuation_paused",
                        json!(context.continuation_paused),
                    ),
                ],
            )
        });
        let reconciliation_outcome = self.planning_feature.runtime.reconcile_post_turn(
            PlanningPostTurnReconciliationRequest {
                workspace_directory: &request.workspace_directory,
                completed_turn_id: &request.completed_turn_id,
                changed_planning_file_paths: &request.changed_planning_file_paths,
                execution_snapshot_capture: request.execution_snapshot_capture.as_ref(),
                current_runtime_projection: &context.current_runtime_projection,
            },
        );
        let reconciliation_result = reconciliation_outcome.reconciliation_result;
        let mut runtime_notices = reconciliation_result.notices.clone();
        let mut runtime_projection = reconciliation_outcome.runtime_projection;
        let mut runtime_projection_workspace_directory = request.workspace_directory.clone();
        let planning_settlement_enabled =
            !context.planning_settlement_paused && request.continuation_permit.is_current();
        if receipt_baseline.is_none() && planning_settlement_enabled {
            receipt_baseline = self
                .planning_feature
                .queue
                .load_authority_snapshot(&request.workspace_directory)
                .ok();
        }
        let official_completion_capture = request
            .continuation_permit
            .with_current(|| self.begin_official_completion_if_needed(context, request))
            .unwrap_or(OfficialCompletionCapture::NotApplicable);
        let official_completion_capture_failed = matches!(
            official_completion_capture,
            OfficialCompletionCapture::Failed { .. }
        );
        let official_completion_report = match &official_completion_capture {
            OfficialCompletionCapture::Captured(report) => Some(report.as_ref()),
            OfficialCompletionCapture::NotApplicable | OfficialCompletionCapture::Failed { .. } => {
                None
            }
        };
        if let OfficialCompletionCapture::Failed { detail } = &official_completion_capture {
            runtime_notices.push(detail.clone());
            runtime_projection = PlanningRuntimeProjection::invalid(detail.clone());
        }
        if request.continuation_permit.is_current()
            && !official_completion_capture_failed
            && (planning_settlement_enabled || official_completion_report.is_some())
            && let Some(repair_request) = reconciliation_result.repair_request.as_ref()
        {
            let repair_outcome = self.run_hidden_planning_repairs(
                context.thread_id.as_str(),
                &request.workspace_directory,
                &request.completed_turn_id,
                repair_request,
                previous_handoff_task(context),
                &request.continuation_permit,
            );
            runtime_projection = repair_outcome.runtime_projection;
        }
        let handled_parallel_completion =
            if let Some(completion_report) = official_completion_report {
                let official_completion_outcome = self.run_official_completion_refresh(
                    context,
                    request,
                    planning_workspace_directory,
                    &runtime_projection,
                    completion_report,
                );
                runtime_notices.extend(official_completion_outcome.runtime_notices);
                runtime_projection = official_completion_outcome.runtime_projection;
                runtime_projection_workspace_directory =
                    official_completion_outcome.runtime_projection_workspace_directory;
                true
            } else {
                false
            };
        if !handled_parallel_completion
            && !official_completion_capture_failed
            && planning_settlement_enabled
            && request.continuation_permit.is_current()
        {
            let refresh_outcome =
                self.run_planning_queue_refresh(context, request, runtime_projection.clone());
            runtime_projection = refresh_outcome.runtime_projection;
        }
        let mut post_turn_decision = if !request.continuation_permit.is_current() {
            PostTurnDecision::from_action(
                request.completed_turn_id.clone(),
                PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PostTurnContinuationPaused,
                },
            )
        } else if official_completion_capture_failed {
            PostTurnDecision::from_action(
                request.completed_turn_id.clone(),
                PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
                },
            )
        } else if handled_parallel_completion {
            PostTurnDecision::from_application_decision(
                request.completed_turn_id.clone(),
                decide_parallel_official_completion_post_turn(&runtime_projection),
            )
        } else {
            self.auto_follow_decision_from_projection(context, request, &runtime_projection)
        };
        if request.continuation_permit.is_current()
            && let Some(receipt_baseline) = receipt_baseline
            && let Ok(receipt_final) = self
                .planning_feature
                .queue
                .load_authority_snapshot(&request.workspace_directory)
            && let Ok(receipt_mutations) = self.planning_feature.queue.load_authority_mutations(
                &request.workspace_directory,
                receipt_baseline.planning_revision,
                receipt_final.planning_revision,
            )
        {
            let receipt = build_queue_mutation_receipt(
                &context.thread_id,
                &request.completed_turn_id,
                receipt_baseline.planning_revision,
                receipt_mutations,
                receipt_final,
            );
            if !receipt.entries.is_empty() {
                post_turn_decision.provenance.queue_mutation_receipt = Some(receipt);
            }
        }
        event_log::emit_lazy("post_turn_evaluation_completed", || {
            post_turn_event_detail(
                post_turn_log_context(context, request),
                "post_turn",
                "completed",
                Some(post_turn_action_decision(&post_turn_decision.action)),
                Some(&runtime_projection),
                [
                    (
                        "handled_parallel_completion",
                        json!(handled_parallel_completion),
                    ),
                    ("runtime_notices_count", json!(runtime_notices.len())),
                    (
                        "operator_alerts_count",
                        json!(post_turn_decision.operator_alerts.len()),
                    ),
                    (
                        "parallel_queue_signal",
                        json!(
                            post_turn_decision
                                .provenance
                                .parallel_queue_signal
                                .map(|signal| format!("{signal:?}"))
                        ),
                    ),
                    (
                        "action",
                        post_turn_action_log_detail(
                            &post_turn_decision.action,
                            &post_turn_decision.provenance,
                        ),
                    ),
                ],
            )
        });

        PostTurnEvaluationExecution {
            thread_id: context.thread_id.clone(),
            completed_turn_id: request.completed_turn_id.clone(),
            runtime_projection_workspace_directory,
            evaluation: PostTurnEvaluationOutcome {
                provenance: post_turn_decision.provenance,
                runtime_projection,
                planning_repair_state: None,
                runtime_notices,
                action: post_turn_decision.action,
                operator_alerts: post_turn_decision.operator_alerts,
            },
            planning_worker_panel_state: self.planning_worker_panel_state,
        }
    }
    // Planning queue refresh is the normal auto-follow path after a main-session
    // reply. It skips non-ready workspaces, honors queue-idle policy, records
    // worker panel state, and promotes justified proposals into the executable
    // queue when no actionable head exists yet.
    #[tracing::instrument(level = "trace", skip(self, context, request, current_projection))]
    fn run_planning_queue_refresh(
        &mut self,
        context: &PostTurnEvaluationContext,
        request: &PostTurnEvaluationRequest,
        current_projection: PlanningRuntimeProjection,
    ) -> PlanningQueueRefreshOutcome {
        let preparation = self
            .planning_feature
            .worker
            .prepare_post_turn_queue_refresh(PlanningPostTurnQueueRefreshPreparationRequest {
                workspace_directory: &request.workspace_directory,
                parent_thread_id: Some(context.thread_id.as_str())
                    .filter(|thread_id| !thread_id.trim().is_empty()),
                completed_turn_id: &request.completed_turn_id,
                latest_user_message: context.latest_user_message.as_deref(),
                latest_main_reply: context.latest_main_reply.as_deref(),
                previous_handoff_task: previous_handoff_task(context),
                current_runtime_projection: &current_projection,
            });
        let prepared = match preparation {
            PlanningPostTurnQueueRefreshPreparation::Skipped(skipped) => {
                event_log::emit_lazy("planning_worker_refresh_skipped", || {
                    planning_worker_refresh_skipped_detail(
                        post_turn_log_context(context, request),
                        skipped.reason.log_label(),
                        &skipped.runtime_projection,
                    )
                });
                return PlanningQueueRefreshOutcome {
                    runtime_projection: skipped.runtime_projection,
                };
            }
            PlanningPostTurnQueueRefreshPreparation::Ready(prepared) => prepared,
        };
        if !request.continuation_permit.is_current() {
            return PlanningQueueRefreshOutcome {
                runtime_projection: current_projection,
            };
        }
        event_log::emit_lazy("planning_worker_refresh_started", || {
            post_turn_event_detail(
                post_turn_log_context(context, request),
                "refresh",
                "started",
                Some("run_worker"),
                Some(&current_projection),
                [
                    ("mode", json!(prepared.mode_label())),
                    (
                        "latest_main_reply_chars",
                        json!(prepared.latest_main_reply_char_count()),
                    ),
                    (
                        "has_latest_user_message",
                        json!(prepared.has_latest_user_message()),
                    ),
                    (
                        "has_previous_handoff",
                        json!(prepared.has_previous_handoff()),
                    ),
                    (
                        "worker_prompt_chars",
                        json!(prepared.worker_prompt().chars().count()),
                    ),
                ],
            )
        });
        self.record_planning_worker_running(
            PlanningWorkerStatus::RefreshRunning,
            prepared.panel_operation_label(),
            prepared.worker_prompt().to_string(),
        );
        let worker_outcome = self
            .planning_feature
            .worker
            .refresh_prepared_queue_from_reply_with_permit(
                prepared.as_ref(),
                &request.continuation_permit,
            );
        let outcome = match worker_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                let detail = if prepared.is_queue_idle_derivation() {
                    format!("planning worker queue-idle derivation failed: {error}")
                } else {
                    format!("planning worker refresh failed: {error}")
                };
                let invalid_projection = PlanningRuntimeProjection::invalid(
                    PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON,
                );
                event_log::emit_lazy("planning_worker_refresh_failed", || {
                    post_turn_event_detail(
                        post_turn_log_context(context, request),
                        "refresh",
                        "worker_failed",
                        Some("block_auto_follow"),
                        Some(&invalid_projection),
                        [
                            ("mode", json!(prepared.mode_label())),
                            ("error", json!(error.to_string())),
                            (
                                "invalid_reason",
                                json!(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON),
                            ),
                        ],
                    )
                });
                self.record_planning_worker_failure(
                    PlanningWorkerStatus::RefreshFailed,
                    &detail,
                    &invalid_projection,
                );
                return PlanningQueueRefreshOutcome {
                    runtime_projection: invalid_projection,
                };
            }
        };

        self.record_planning_worker_outcome(PlanningWorkerStatus::RefreshSucceeded, &outcome);
        event_log::emit_lazy("planning_worker_refresh_succeeded", || {
            post_turn_event_detail(
                post_turn_log_context(context, request),
                "refresh",
                "worker_succeeded",
                Some("apply_outcome"),
                Some(&outcome.runtime_projection),
                [
                    ("mode", json!(prepared.mode_label())),
                    ("repair_requested", json!(outcome.repair_request.is_some())),
                    (
                        "task_authority_changed",
                        json!(outcome.task_authority_changed),
                    ),
                    ("notices_count", json!(outcome.notices.len())),
                    (
                        "has_worker_summary",
                        json!(outcome.worker_summary.is_some()),
                    ),
                    (
                        "has_rejected_summary",
                        json!(outcome.rejected_summary.is_some()),
                    ),
                ],
            )
        });
        let mut runtime_projection = outcome.runtime_projection.clone();
        if let Some(repair_request) = outcome.repair_request.as_ref() {
            let repair_outcome = self.run_hidden_planning_repairs(
                context.thread_id.as_str(),
                &request.workspace_directory,
                &request.completed_turn_id,
                repair_request,
                previous_handoff_task(context),
                &request.continuation_permit,
            );
            runtime_projection = if repair_outcome.resolved {
                repair_outcome.runtime_projection
            } else {
                event_log::emit_lazy("planning_worker_refresh_repair_unresolved", || {
                    post_turn_event_detail(
                        post_turn_log_context(context, request),
                        "repair",
                        "unresolved_after_refresh",
                        Some("block_auto_follow"),
                        Some(&repair_outcome.runtime_projection),
                        [
                            (
                                "repair_failure_summary",
                                json!(repair_request.failure_summary.as_str()),
                            ),
                            (
                                "invalid_reason",
                                json!(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON),
                            ),
                        ],
                    )
                });
                PlanningRuntimeProjection::invalid(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON)
            };
        }
        let Some(finalization) = self
            .planning_feature
            .worker
            .finalize_post_turn_queue_refresh_with_permit(
                PlanningPostTurnQueueRefreshFinalizationRequest {
                    workspace_directory: &request.workspace_directory,
                    parent_thread_id: Some(context.thread_id.as_str())
                        .filter(|thread_id| !thread_id.trim().is_empty()),
                    completed_turn_id: &request.completed_turn_id,
                    previous_handoff_task: previous_handoff_task(context),
                    previous_runtime_projection: &context.current_runtime_projection,
                    refreshed_runtime_projection: &runtime_projection,
                    queue_idle_derivation: prepared.is_queue_idle_derivation(),
                },
                &request.continuation_permit,
            )
        else {
            return PlanningQueueRefreshOutcome {
                runtime_projection: current_projection,
            };
        };
        runtime_projection = finalization.runtime_projection;
        for event in finalization.events {
            match event {
                PlanningPostTurnQueueRefreshFinalizationEvent::ProposalPromotionCompleted {
                    outcome: promotion_outcome,
                } => {
                    event_log::emit_lazy("planning_worker_proposal_promotion_completed", || {
                        post_turn_event_detail(
                            post_turn_log_context(context, request),
                            "proposal_promotion",
                            "completed",
                            promotion_outcome
                                .promoted_task_title
                                .as_ref()
                                .map(|_| "promoted")
                                .or(Some("no_promotable_proposal")),
                            Some(&promotion_outcome.runtime_projection),
                            [(
                                "promoted_task_title",
                                json!(promotion_outcome.promoted_task_title.as_deref()),
                            )],
                        )
                    });
                    self.planning_worker_panel_state.last_queue_summary =
                        planning_worker_queue_summary(&promotion_outcome.runtime_projection);
                    self.planning_worker_panel_state.last_host_detail =
                        promotion_outcome.promoted_task_title.map(|title| {
                            format!(
                                "host promoted top follow-up proposal into the executable queue: {title}"
                            )
                        });
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::ProposalPromotionFailed {
                    detail,
                    runtime_projection: invalid_projection,
                } => {
                    event_log::emit_lazy("planning_worker_proposal_promotion_failed", || {
                        post_turn_event_detail(
                            post_turn_log_context(context, request),
                            "proposal_promotion",
                            "failed",
                            Some("block_auto_follow"),
                            Some(&invalid_projection),
                            [
                                ("error", json!(detail.as_str())),
                                (
                                    "invalid_reason",
                                    json!(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON),
                                ),
                            ],
                        )
                    });
                    self.record_planning_worker_failure(
                        PlanningWorkerStatus::RefreshFailed,
                        &detail,
                        &invalid_projection,
                    );
                    return PlanningQueueRefreshOutcome {
                        runtime_projection: invalid_projection,
                    };
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::QueueIdleDerivationEmpty {
                    detail,
                } => {
                    self.planning_worker_panel_state.last_host_detail = Some(detail);
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::RepeatedQueueHead {
                    detail,
                    runtime_projection: guard_projection,
                } => {
                    self.planning_worker_panel_state.status = PlanningWorkerStatus::RefreshFailed;
                    self.planning_worker_panel_state.last_host_detail = Some(detail.clone());
                    event_log::emit_lazy(
                        "planning_worker_refresh_paused_repeated_queue_head",
                        || {
                            post_turn_event_detail(
                                post_turn_log_context(context, request),
                                "refresh",
                                "repeated_queue_head_guard",
                                Some("pause_auto_follow"),
                                Some(&guard_projection),
                                [("pause_reason", json!(detail.as_str()))],
                            )
                        },
                    );
                }
            }
        }

        PlanningQueueRefreshOutcome { runtime_projection }
    }

    // The final action is always derived from the latest runtime projection. Explicit
    // pause states and queue-idle stop policy win before the conversation model
    // is allowed to enqueue another prompt.
    #[tracing::instrument(level = "trace", skip(self, context, request, runtime_projection))]
    fn auto_follow_decision_from_projection(
        &self,
        context: &PostTurnEvaluationContext,
        request: &PostTurnEvaluationRequest,
        runtime_projection: &PlanningRuntimeProjection,
    ) -> PostTurnDecision {
        match self.planning_feature.runtime.decide_post_turn_auto_follow(
            PlanningPostTurnAutoFollowRequest {
                continuation_paused: context.continuation_paused,
                can_queue_next: context.can_queue_next,
                latest_agent_message: context.latest_main_reply.as_deref(),
                stop_keyword: context.stop_keyword.as_str(),
                stop_keyword_matched: context.stop_keyword_matched,
                no_file_changes_stop_matched: context.no_file_changes_stop_matched,
                runtime_projection,
            },
        ) {
            PlanningPostTurnAutoFollowDecision::QueuePrompt(queued_prompt) => {
                event_log::emit_lazy("auto_follow_decision", || {
                    post_turn_event_detail(
                        post_turn_log_context(context, request),
                        "auto_follow",
                        "decision",
                        Some("queue"),
                        Some(runtime_projection),
                        [
                            ("mode_label", json!(context.mode_label.as_str())),
                            ("prompt_chars", json!(queued_prompt.prompt.chars().count())),
                            (
                                "transcript_text_chars",
                                json!(queued_prompt.transcript_text.chars().count()),
                            ),
                            (
                                "handoff_task_id",
                                json!(
                                    queued_prompt
                                        .handoff_task
                                        .as_ref()
                                        .map(|task| task.task_id.as_str())
                                ),
                            ),
                        ],
                    )
                });
                PostTurnDecision::from_action_with_provenance(
                    PostTurnContinuationAction::QueueAutoPrompt(Box::new(PostTurnQueuedPrompt {
                        prompt: queued_prompt.prompt,
                        mode_label: context.mode_label.clone(),
                        transcript_text: queued_prompt.transcript_text,
                    })),
                    PostTurnEvaluationProvenance::new(request.completed_turn_id.clone())
                        .with_handoff_task(queued_prompt.handoff_task)
                        .with_parallel_queue_signal(
                            context
                                .parallel_mode_enabled
                                .then_some(ParallelModePostTurnQueueSignal::AutoFollowQueued),
                        ),
                )
            }
            PlanningPostTurnAutoFollowDecision::Skip(reason) => {
                let reason = auto_follow_skip_reason_from_planning(reason);
                event_log::emit_lazy("auto_follow_decision", || {
                    post_turn_event_detail(
                        post_turn_log_context(context, request),
                        "auto_follow",
                        "decision",
                        Some("skip"),
                        Some(runtime_projection),
                        [("reason", json!(format!("{:?}", reason)))],
                    )
                });
                PostTurnDecision::from_action(
                    request.completed_turn_id.clone(),
                    PostTurnContinuationAction::SkipAutoFollow { reason },
                )
            }
        }
    }
}

fn auto_follow_skip_reason_from_post_turn(
    reason: PostTurnAutoFollowStopReason,
) -> PostTurnAutoFollowSkipReason {
    match reason {
        PostTurnAutoFollowStopReason::PlanningQueueDrained => {
            PostTurnAutoFollowSkipReason::PlanningQueueDrained
        }
        PostTurnAutoFollowStopReason::ParallelSessionCompleted => {
            PostTurnAutoFollowSkipReason::ParallelSessionCompleted
        }
    }
}

fn auto_follow_skip_reason_from_planning(
    reason: PlanningPostTurnAutoFollowSkipReason,
) -> PostTurnAutoFollowSkipReason {
    match reason {
        PlanningPostTurnAutoFollowSkipReason::PostTurnContinuationPaused => {
            PostTurnAutoFollowSkipReason::PostTurnContinuationPaused
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueDrained => {
            PostTurnAutoFollowSkipReason::PlanningQueueDrained
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop => {
            PostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop
        }
        PlanningPostTurnAutoFollowSkipReason::LimitReached => {
            PostTurnAutoFollowSkipReason::LimitReached
        }
        PlanningPostTurnAutoFollowSkipReason::NoAgentReply => {
            PostTurnAutoFollowSkipReason::NoAgentReply
        }
        PlanningPostTurnAutoFollowSkipReason::StopKeywordMatched => {
            PostTurnAutoFollowSkipReason::StopKeywordMatched
        }
        PlanningPostTurnAutoFollowSkipReason::NoFileChanges => {
            PostTurnAutoFollowSkipReason::NoFileChanges
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningBlocked => {
            PostTurnAutoFollowSkipReason::PlanningBlocked
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueHeadRequired => {
            PostTurnAutoFollowSkipReason::PlanningQueueHeadRequired
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead => {
            PostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead
        }
    }
}

fn operator_alerts_for_action(action: &PostTurnContinuationAction) -> Vec<OperatorAlert> {
    match action {
        PostTurnContinuationAction::SkipAutoFollow {
            reason: PostTurnAutoFollowSkipReason::PlanningQueueDrained,
        } => vec![OperatorAlert::planning_queue_drained()],
        PostTurnContinuationAction::QueueAutoPrompt(_)
        | PostTurnContinuationAction::SkipAutoFollow { .. } => Vec::new(),
    }
}

// Timeout fallback reports a failed refresh while returning control to the main
// session. The shared generation is invalidated first, so a background worker
// that finishes later cannot apply hidden planning results or enqueue a prompt.
fn post_turn_evaluation_timeout_execution(
    context: &PostTurnEvaluationContext,
    request: &PostTurnEvaluationRequest,
    timeout: Duration,
) -> PostTurnEvaluationExecution {
    let message = format!(
        "post-turn planning worker evaluation timed out after {} seconds",
        timeout.as_secs()
    );
    PostTurnEvaluationExecution {
        thread_id: context.thread_id.clone(),
        completed_turn_id: request.completed_turn_id.clone(),
        runtime_projection_workspace_directory: request.workspace_directory.clone(),
        evaluation: PostTurnEvaluationOutcome {
            provenance: PostTurnEvaluationProvenance::new(request.completed_turn_id.clone()),
            runtime_projection: PlanningRuntimeProjection::invalid(message.clone()),
            planning_repair_state: None,
            runtime_notices: vec![message.clone()],
            action: PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::PostTurnEvaluationTimedOut,
            },
            operator_alerts: Vec::new(),
        },
        planning_worker_panel_state: PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshFailed,
            last_operation_label: Some("post-turn".to_string()),
            last_summary: Some(message),
            last_rejected_summary: None,
            last_queue_summary: Some("planning refresh timed out".to_string()),
            last_notice_detail: None,
            last_prompt: None,
            last_response: None,
            last_host_detail: Some(
                "host recovered the main-session from a stalled post-turn planning worker evaluation"
                    .to_string(),
            ),
        },
    }
}

fn post_turn_evaluation_failure_execution(
    context: &PostTurnEvaluationContext,
    request: &PostTurnEvaluationRequest,
    message: String,
) -> PostTurnEvaluationExecution {
    PostTurnEvaluationExecution {
        thread_id: context.thread_id.clone(),
        completed_turn_id: request.completed_turn_id.clone(),
        runtime_projection_workspace_directory: request.workspace_directory.clone(),
        evaluation: PostTurnEvaluationOutcome {
            provenance: PostTurnEvaluationProvenance::new(request.completed_turn_id.clone()),
            runtime_projection: PlanningRuntimeProjection::invalid(message.clone()),
            planning_repair_state: None,
            runtime_notices: vec![message.clone()],
            action: PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
            },
            operator_alerts: Vec::new(),
        },
        planning_worker_panel_state: PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshFailed,
            last_operation_label: Some("post-turn".to_string()),
            last_summary: Some(message),
            last_rejected_summary: None,
            last_queue_summary: Some("planning refresh failed".to_string()),
            last_notice_detail: None,
            last_prompt: None,
            last_response: None,
            last_host_detail: Some(
                "host blocked continuation because the post-turn evaluator terminated unexpectedly"
                    .to_string(),
            ),
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::adapter::outbound::github::GithubAutomationAdapter;
    use crate::application::port::outbound::github_automation_port::{
        GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
        GithubRepositoryVisibility,
    };
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
        PlanningTaskAuthorityCommit, PlanningTaskAuthorityMutationAudit,
        PlanningTaskRepositoryPort, task_authority_mutation_records,
    };
    use crate::application::port::outbound::planning_worker_port::{
        NoopPlanningWorkerPort, PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
        test_planning_worker_runtime_envelope,
    };
    use crate::application::service::parallel_mode::ParallelModeService;
    use crate::application::service::planning::task_tool::{
        PlanningTaskCreatePayload, PlanningTaskToolCreateRequest, PlanningTaskToolRequest,
        PlanningTaskToolService,
    };
    use crate::application::service::planning::{
        OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON,
        PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
        PlanningRuntimeWorkspaceStatus, PlanningWorkerRunOutcome,
    };
    use crate::domain::parallel_mode::{
        ParallelModeAgentSessionDetailSnapshot, ParallelModeCapabilityKey,
        ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
        ParallelModeLiveSessionDetailDefaults, ParallelModeSlotLeaseRequest,
        ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    };
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
        PriorityQueueProjection, PriorityQueueService, PriorityQueueSkippedTask, PriorityQueueTask,
        QueueIdleConfig, QueueIdlePolicy, TaskActor, TaskAuthorityDocument, TaskDefinition,
        TaskMutationProvenance, TaskStatus,
    };
    use std::collections::VecDeque;
    use std::fs;
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn post_turn_evaluator_boundary_uses_context_not_conversation_model() {
        let source = include_str!("post_turn_evaluation.rs");
        let official_completion_source =
            include_str!("post_turn_evaluation/official_completion.rs");
        let legacy_run_signature =
            ["fn run(\n        mut self,\n        conversation: &ConversationViewModel"].concat();
        let legacy_refresh_signature = [
            "fn run_planning_queue_refresh(\n        &mut self,\n        conversation: &ConversationViewModel",
        ]
        .concat();
        let legacy_fallback_name = ["fallback", "_conversation"].concat();

        assert!(source.contains("struct PostTurnEvaluationContext"));
        assert!(!source.contains(&legacy_run_signature));
        assert!(!source.contains(&legacy_refresh_signature));
        assert!(!source.contains(&legacy_fallback_name));
        assert!(!official_completion_source.contains("conversation: &ConversationViewModel"));
    }

    #[test]
    fn queue_receipt_diff_reports_completed_and_new_tasks_from_authority() {
        let current = task_definition("current", "Current work", TaskStatus::Ready);
        let mut completed = current.clone();
        completed.status = TaskStatus::Done;
        completed.updated_at = "2026-07-15T01:00:00Z".to_string();
        completed.provenance = TaskMutationProvenance::new(OriginSessionKind::Main)
            .with_thread_turn(Some("thread-1".to_string()), Some("turn-1".to_string()));
        let mut created = task_definition("follow-up", "Follow-up work", TaskStatus::Proposed);
        created.provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
            .with_parent(Some("thread-1".to_string()), Some("turn-1".to_string()));
        let mut foreign = task_definition("foreign", "Unrelated work", TaskStatus::Ready);
        foreign.provenance = TaskMutationProvenance::new(OriginSessionKind::System)
            .with_thread_turn(None, Some("turn-1".to_string()));
        let mutation = |task: &TaskDefinition,
                        mutation_kind: PlanningTaskAuthorityMutationKind,
                        before_status| PlanningTaskAuthorityMutationRecord {
            planning_revision: 5,
            task_id: task.id.clone(),
            task_title: task.title.clone(),
            mutation_kind,
            before_status,
            before_updated_at: before_status.map(|_| "2026-07-15T00:00:00Z".to_string()),
            after_status: Some(task.status),
            after_updated_at: Some(task.updated_at.clone()),
            after_task: Some(task.clone()),
            legacy_source_turn_id: None,
            provenance: task.provenance.clone(),
        };
        let mutations = vec![
            mutation(
                &completed,
                PlanningTaskAuthorityMutationKind::Updated,
                Some(TaskStatus::Ready),
            ),
            mutation(&created, PlanningTaskAuthorityMutationKind::Created, None),
            mutation(&foreign, PlanningTaskAuthorityMutationKind::Created, None),
        ];

        let receipt = build_queue_mutation_receipt(
            "thread-1",
            "turn-1",
            4,
            mutations,
            PlanningQueueAuthoritySnapshot {
                planning_revision: 5,
                tasks: vec![completed, created, foreign],
            },
        );

        assert_eq!(receipt.planning_revision, 5);
        assert_eq!(receipt.entries.len(), 2);
        assert_eq!(
            receipt.entries[0].mutation_kind,
            PlanningQueueMutationKind::Updated
        );
        assert_eq!(receipt.entries[0].before_status, Some(TaskStatus::Ready));
        assert_eq!(receipt.entries[0].after_status, TaskStatus::Done);
        assert_eq!(
            receipt.entries[1].mutation_kind,
            PlanningQueueMutationKind::Created
        );
        assert_eq!(receipt.entries[1].after_status, TaskStatus::Proposed);
        assert_eq!(receipt.cancellable_created_entries().count(), 1);
    }

    #[test]
    fn queue_receipt_journal_preserves_creation_ownership_across_concurrent_writes() {
        let current_provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
            .with_parent(Some("thread-1".to_string()), Some("turn-1".to_string()));
        let foreign_provenance = TaskMutationProvenance::new(OriginSessionKind::System)
            .with_thread_turn(None, Some("turn-1".to_string()));
        let record = |revision,
                      task_id: &str,
                      kind,
                      before_status,
                      after_updated_at: &str,
                      provenance: TaskMutationProvenance| {
            let mut after_task = task_definition(task_id, task_id, TaskStatus::Ready);
            after_task.updated_at = after_updated_at.to_string();
            after_task.provenance = provenance.clone();
            PlanningTaskAuthorityMutationRecord {
                planning_revision: revision,
                task_id: task_id.to_string(),
                task_title: task_id.to_string(),
                mutation_kind: kind,
                before_status,
                before_updated_at: before_status.map(|_| "before".to_string()),
                after_status: Some(TaskStatus::Ready),
                after_updated_at: Some(after_updated_at.to_string()),
                after_task: Some(after_task),
                legacy_source_turn_id: None,
                provenance,
            }
        };
        let mutations = vec![
            record(
                11,
                "external-created",
                PlanningTaskAuthorityMutationKind::Updated,
                Some(TaskStatus::Proposed),
                "current-update",
                current_provenance.clone(),
            ),
            record(
                12,
                "current-then-foreign",
                PlanningTaskAuthorityMutationKind::Created,
                None,
                "current-create",
                current_provenance.clone(),
            ),
            record(
                12,
                "current-foreign-current",
                PlanningTaskAuthorityMutationKind::Created,
                None,
                "same-second",
                current_provenance.clone(),
            ),
            record(
                13,
                "current-foreign-current",
                PlanningTaskAuthorityMutationKind::Updated,
                Some(TaskStatus::Ready),
                "same-second",
                foreign_provenance.clone(),
            ),
            record(
                14,
                "current-foreign-current",
                PlanningTaskAuthorityMutationKind::Updated,
                Some(TaskStatus::Ready),
                "same-second",
                current_provenance.clone(),
            ),
            record(
                12,
                "current-unchanged",
                PlanningTaskAuthorityMutationKind::Created,
                None,
                "current-stable",
                current_provenance.clone(),
            ),
            record(
                15,
                "foreign-unrelated",
                PlanningTaskAuthorityMutationKind::Created,
                None,
                "foreign-unrelated",
                foreign_provenance,
            ),
        ];
        let final_task = |id: &str, updated_at: &str| {
            let mut task = task_definition(id, id, TaskStatus::Ready);
            task.updated_at = updated_at.to_string();
            task
        };
        let mut externally_rewritten = final_task("current-then-foreign", "current-create");
        externally_rewritten.title = "Foreign title rewrite".to_string();
        externally_rewritten.provenance = current_provenance.clone();
        let mut external_created = final_task("external-created", "current-update");
        external_created.provenance = current_provenance.clone();
        let mut current_unchanged = final_task("current-unchanged", "current-stable");
        current_unchanged.provenance = current_provenance;
        let mut contaminated = final_task("current-foreign-current", "same-second");
        contaminated.provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
            .with_parent(Some("thread-1".to_string()), Some("turn-1".to_string()));

        let receipt = build_queue_mutation_receipt(
            "thread-1",
            "turn-1",
            10,
            mutations,
            PlanningQueueAuthoritySnapshot {
                planning_revision: 15,
                tasks: vec![
                    external_created,
                    externally_rewritten,
                    current_unchanged,
                    contaminated,
                    final_task("foreign-unrelated", "foreign-unrelated"),
                ],
            },
        );

        assert_eq!(receipt.entries.len(), 4);
        let external = receipt
            .entries
            .iter()
            .find(|entry| entry.task_id == "external-created")
            .unwrap();
        assert_eq!(external.mutation_kind, PlanningQueueMutationKind::Updated);
        let changed = receipt
            .entries
            .iter()
            .find(|entry| entry.task_id == "current-then-foreign")
            .unwrap();
        assert_eq!(changed.mutation_kind, PlanningQueueMutationKind::Created);
        assert!(!changed.unchanged_since_mutation);
        assert!(!changed.is_created_and_cancellable());
        let stable = receipt
            .entries
            .iter()
            .find(|entry| entry.task_id == "current-unchanged")
            .unwrap();
        assert!(stable.unchanged_since_mutation);
        assert!(stable.is_created_and_cancellable());
        let contaminated = receipt
            .entries
            .iter()
            .find(|entry| entry.task_id == "current-foreign-current")
            .unwrap();
        assert!(!contaminated.unchanged_since_mutation);
        assert!(!contaminated.is_created_and_cancellable());
    }

    #[test]
    fn queue_receipt_requires_origin_specific_thread_and_turn_pairs() {
        let mut task = task_definition("task-1", "Task 1", TaskStatus::Ready);
        task.provenance = TaskMutationProvenance::new(OriginSessionKind::Main)
            .with_thread_turn(Some("other-thread".to_string()), Some("turn-1".to_string()))
            .with_parent(Some("thread-1".to_string()), Some("turn-1".to_string()));
        let mut mutation = PlanningTaskAuthorityMutationRecord {
            planning_revision: 1,
            task_id: task.id.clone(),
            task_title: task.title.clone(),
            mutation_kind: PlanningTaskAuthorityMutationKind::Created,
            before_status: None,
            before_updated_at: None,
            after_status: Some(task.status),
            after_updated_at: Some(task.updated_at.clone()),
            after_task: Some(task.clone()),
            legacy_source_turn_id: None,
            provenance: task.provenance.clone(),
        };

        assert!(!task_mutation_belongs_to_turn(
            &mutation, "thread-1", "turn-1"
        ));

        mutation.provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
            .with_thread_turn(
                Some("worker-thread".to_string()),
                Some("turn-1".to_string()),
            )
            .with_parent(Some("other-thread".to_string()), Some("turn-1".to_string()));
        assert!(!task_mutation_belongs_to_turn(
            &mutation, "thread-1", "turn-1"
        ));

        mutation.provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
            .with_parent(Some("thread-1".to_string()), Some("turn-1".to_string()));
        assert!(task_mutation_belongs_to_turn(
            &mutation, "thread-1", "turn-1"
        ));
    }

    #[test]
    fn queue_receipt_rejects_plain_rewrite_absorbed_by_later_current_promotion() {
        let current_provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
            .with_parent(Some("thread-1".to_string()), Some("turn-1".to_string()));
        let mut proposed = task_definition("task-1", "Current title", TaskStatus::Proposed);
        proposed.provenance = current_provenance.clone();
        proposed.updated_at = "created".to_string();
        let proposed_authority = TaskAuthorityDocument {
            version: 1,
            tasks: vec![proposed.clone()],
        };
        let task_id = proposed.id.clone();
        let current_audit = PlanningTaskAuthorityMutationAudit {
            task_ids: std::slice::from_ref(&task_id),
            legacy_source_turn_id: None,
            provenance: &current_provenance,
        };
        let mut mutations =
            task_authority_mutation_records(None, &proposed_authority, 11, Some(current_audit));

        let mut externally_rewritten = proposed;
        externally_rewritten.title = "Admin title contribution".to_string();
        let rewritten_authority = TaskAuthorityDocument {
            version: 1,
            tasks: vec![externally_rewritten.clone()],
        };
        mutations.extend(task_authority_mutation_records(
            Some(&proposed_authority),
            &rewritten_authority,
            12,
            None,
        ));

        let mut promoted = externally_rewritten;
        promoted.status = TaskStatus::Ready;
        promoted.updated_at = "promoted".to_string();
        let promoted_authority = TaskAuthorityDocument {
            version: 1,
            tasks: vec![promoted.clone()],
        };
        mutations.extend(task_authority_mutation_records(
            Some(&rewritten_authority),
            &promoted_authority,
            13,
            Some(current_audit),
        ));

        let receipt = build_queue_mutation_receipt(
            "thread-1",
            "turn-1",
            10,
            mutations,
            PlanningQueueAuthoritySnapshot {
                planning_revision: 13,
                tasks: vec![promoted],
            },
        );

        assert_eq!(receipt.entries.len(), 1);
        assert_eq!(
            receipt.entries[0].mutation_kind,
            PlanningQueueMutationKind::Created
        );
        assert!(!receipt.entries[0].unchanged_since_mutation);
        assert!(!receipt.created_batch_is_cancellable());
    }

    #[test]
    fn parallel_completion_reports_drained_queue_when_official_refresh_finishes_all_work() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            None,
            None,
            PriorityQueueProjection {
                next_task: None,
                active_tasks: Vec::new(),
                proposed_tasks: Vec::new(),
                skipped_tasks: vec![PriorityQueueSkippedTask {
                    task_id: "done-task".to_string(),
                    task_title: "Finished parallel task".to_string(),
                    direction_id: "general-workstream".to_string(),
                    status: TaskStatus::Done,
                    reason: "status done is not executable".to_string(),
                }],
            },
        );

        let decision = PostTurnDecision::from_application_decision(
            "turn-1".to_string(),
            decide_parallel_official_completion_post_turn(&runtime_projection),
        );
        let PostTurnContinuationAction::SkipAutoFollow { reason } = decision.action else {
            panic!("parallel completion should skip auto-follow");
        };

        assert_eq!(reason, PostTurnAutoFollowSkipReason::PlanningQueueDrained);
        assert_eq!(decision.provenance.completed_turn_id, "turn-1");
        assert_eq!(decision.provenance.parallel_queue_signal, None);
        assert_eq!(decision.operator_alerts.len(), 1);
        assert_eq!(
            decision.operator_alerts[0].title,
            "All planning tasks complete"
        );
    }

    #[test]
    fn parallel_completion_keeps_supervisor_handoff_when_queue_still_has_work() {
        let runtime_projection = PlanningRuntimeProjection::invalid("planning still blocked");

        let decision = PostTurnDecision::from_application_decision(
            "turn-1".to_string(),
            decide_parallel_official_completion_post_turn(&runtime_projection),
        );
        let PostTurnContinuationAction::SkipAutoFollow { reason } = decision.action else {
            panic!("parallel completion should skip auto-follow");
        };

        assert_eq!(
            reason,
            PostTurnAutoFollowSkipReason::ParallelSessionCompleted
        );
        assert_eq!(
            decision.provenance.parallel_queue_signal,
            Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized)
        );
        assert!(decision.operator_alerts.is_empty());
    }

    #[test]
    fn timeout_execution_returns_blocked_action_and_failed_panel_state() {
        let context = test_context(ready_projection(Some(queue_task())));
        let request = test_request(context.clone());

        let execution =
            post_turn_evaluation_timeout_execution(&context, &request, Duration::from_secs(7));

        assert_eq!(execution.thread_id, "thread-1");
        assert_eq!(execution.completed_turn_id, "turn-1");
        assert_eq!(
            execution.evaluation.action,
            PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::PostTurnEvaluationTimedOut
            }
        );
        assert_eq!(
            execution.evaluation.runtime_projection.failure_reason(),
            Some("post-turn planning worker evaluation timed out after 7 seconds")
        );
        assert_eq!(
            execution.planning_worker_panel_state.status,
            PlanningWorkerStatus::RefreshFailed
        );
        assert_eq!(
            execution
                .planning_worker_panel_state
                .last_queue_summary
                .as_deref(),
            Some("planning refresh timed out")
        );
    }

    #[test]
    fn operator_alerts_only_surface_planning_queue_drained_skip() {
        assert_eq!(
            operator_alerts_for_action(&PostTurnContinuationAction::QueueAutoPrompt(Box::new(
                PostTurnQueuedPrompt {
                    prompt: "continue".to_string(),
                    mode_label: "auto".to_string(),
                    transcript_text: "queued".to_string(),
                },
            ))),
            Vec::<OperatorAlert>::new()
        );
        assert_eq!(
            operator_alerts_for_action(&PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::NoAgentReply,
            }),
            Vec::<OperatorAlert>::new()
        );

        let alerts = operator_alerts_for_action(&PostTurnContinuationAction::SkipAutoFollow {
            reason: PostTurnAutoFollowSkipReason::PlanningQueueDrained,
        });

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].title, "All planning tasks complete");
    }

    #[test]
    fn planning_skip_reason_mapping_covers_all_post_turn_action_variants() {
        let cases = [
            (
                PlanningPostTurnAutoFollowSkipReason::PostTurnContinuationPaused,
                PostTurnAutoFollowSkipReason::PostTurnContinuationPaused,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningQueueDrained,
                PostTurnAutoFollowSkipReason::PlanningQueueDrained,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop,
                PostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::LimitReached,
                PostTurnAutoFollowSkipReason::LimitReached,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::NoAgentReply,
                PostTurnAutoFollowSkipReason::NoAgentReply,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::StopKeywordMatched,
                PostTurnAutoFollowSkipReason::StopKeywordMatched,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::NoFileChanges,
                PostTurnAutoFollowSkipReason::NoFileChanges,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningBlocked,
                PostTurnAutoFollowSkipReason::PlanningBlocked,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningQueueHeadRequired,
                PostTurnAutoFollowSkipReason::PlanningQueueHeadRequired,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead,
                PostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead,
            ),
        ];

        for (planning_reason, post_turn_reason) in cases {
            assert_eq!(
                auto_follow_skip_reason_from_planning(planning_reason),
                post_turn_reason
            );
        }
        assert_eq!(
            auto_follow_skip_reason_from_post_turn(
                PostTurnAutoFollowStopReason::PlanningQueueDrained,
            ),
            PostTurnAutoFollowSkipReason::PlanningQueueDrained
        );
        assert_eq!(
            auto_follow_skip_reason_from_post_turn(
                PostTurnAutoFollowStopReason::ParallelSessionCompleted,
            ),
            PostTurnAutoFollowSkipReason::ParallelSessionCompleted
        );
    }

    #[test]
    fn queue_refresh_skip_keeps_projection_and_preserves_existing_panel_state() {
        with_test_event_logging(|| {
            let mut executor = test_executor();
            executor.planning_worker_panel_state.status = PlanningWorkerStatus::RefreshSucceeded;
            executor.planning_worker_panel_state.last_summary =
                Some("previous summary".to_string());
            let mut context = test_context(PlanningRuntimeProjection::invalid("planning blocked"));
            context.latest_main_reply = Some("worker reply".to_string());
            let request = test_request(context.clone());

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                outcome.runtime_projection.failure_reason(),
                Some("planning blocked")
            );
            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshSucceeded
            );
            assert_eq!(
                executor.planning_worker_panel_state.last_summary.as_deref(),
                Some("previous summary")
            );
        });
    }

    #[test]
    fn service_evaluate_auto_follow_off_still_settles_queue_without_next_prompt() {
        with_test_event_logging(|| {
            let service = test_service();
            let mut context = test_context(ready_projection(Some(queue_task())));
            let workspace = TempPlanningWorkspace::new_git("post-turn-paused");
            let workspace_directory = workspace.path.clone();
            context.planning_workspace_directory = workspace_directory.clone();
            context.continuation_paused = true;
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace_directory;
            request.planning_worker_panel_state.status = PlanningWorkerStatus::RefreshSucceeded;
            request.planning_worker_panel_state.last_summary = Some("previous summary".to_string());

            let execution = service.evaluate(request);

            assert_eq!(
                execution.evaluation.action,
                PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PostTurnContinuationPaused
                }
            );
            assert_eq!(
                execution.evaluation.runtime_projection.workspace_status(),
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            );
            assert!(
                execution
                    .evaluation
                    .runtime_projection
                    .queue_head()
                    .is_none()
            );
            assert!(execution.evaluation.runtime_notices.is_empty());
            assert_eq!(
                execution.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshSucceeded
            );
            assert_eq!(
                execution
                    .planning_worker_panel_state
                    .last_summary
                    .as_deref(),
                Some("planning worker disabled")
            );
            let receipt = execution.evaluation.provenance.queue_mutation_receipt;
            assert!(receipt.is_none());
        });
    }

    #[test]
    fn service_evaluate_explicit_stop_skips_queue_settlement() {
        with_test_event_logging(|| {
            let service = test_service();
            let mut context = test_context(ready_projection(Some(queue_task())));
            let workspace = TempPlanningWorkspace::new_git("post-turn-explicit-stop");
            context.planning_workspace_directory = workspace.path.clone();
            context.continuation_paused = true;
            context.planning_settlement_paused = true;
            let mut request = test_request(context);
            request.workspace_directory = workspace.path.clone();
            request.planning_worker_panel_state.status = PlanningWorkerStatus::RefreshSucceeded;
            request.planning_worker_panel_state.last_summary = Some("previous summary".to_string());

            let execution = service.evaluate(request);

            assert_eq!(
                execution.evaluation.action,
                PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PostTurnContinuationPaused
                }
            );
            assert!(
                execution
                    .evaluation
                    .provenance
                    .queue_mutation_receipt
                    .is_none()
            );
            assert_eq!(
                execution
                    .planning_worker_panel_state
                    .last_summary
                    .as_deref(),
                Some("previous summary")
            );
        });
    }

    #[test]
    fn service_receipt_detects_tool_only_worker_mutation_with_empty_final_commands() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new_git("post-turn-tool-only-receipt");
            seed_ready_queue_authority(&workspace.path);
            let repository = Arc::new(NoopPlanningTaskRepositoryPort);
            let worker = Arc::new(ToolOnlyMutationWorkerPort {
                repository: repository.clone(),
            });
            let planning = PlanningServices::from_ports(
                Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
                Arc::new(NoopPlanningAuthorityPort::default()),
                repository.clone(),
                worker,
            );
            let service = PostTurnEvaluationService::new(
                planning,
                ParallelModeTurnService::new(ParallelModeService::new(
                    Arc::new(SqlitePlanningAuthorityAdapter::new()),
                    Arc::new(GithubAutomationAdapter::new()),
                    Arc::new(GitParallelModeRuntimeAdapter::new()),
                )),
            );
            let baseline = repository
                .load_task_authority_snapshot(&workspace.path)
                .unwrap()
                .unwrap();
            let mut context = test_context(ready_projection(Some(queue_task())));
            context.planning_workspace_directory = workspace.path.clone();
            context.continuation_paused = true;
            let mut request = test_request(context);
            request.workspace_directory = workspace.path.clone();
            request.execution_snapshot_capture = Some(
                crate::application::service::planning::PlanningTurnExecutionSnapshotCapture::ready(
                    workspace.path.clone(),
                    crate::application::service::planning::PlanningExecutionSnapshot {
                        result_output_markdown: None,
                        planning_revision: Some(baseline.planning_revision),
                        task_authority: Some(baseline.task_authority),
                    },
                ),
            );

            let execution = service.evaluate(request);
            let receipt = execution
                .evaluation
                .provenance
                .queue_mutation_receipt
                .expect("tool-only mutation should produce a receipt");

            assert!(receipt.entries.iter().any(|entry| {
                entry.mutation_kind == PlanningQueueMutationKind::Created
                    && entry.task_title == "Tool-only follow-up"
            }));
            assert_eq!(
                execution.evaluation.action,
                PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PostTurnContinuationPaused
                }
            );
        });
    }

    #[test]
    fn service_receipt_folds_current_turn_proposal_promotion_into_created_entry() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new_git("post-turn-tool-promotion-receipt");
            seed_queue_idle_review_authority(&workspace.path);
            let repository = Arc::new(NoopPlanningTaskRepositoryPort);
            let worker = Arc::new(ToolOnlyMutationWorkerPort {
                repository: repository.clone(),
            });
            let planning = PlanningServices::from_ports(
                Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
                Arc::new(NoopPlanningAuthorityPort::default()),
                repository.clone(),
                worker,
            );
            let service = PostTurnEvaluationService::new(
                planning,
                ParallelModeTurnService::new(ParallelModeService::new(
                    Arc::new(SqlitePlanningAuthorityAdapter::new()),
                    Arc::new(GithubAutomationAdapter::new()),
                    Arc::new(GitParallelModeRuntimeAdapter::new()),
                )),
            );
            let baseline = repository
                .load_task_authority_snapshot(&workspace.path)
                .unwrap()
                .unwrap();
            let mut context = test_context(ready_projection(None));
            context.planning_workspace_directory = workspace.path.clone();
            context.continuation_paused = true;
            let mut request = test_request(context);
            request.workspace_directory = workspace.path.clone();
            request.execution_snapshot_capture = Some(
                crate::application::service::planning::PlanningTurnExecutionSnapshotCapture::ready(
                    workspace.path.clone(),
                    crate::application::service::planning::PlanningExecutionSnapshot {
                        result_output_markdown: None,
                        planning_revision: Some(baseline.planning_revision),
                        task_authority: Some(baseline.task_authority),
                    },
                ),
            );

            let execution = service.evaluate(request);
            let receipt = execution
                .evaluation
                .provenance
                .queue_mutation_receipt
                .expect("created proposal and host promotion should produce one receipt");
            let entry = receipt
                .entries
                .iter()
                .find(|entry| entry.task_title == "Tool-only follow-up")
                .expect("tool-created task should remain attributed to the completed turn");

            assert_eq!(entry.mutation_kind, PlanningQueueMutationKind::Created);
            assert_eq!(entry.after_status, TaskStatus::Ready);
            assert!(entry.unchanged_since_mutation);
            assert!(entry.is_created_and_cancellable());
        });
    }

    #[test]
    fn service_evaluate_enabled_request_runs_refresh_and_surfaces_drained_alert() {
        with_test_event_logging(|| {
            let service = test_service();
            let mut context = test_context(ready_projection(Some(queue_task())));
            let workspace = TempPlanningWorkspace::new_git("post-turn-enabled-refresh");
            let workspace_directory = workspace.path.clone();
            context.planning_workspace_directory = workspace_directory.clone();
            let mut request = test_request(context);
            request.workspace_directory = workspace_directory;

            let execution = service.evaluate(request);

            assert_eq!(
                execution.evaluation.action,
                PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PlanningQueueDrained
                }
            );
            assert_eq!(
                execution.evaluation.runtime_projection.workspace_status(),
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            );
            assert_eq!(execution.evaluation.operator_alerts.len(), 1);
            assert_eq!(
                execution.evaluation.operator_alerts[0].title,
                "All planning tasks complete"
            );
            assert_eq!(
                execution.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshSucceeded
            );
            assert_eq!(
                execution
                    .planning_worker_panel_state
                    .last_summary
                    .as_deref(),
                Some("planning worker disabled")
            );
        });
    }

    #[test]
    fn service_evaluate_with_timeout_returns_completed_execution_before_deadline() {
        let service = test_service();
        let mut context = test_context(ready_projection(Some(queue_task())));
        let workspace = TempPlanningWorkspace::new_git("post-turn-timeout-success");
        let workspace_directory = workspace.path.clone();
        context.planning_workspace_directory = workspace_directory.clone();
        let mut request = test_request(context);
        request.workspace_directory = workspace_directory;

        let execution = service.evaluate_with_timeout(request, Duration::from_secs(5));

        assert_eq!(execution.thread_id, "thread-1");
        assert_eq!(execution.completed_turn_id, "turn-1");
        assert_eq!(
            execution.evaluation.action,
            PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::PlanningQueueDrained
            }
        );
        assert_eq!(
            execution.planning_worker_panel_state.status,
            PlanningWorkerStatus::RefreshSucceeded
        );
    }

    #[test]
    fn service_timeout_invalidates_inflight_hidden_worker_generation() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (returned_tx, returned_rx) = std::sync::mpsc::channel();
        let worker = Arc::new(BlockingPlanningWorkerPort {
            started_tx,
            release_rx: Mutex::new(release_rx),
            returned_tx,
        });
        let service = test_service_with_worker(worker);
        let mut context = test_context(ready_projection(Some(queue_task())));
        let workspace = TempPlanningWorkspace::new_git("post-turn-timeout-cancel");
        context.planning_workspace_directory = workspace.path.clone();
        let mut request = test_request(context);
        request.workspace_directory = workspace.path.clone();
        let permit = request.continuation_permit.clone();
        let (execution_tx, execution_rx) = std::sync::mpsc::channel();

        let evaluator = std::thread::spawn(move || {
            let execution = service.evaluate_with_timeout(request, Duration::from_millis(500));
            execution_tx
                .send(execution)
                .expect("timeout execution should be observed");
        });

        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("hidden worker should enter before the timeout result is observed");
        let execution = execution_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timeout execution should return after cancellation settlement");

        assert_eq!(
            execution.evaluation.action,
            PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::PostTurnEvaluationTimedOut
            }
        );
        assert!(!permit.is_current());
        release_tx.send(()).expect("hidden worker should release");
        returned_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("hidden worker should return after cancellation");
        evaluator.join().expect("timeout evaluator should join");
    }

    #[test]
    fn service_worker_panic_is_reported_immediately_as_failure_not_timeout() {
        let service = test_service_with_worker(Arc::new(PanickingPlanningWorkerPort));
        let mut context = test_context(ready_projection(Some(queue_task())));
        let workspace = TempPlanningWorkspace::new_git("post-turn-worker-panic");
        context.planning_workspace_directory = workspace.path.clone();
        let mut request = test_request(context);
        request.workspace_directory = workspace.path.clone();
        let permit = request.continuation_permit.clone();

        let execution = service.evaluate_with_timeout(request, Duration::from_secs(5));

        assert_eq!(
            execution.evaluation.action,
            PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
            }
        );
        assert_eq!(
            execution.evaluation.runtime_projection.failure_reason(),
            Some("post-turn evaluation worker panicked")
        );
        assert!(!permit.is_current());
        assert_eq!(
            execution
                .planning_worker_panel_state
                .last_queue_summary
                .as_deref(),
            Some("planning refresh failed")
        );
    }

    #[test]
    fn queue_refresh_worker_failure_records_refresh_failure_panel() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("queue-refresh-worker-failure");
            let mut executor = test_executor_with_worker(Arc::new(FailingPlanningWorkerPort));
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                outcome.runtime_projection.failure_reason(),
                Some(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON)
            );
            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_operation_label
                    .as_deref(),
                Some("refresh")
            );
            assert_eq!(
                executor.planning_worker_panel_state.last_summary.as_deref(),
                Some("planning worker refresh failed: worker boom")
            );
        });
    }

    #[test]
    fn superseded_queue_refresh_never_calls_planning_worker_port() {
        let workspace = TempPlanningWorkspace::new("queue-refresh-superseded");
        let worker = Arc::new(CountingPlanningWorkerPort::default());
        let mut executor = test_executor_with_worker(worker.clone());
        let context = test_context(ready_projection(Some(queue_task())));
        let mut request = test_request(context.clone());
        request.workspace_directory = workspace.path.clone();
        let gate = crate::domain::planning::PostTurnContinuationGate::default();
        request.continuation_permit = gate.capture();
        gate.advance();

        let outcome = executor.run_planning_queue_refresh(
            &context,
            &request,
            context.current_runtime_projection.clone(),
        );

        assert_eq!(worker.call_count(), 0);
        assert_eq!(
            outcome.runtime_projection,
            context.current_runtime_projection
        );
    }

    #[test]
    fn queue_idle_derivation_worker_failure_records_idle_specific_panel_copy() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("queue-idle-worker-failure");
            let mut executor = test_executor_with_worker(Arc::new(FailingPlanningWorkerPort));
            let mut context = test_context(ready_projection(None));
            context.previous_handoff_task = None;
            context.latest_main_reply = Some("finished the requested work".to_string());
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                outcome.runtime_projection.failure_reason(),
                Some(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON)
            );
            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_operation_label
                    .as_deref(),
                Some("queue-idle-derive")
            );
            assert_eq!(
                executor.planning_worker_panel_state.last_summary.as_deref(),
                Some("planning worker queue-idle derivation failed: worker boom")
            );
        });
    }

    #[test]
    fn queue_refresh_success_records_worker_outcome_and_drained_projection() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("queue-refresh-success");
            let mut executor = test_executor();
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshSucceeded
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_operation_label
                    .as_deref(),
                Some("refresh")
            );
            assert_eq!(
                executor.planning_worker_panel_state.last_summary.as_deref(),
                Some("planning worker disabled")
            );
            assert_eq!(
                outcome.runtime_projection.workspace_status(),
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            );
        });
    }

    #[test]
    fn queue_refresh_unresolved_repair_blocks_auto_follow_with_refresh_failure_projection() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("queue-refresh-unresolved-repair");
            seed_ready_queue_authority(&workspace.path);
            let mut executor = test_executor_with_worker(Arc::new(StaticPlanningWorkerPort::new(
                invalid_task_command_worker_message(),
            )));
            let current_projection = executor
                .planning_feature
                .runtime
                .load_runtime_projection_or_invalid(&workspace.path);
            let context = test_context(current_projection);
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                outcome.runtime_projection.failure_reason(),
                Some(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON)
            );
            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RepairFailed
            );
            assert!(
                executor
                    .planning_worker_panel_state
                    .last_summary
                    .as_deref()
                    .is_some_and(|summary| summary.contains("planning worker repair exhausted")),
                "panel state: {:?}",
                executor.planning_worker_panel_state
            );
        });
    }

    #[test]
    fn queue_refresh_resolved_repair_uses_repaired_projection_for_final_decision() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("queue-refresh-resolved-repair");
            seed_ready_queue_authority(&workspace.path);
            let mut executor =
                test_executor_with_worker(Arc::new(SequencedPlanningWorkerPort::new([
                    invalid_task_command_worker_message(),
                    done_task_command_worker_message(),
                ])));
            let current_projection = executor
                .planning_feature
                .runtime
                .load_runtime_projection_or_invalid(&workspace.path);
            let context = test_context(current_projection);
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RepairSucceeded
            );
            assert_eq!(
                outcome.runtime_projection.workspace_status(),
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            );
            assert!(
                outcome
                    .runtime_projection
                    .auto_follow_pause_reason()
                    .is_none()
            );
        });
    }

    #[test]
    fn queue_refresh_repeated_handoff_marks_panel_failed_and_pauses_projection() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("queue-refresh-repeated-handoff");
            seed_ready_queue_authority(&workspace.path);
            let mut executor = test_executor_with_worker(Arc::new(StaticPlanningWorkerPort::new(
                empty_task_commands_worker_message(),
            )));
            let current_projection = executor
                .planning_feature
                .runtime
                .load_runtime_projection_or_invalid(&workspace.path);
            let mut context = test_context(current_projection);
            context.previous_handoff_task = Some(queue_handoff());
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            assert!(
                executor
                    .planning_worker_panel_state
                    .last_host_detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("previously handed-off task unchanged"))
            );
            assert!(
                outcome
                    .runtime_projection
                    .auto_follow_pause_reason()
                    .is_some_and(|reason| reason.contains("previously handed-off task unchanged"))
            );
        });
    }

    #[test]
    fn queue_idle_derivation_empty_records_host_detail_without_failure() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("queue-idle-derivation-empty");
            seed_queue_idle_review_authority(&workspace.path);
            let mut executor = test_executor();
            let current_projection = executor
                .planning_feature
                .runtime
                .load_runtime_projection_or_invalid(&workspace.path);
            let mut context = test_context(current_projection);
            context.previous_handoff_task = None;
            context.latest_main_reply = Some("the requested work is complete".to_string());
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshSucceeded
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_operation_label
                    .as_deref(),
                Some("queue-idle-derive")
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_host_detail
                    .as_deref(),
                Some(
                    "planning worker derived no justified follow-up task from the latest request and reply"
                )
            );
            assert_eq!(
                outcome.runtime_projection.workspace_status(),
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            );
        });
    }

    #[test]
    fn queue_refresh_promotes_worker_proposal_and_records_host_detail() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("queue-refresh-promotes-proposal");
            seed_ready_queue_authority(&workspace.path);
            let mut executor = test_executor_with_worker(Arc::new(StaticPlanningWorkerPort::new(
                proposed_followup_worker_message(),
            )));
            let current_projection = executor
                .planning_feature
                .runtime
                .load_runtime_projection_or_invalid(&workspace.path);
            let context = test_context(current_projection);
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();

            let outcome = executor.run_planning_queue_refresh(
                &context,
                &request,
                context.current_runtime_projection.clone(),
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshSucceeded
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_host_detail
                    .as_deref(),
                Some(
                    "host promoted top follow-up proposal into the executable queue: Review follow-up proposal"
                )
            );
            assert!(
                executor
                    .planning_worker_panel_state
                    .last_queue_summary
                    .as_deref()
                    .is_some_and(|summary| summary.contains("Review follow-up proposal"))
            );
            assert_eq!(
                outcome.runtime_projection.workspace_status(),
                PlanningRuntimeWorkspaceStatus::ReadyWithTask
            );
            assert!(
                outcome
                    .runtime_projection
                    .queue_summary()
                    .is_some_and(|summary| summary.contains("Review follow-up proposal"))
            );
        });
    }

    #[test]
    fn auto_follow_decision_queues_prompt_with_handoff_provenance() {
        with_test_event_logging(|| {
            let executor = test_executor();
            let mut context = test_context(ready_projection(Some(queue_task())));
            context.parallel_mode_enabled = true;
            let request = test_request(context.clone());

            let decision = executor.auto_follow_decision_from_projection(
                &context,
                &request,
                &context.current_runtime_projection,
            );

            let PostTurnContinuationAction::QueueAutoPrompt(prompt) = decision.action else {
                panic!("ready queue head should produce queued prompt action");
            };
            assert_eq!(prompt.mode_label, "auto-follow");
            assert!(prompt.prompt.contains("Queue head"));
            assert_eq!(
                decision
                    .provenance
                    .handoff_task
                    .as_ref()
                    .map(|task| task.task_id.as_str()),
                Some("task-1")
            );
            assert_eq!(
                decision.provenance.parallel_queue_signal,
                Some(ParallelModePostTurnQueueSignal::AutoFollowQueued)
            );
            assert!(decision.operator_alerts.is_empty());
        });
    }

    #[test]
    fn auto_follow_decision_maps_skip_to_post_turn_action() {
        with_test_event_logging(|| {
            let executor = test_executor();
            let mut context = test_context(ready_projection(Some(queue_task())));
            context.can_queue_next = false;
            let request = test_request(context.clone());

            let decision = executor.auto_follow_decision_from_projection(
                &context,
                &request,
                &context.current_runtime_projection,
            );

            assert_eq!(
                decision.action,
                PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::LimitReached
                }
            );
            assert_eq!(decision.provenance.completed_turn_id, "turn-1");
            assert!(decision.operator_alerts.is_empty());
        });
    }

    #[test]
    fn official_completion_capture_failure_updates_panel_state() {
        with_test_event_logging(|| {
            let mut executor = test_executor();
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.changed_planning_file_paths =
                vec![".codex-exec-loop/planning/result.md".into()];
            attach_synthetic_expected_lease(&mut request);

            let capture = executor.begin_official_completion_if_needed(&context, &request);

            assert!(matches!(capture, OfficialCompletionCapture::Failed { .. }));
            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            assert_eq!(
                executor.planning_worker_panel_state.last_summary.as_deref(),
                Some("parallel completion capture failed: repository inspection failed")
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_queue_summary
                    .as_deref(),
                Some("queue head: Queue head")
            );
        });
    }

    #[test]
    fn official_completion_capture_failure_never_falls_back_to_normal_refresh() {
        with_test_event_logging(|| {
            let worker = Arc::new(CountingPlanningWorkerPort::default());
            let service = test_service_with_worker(worker.clone());
            let workspace = TempPlanningWorkspace::new("official-capture-failure");
            let mut context = test_context(ready_projection(Some(queue_task())));
            context.planning_workspace_directory = workspace.path.clone();
            context.parallel_mode_enabled = true;
            let mut request = test_request(context);
            request.workspace_directory = workspace.path.clone();
            attach_synthetic_expected_lease(&mut request);

            let execution = service.evaluate(request);

            assert_eq!(worker.call_count(), 0);
            assert_eq!(
                execution.runtime_projection_workspace_directory,
                workspace.path
            );
            assert_eq!(
                execution.evaluation.action,
                PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
                }
            );
            assert!(
                execution
                    .evaluation
                    .runtime_projection
                    .failure_reason()
                    .is_some_and(|detail| detail.contains("parallel completion capture failed"))
            );
            assert!(
                execution
                    .evaluation
                    .runtime_notices
                    .iter()
                    .any(|detail| detail.contains("parallel completion capture failed"))
            );
        });
    }

    #[test]
    fn stale_captured_lease_before_completion_begin_does_not_touch_replacement_session() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new_git("official-stale-before-begin");
            let parallel_service = test_parallel_mode_service_for_post_turn();
            parallel_service
                .reset_pool_on_parallel_initial_setup_report(&workspace.path)
                .expect("parallel pool should initialize");
            let lease = parallel_service
                .acquire_slot_lease(
                    &workspace.path,
                    ParallelModeSlotLeaseRequest::from_task_identity("task-1", "Queue head"),
                )
                .expect("parallel slot should lease");
            parallel_service
                .mark_workspace_slot_running(&lease.worktree_path)
                .expect("parallel slot should become running");
            let replacement = replace_slot_generation(&workspace.path, &lease);
            let mut executor = test_executor_with_parallel_mode_service(parallel_service);
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = lease.worktree_path.clone();
            attach_expected_lease(&mut request, lease);

            let capture = executor.begin_official_completion_if_needed(&context, &request);

            assert!(matches!(
                capture,
                OfficialCompletionCapture::Failed { detail }
                    if detail.contains("captured slot lease generation is stale")
            ));
            assert_replacement_generation_unmodified(&workspace.path, &replacement);
        });
    }

    #[test]
    fn stale_captured_lease_before_refresh_does_not_touch_replacement_session() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new_git("official-stale-before-refresh");
            let parallel_service = test_parallel_mode_service_for_post_turn();
            parallel_service
                .reset_pool_on_parallel_initial_setup_report(&workspace.path)
                .expect("parallel pool should initialize");
            let lease = parallel_service
                .acquire_slot_lease(
                    &workspace.path,
                    ParallelModeSlotLeaseRequest::from_task_identity("task-1", "Queue head"),
                )
                .expect("parallel slot should lease");
            parallel_service
                .mark_workspace_slot_running(&lease.worktree_path)
                .expect("parallel slot should become running");
            let worker = Arc::new(CountingPlanningWorkerPort::default());
            let mut executor = test_executor_with_worker_and_parallel_mode_service(
                worker.clone(),
                parallel_service,
            );
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = lease.worktree_path.clone();
            attach_expected_lease(&mut request, lease.clone());
            let report = match executor.begin_official_completion_if_needed(&context, &request) {
                OfficialCompletionCapture::Captured(report) => report,
                capture => panic!("current captured lease should begin completion: {capture:?}"),
            };
            let replacement = replace_slot_generation(&workspace.path, &lease);

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &request.workspace_directory,
                &context.current_runtime_projection,
                &report,
            );

            assert_eq!(worker.call_count(), 0);
            assert!(
                outcome
                    .runtime_projection
                    .auto_follow_pause_reason()
                    .is_some_and(
                        |detail| detail.contains("captured slot lease generation is stale")
                    )
            );
            assert_replacement_generation_unmodified(&workspace.path, &replacement);
        });
    }

    #[test]
    fn stale_captured_lease_before_commit_ready_does_not_touch_replacement_session() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new_git("official-stale-before-commit-ready");
            let parallel_service = test_parallel_mode_service_for_post_turn();
            parallel_service
                .reset_pool_on_parallel_initial_setup_report(&workspace.path)
                .expect("parallel pool should initialize");
            let lease = parallel_service
                .acquire_slot_lease(
                    &workspace.path,
                    ParallelModeSlotLeaseRequest::from_task_identity("task-1", "Queue head"),
                )
                .expect("parallel slot should lease");
            parallel_service
                .mark_workspace_slot_running(&lease.worktree_path)
                .expect("parallel slot should become running");
            let replacement = replacement_slot_generation(&lease);
            let worker = Arc::new(ReuseSlotBeforeCommitReadyWorkerPort::new(
                &workspace.path,
                lease.clone(),
            ));
            let mut executor =
                test_executor_with_worker_and_parallel_mode_service(worker, parallel_service);
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = lease.worktree_path.clone();
            attach_expected_lease(&mut request, lease);
            let report = match executor.begin_official_completion_if_needed(&context, &request) {
                OfficialCompletionCapture::Captured(report) => report,
                capture => panic!("current captured lease should begin completion: {capture:?}"),
            };

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &request.workspace_directory,
                &context.current_runtime_projection,
                &report,
            );

            assert!(
                outcome
                    .runtime_projection
                    .auto_follow_pause_reason()
                    .is_some_and(|detail| {
                        detail.contains("commit_ready_persistence")
                            && detail.contains("captured slot lease generation is stale")
                    })
            );
            assert_replacement_generation_unmodified(&workspace.path, &replacement);
        });
    }

    #[test]
    fn official_completion_refresh_blocks_when_planning_workspace_is_unavailable() {
        with_test_event_logging(|| {
            let blocked_workspace = TempPlanningWorkspaceBlocker::new("official-refresh-blocked");
            let mut executor = test_executor();
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            attach_synthetic_expected_lease(&mut request);
            let contract = official_completion_contract();

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &blocked_workspace.path,
                &context.current_runtime_projection,
                &contract,
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            let failure_detail = executor
                .planning_worker_panel_state
                .last_summary
                .as_deref()
                .expect("blocked refresh should record a panel failure detail");
            assert!(failure_detail.starts_with("failed to load planning workspace:"));
            assert!(failure_detail.contains("workspace root"));
            assert!(failure_detail.contains(&blocked_workspace.path));
            assert_eq!(
                outcome.runtime_projection.failure_reason(),
                Some(failure_detail)
            );
            assert_eq!(
                outcome.runtime_projection_workspace_directory,
                blocked_workspace.path
            );
            assert!(outcome.runtime_notices.iter().any(|notice| {
                notice.contains("official completion failure state could not be recorded")
            }));
        });
    }

    #[test]
    fn official_completion_refresh_records_worker_execution_failure() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("official-refresh-worker-failure");
            let mut executor = test_executor_with_worker(Arc::new(FailingPlanningWorkerPort));
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();
            attach_synthetic_expected_lease(&mut request);
            let contract = official_completion_contract();

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &workspace.path,
                &context.current_runtime_projection,
                &contract,
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_operation_label
                    .as_deref(),
                Some("official-refresh")
            );
            assert_eq!(
                executor.planning_worker_panel_state.last_summary.as_deref(),
                Some("official completion refresh failed: worker boom")
            );
            assert_eq!(
                outcome.runtime_projection.auto_follow_pause_reason(),
                Some("official completion refresh failed: worker boom")
            );
            assert!(outcome.runtime_notices.iter().any(|notice| {
                notice.contains("official completion refreshing state could not be recorded")
            }));
        });
    }

    #[test]
    fn official_completion_refresh_success_finalizes_slot_and_preserves_worker_summary() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new_git("official-refresh-success");
            let parallel_service = test_parallel_mode_service_for_post_turn();
            parallel_service
                .reset_pool_on_parallel_initial_setup_report(&workspace.path)
                .expect("parallel pool should initialize");
            let lease = parallel_service
                .acquire_slot_lease(
                    &workspace.path,
                    ParallelModeSlotLeaseRequest::from_task_identity("task-1", "Queue head"),
                )
                .expect("parallel slot should lease");
            parallel_service
                .mark_workspace_slot_running(&lease.worktree_path)
                .expect("parallel slot should enter running state");
            let mut executor = test_executor_with_parallel_mode_service(parallel_service.clone());
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = lease.worktree_path.clone();
            attach_expected_lease(&mut request, lease);
            let contract = official_completion_contract();

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &workspace.path,
                &context.current_runtime_projection,
                &contract,
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshSucceeded
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_operation_label
                    .as_deref(),
                Some("official-refresh")
            );
            assert_eq!(
                executor.planning_worker_panel_state.last_summary.as_deref(),
                Some("planning worker disabled")
            );
            assert_eq!(
                outcome.runtime_projection_workspace_directory,
                workspace.path
            );
            assert_ne!(
                outcome.runtime_projection_workspace_directory,
                request.workspace_directory
            );
            assert_eq!(
                executor
                    .planning_worker_panel_state
                    .last_notice_detail
                    .as_deref(),
                None
            );
            assert_eq!(
                outcome.runtime_projection.workspace_status(),
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            );
            let snapshot =
                parallel_service.build_passive_supervisor_snapshot(&workspace.path, None);
            assert_eq!(
                snapshot
                    .detail
                    .session
                    .as_ref()
                    .map(|detail| detail.state_label.as_str()),
                Some("commit_ready")
            );
            assert!(outcome.runtime_notices.iter().any(|notice| {
                notice.contains("remains commit-ready because no guarded automation epoch")
            }));
        });
    }

    #[test]
    fn official_completion_commit_ready_failure_pauses_auto_follow() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("official-refresh-commit-ready-failure");
            let mut executor = test_executor();
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();
            attach_synthetic_expected_lease(&mut request);
            let contract = official_completion_contract();

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &workspace.path,
                &context.current_runtime_projection,
                &contract,
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            assert!(
                outcome
                    .runtime_projection
                    .auto_follow_pause_reason()
                    .is_some_and(|reason| reason.contains("commit_ready_persistence"))
            );
            assert!(outcome.runtime_notices.iter().any(|notice| {
                notice.contains("commit-ready state could not be recorded after official refresh")
            }));
        });
    }

    #[test]
    fn official_completion_refresh_unresolved_repair_blocks_slot_finalization() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("official-refresh-unresolved-repair");
            let mut executor = test_executor_with_worker(Arc::new(StaticPlanningWorkerPort::new(
                invalid_task_command_worker_message(),
            )));
            let context = test_context(ready_projection(Some(queue_task())));
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();
            attach_synthetic_expected_lease(&mut request);
            let contract = official_completion_contract();

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &workspace.path,
                &context.current_runtime_projection,
                &contract,
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            assert_eq!(
                executor.planning_worker_panel_state.last_summary.as_deref(),
                Some(OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON)
            );
            assert_eq!(
                outcome.runtime_projection.auto_follow_pause_reason(),
                Some(OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON)
            );
        });
    }

    #[test]
    fn official_completion_refresh_resolved_repair_uses_repaired_projection() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new_git("official-refresh-resolved-repair");
            let parallel_service = test_parallel_mode_service_for_post_turn();
            parallel_service
                .reset_pool_on_parallel_initial_setup_report(&workspace.path)
                .expect("parallel pool should initialize");
            let lease = parallel_service
                .acquire_slot_lease(
                    &workspace.path,
                    ParallelModeSlotLeaseRequest::from_task_identity("task-1", "Queue head"),
                )
                .expect("parallel slot should lease");
            parallel_service
                .mark_workspace_slot_running(&lease.worktree_path)
                .expect("parallel slot should enter running state");
            seed_ready_queue_authority(&workspace.path);
            let mut executor = test_executor_with_worker_and_parallel_mode_service(
                Arc::new(SequencedPlanningWorkerPort::new([
                    invalid_task_command_worker_message(),
                    done_task_command_worker_message(),
                ])),
                parallel_service,
            );
            let current_projection = executor
                .planning_feature
                .runtime
                .load_runtime_projection_or_invalid(&workspace.path);
            let context = test_context(current_projection);
            let mut request = test_request(context.clone());
            request.workspace_directory = lease.worktree_path.clone();
            attach_expected_lease(&mut request, lease);
            let contract = official_completion_contract();

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &workspace.path,
                &context.current_runtime_projection,
                &contract,
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RepairSucceeded
            );
            assert_eq!(
                outcome.runtime_projection.workspace_status(),
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
            );
            assert!(
                outcome
                    .runtime_projection
                    .auto_follow_pause_reason()
                    .is_none()
            );
        });
    }

    #[test]
    fn official_completion_refresh_repeated_queue_head_blocks_slot_finalization() {
        with_test_event_logging(|| {
            let workspace = TempPlanningWorkspace::new("official-refresh-repeated-head");
            seed_ready_queue_authority(&workspace.path);
            let mut executor = test_executor();
            let current_projection = executor
                .planning_feature
                .runtime
                .load_runtime_projection_or_invalid(&workspace.path);
            let mut context = test_context(current_projection);
            context.previous_handoff_task = Some(queue_handoff());
            let mut request = test_request(context.clone());
            request.workspace_directory = workspace.path.clone();
            attach_synthetic_expected_lease(&mut request);
            let contract = official_completion_contract();

            let outcome = executor.run_official_completion_refresh(
                &context,
                &request,
                &workspace.path,
                &context.current_runtime_projection,
                &contract,
            );

            assert_eq!(
                executor.planning_worker_panel_state.status,
                PlanningWorkerStatus::RefreshFailed
            );
            assert!(
                executor
                    .planning_worker_panel_state
                    .last_summary
                    .as_deref()
                    .is_some_and(|summary| summary.contains("previously handed-off task unchanged"))
            );
            assert!(
                outcome
                    .runtime_projection
                    .auto_follow_pause_reason()
                    .is_some_and(|reason| reason.contains("previously handed-off task unchanged"))
            );
        });
    }

    #[test]
    fn planning_worker_outcome_preserves_non_success_status_when_blocked() {
        let mut executor = test_executor();
        let outcome = PlanningWorkerRunOutcome {
            runtime_projection: PlanningRuntimeProjection::invalid(
                "accepted task authority is invalid",
            ),
            notices: Vec::new(),
            repair_request: None,
            worker_summary: None,
            worker_response: None,
            rejected_summary: None,
            task_authority_changed: false,
        };

        executor.record_planning_worker_outcome(PlanningWorkerStatus::Idle, &outcome);

        assert_eq!(
            executor.planning_worker_panel_state.status,
            PlanningWorkerStatus::Idle
        );
    }

    fn test_executor() -> PostTurnEvaluationExecutor {
        test_executor_with_worker(Arc::new(NoopPlanningWorkerPort))
    }

    #[derive(Debug)]
    struct PostTurnTestGithubAutomationPort;

    impl GithubAutomationPort for PostTurnTestGithubAutomationPort {
        fn inspect_capabilities(&self, _repo_root: &str) -> GithubAutomationCapabilities {
            let ready = |key| {
                ParallelModeCapabilitySnapshot::new(
                    key,
                    ParallelModeCapabilityState::Ready,
                    "test capability ready",
                    None,
                )
            };
            GithubAutomationCapabilities::new(
                ready(ParallelModeCapabilityKey::PushRemote),
                ready(ParallelModeCapabilityKey::GhBinary),
                ready(ParallelModeCapabilityKey::GhAuth),
            )
        }

        fn repository_identity(&self, _repo_root: &str) -> anyhow::Result<String> {
            Ok("RefinedStone/codex-exec-loop".to_string())
        }

        fn repository_visibility(
            &self,
            _repo_root: &str,
        ) -> anyhow::Result<GithubRepositoryVisibility> {
            Ok(GithubRepositoryVisibility::Private)
        }

        fn repository_identity_for_push_url(
            &self,
            repo_root: &str,
            _push_remote: &str,
            _credential_redacted_push_url: &str,
        ) -> anyhow::Result<String> {
            self.repository_identity(repo_root)
        }

        fn repository_visibility_for_push_url(
            &self,
            repo_root: &str,
            _push_remote: &str,
            _credential_redacted_push_url: &str,
        ) -> anyhow::Result<GithubRepositoryVisibility> {
            self.repository_visibility(repo_root)
        }

        fn credential_redacted_push_url_for_remote(
            &self,
            repo_root: &str,
            push_remote: &str,
        ) -> anyhow::Result<String> {
            let output = Command::new("git")
                .current_dir(repo_root)
                .args(["remote", "get-url", "--push", push_remote])
                .output()?;
            anyhow::ensure!(
                output.status.success(),
                "test push remote URL is unavailable"
            );
            Ok(String::from_utf8(output.stdout)?.trim().to_string())
        }

        fn remote_branch_names_for_prefix_for_delivery_target(
            &self,
            repo_root: &str,
            _push_remote: &str,
            credential_redacted_push_url: &str,
            branch_prefix: &str,
        ) -> anyhow::Result<Vec<String>> {
            let remote_pattern = format!("refs/heads/{branch_prefix}*");
            let output = Command::new("git")
                .current_dir(repo_root)
                .args([
                    "ls-remote",
                    "--heads",
                    credential_redacted_push_url,
                    remote_pattern.as_str(),
                ])
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()?;
            anyhow::ensure!(output.status.success(), "test remote branch listing failed");
            String::from_utf8(output.stdout)?
                .lines()
                .map(|line| {
                    let (_, remote_ref) = line
                        .split_once(char::is_whitespace)
                        .ok_or_else(|| anyhow::anyhow!("test remote branch row is malformed"))?;
                    remote_ref
                        .trim()
                        .strip_prefix("refs/heads/")
                        .map(str::to_string)
                        .ok_or_else(|| anyhow::anyhow!("test remote branch ref is malformed"))
                })
                .collect()
        }

        fn fetch_branch_to_tracking_ref_for_delivery_target(
            &self,
            repo_root: &str,
            _push_remote: &str,
            credential_redacted_push_url: &str,
            branch_name: &str,
            tracking_ref: &str,
        ) -> anyhow::Result<String> {
            let refspec = format!("+refs/heads/{branch_name}:{tracking_ref}");
            let status = Command::new("git")
                .current_dir(repo_root)
                .args(["fetch", "--quiet", credential_redacted_push_url, &refspec])
                .status()?;
            anyhow::ensure!(status.success(), "test frozen-target fetch failed");
            let output = Command::new("git")
                .current_dir(repo_root)
                .args(["rev-parse", tracking_ref])
                .output()?;
            anyhow::ensure!(output.status.success(), "test tracking ref is unavailable");
            Ok(String::from_utf8(output.stdout)?.trim().to_string())
        }

        fn push_branch(
            &self,
            _repo_root: &str,
            _branch_name: &str,
            _force_with_lease: bool,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn ensure_pull_request(
            &self,
            _repo_root: &str,
            base_branch: &str,
            head_branch: &str,
            _title: &str,
            _body: &str,
        ) -> anyhow::Result<GithubAutomationPullRequest> {
            Ok(GithubAutomationPullRequest::new(
                1,
                "https://example.invalid/pr/1",
                "OPEN",
                base_branch,
                head_branch,
                false,
            ))
        }

        fn inspect_pull_request(
            &self,
            _repo_root: &str,
            pr_number: u64,
        ) -> anyhow::Result<GithubAutomationPullRequest> {
            Ok(GithubAutomationPullRequest::new(
                pr_number,
                "https://example.invalid/pr/1",
                "OPEN",
                "prerelease",
                "akra-agent/test",
                false,
            ))
        }

        fn push_integration_branch(
            &self,
            _repo_root: &str,
            _branch_name: &str,
            _expected_old_commit_sha: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn close_pull_request(&self, _repo_root: &str, _pr_number: u64) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn test_parallel_mode_service_for_post_turn() -> ParallelModeService {
        ParallelModeService::new(
            Arc::new(SqlitePlanningAuthorityAdapter::new()),
            Arc::new(PostTurnTestGithubAutomationPort),
            Arc::new(GitParallelModeRuntimeAdapter::new()),
        )
    }

    fn test_executor_with_parallel_mode_service(
        parallel_mode_service: ParallelModeService,
    ) -> PostTurnEvaluationExecutor {
        test_executor_with_worker_and_parallel_mode_service(
            Arc::new(NoopPlanningWorkerPort),
            parallel_mode_service,
        )
    }

    fn test_executor_with_worker_and_parallel_mode_service(
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
        parallel_mode_service: ParallelModeService,
    ) -> PostTurnEvaluationExecutor {
        PostTurnEvaluationExecutor::new(
            PlanningServices::from_ports(
                Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
                Arc::new(NoopPlanningAuthorityPort::default()),
                Arc::new(NoopPlanningTaskRepositoryPort),
                planning_worker_port,
            ),
            ParallelModeTurnService::new(parallel_mode_service),
            PlanningWorkerPanelState::default(),
        )
    }

    fn test_service() -> PostTurnEvaluationService {
        test_service_with_worker(Arc::new(NoopPlanningWorkerPort))
    }

    fn test_service_with_worker(
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> PostTurnEvaluationService {
        PostTurnEvaluationService::new(
            PlanningServices::from_ports(
                Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
                Arc::new(NoopPlanningAuthorityPort::default()),
                Arc::new(NoopPlanningTaskRepositoryPort),
                planning_worker_port,
            ),
            ParallelModeTurnService::new(ParallelModeService::new(
                Arc::new(SqlitePlanningAuthorityAdapter::new()),
                Arc::new(GithubAutomationAdapter::new()),
                Arc::new(GitParallelModeRuntimeAdapter::new()),
            )),
        )
    }

    fn test_executor_with_worker(
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> PostTurnEvaluationExecutor {
        PostTurnEvaluationExecutor::new(
            PlanningServices::from_ports(
                Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
                Arc::new(NoopPlanningAuthorityPort::default()),
                Arc::new(NoopPlanningTaskRepositoryPort),
                planning_worker_port,
            ),
            ParallelModeTurnService::new(ParallelModeService::new(
                Arc::new(SqlitePlanningAuthorityAdapter::new()),
                Arc::new(GithubAutomationAdapter::new()),
                Arc::new(GitParallelModeRuntimeAdapter::new()),
            )),
            PlanningWorkerPanelState::default(),
        )
    }

    fn test_context(
        current_runtime_projection: PlanningRuntimeProjection,
    ) -> PostTurnEvaluationContext {
        PostTurnEvaluationContext {
            thread_id: "thread-1".to_string(),
            planning_workspace_directory: "/tmp/workspace".to_string(),
            latest_user_message: Some("user request".to_string()),
            latest_main_reply: Some("assistant reply".to_string()),
            previous_handoff_task: None,
            current_runtime_projection,
            parallel_mode_enabled: false,
            parallel_automation_epoch_id: Some(1),
            planning_settlement_paused: false,
            continuation_paused: false,
            can_queue_next: true,
            stop_keyword: "stop".to_string(),
            stop_keyword_matched: false,
            no_file_changes_stop_matched: false,
            mode_label: "auto-follow".to_string(),
        }
    }

    fn test_request(context: PostTurnEvaluationContext) -> PostTurnEvaluationRequest {
        let continuation_gate = crate::domain::planning::PostTurnContinuationGate::default();
        PostTurnEvaluationRequest {
            context,
            workspace_directory: "/tmp/workspace".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
            continuation_permit: continuation_gate.capture(),
        }
    }

    fn attach_synthetic_expected_lease(request: &mut PostTurnEvaluationRequest) {
        let lease = ParallelModeSlotLeaseSnapshot::new(
            "slot-test",
            "task-1",
            "Queue head",
            "agent-task-1",
            "akra-agent/slot-test/task-1",
            request.workspace_directory.clone(),
            ParallelModeSlotLeaseState::Running,
            "2026-07-12T00:00:00Z",
            Some("2026-07-12T00:00:01Z".to_string()),
        )
        .with_lease_generation("a".repeat(64));
        attach_expected_lease(request, lease);
    }

    fn attach_expected_lease(
        request: &mut PostTurnEvaluationRequest,
        lease: ParallelModeSlotLeaseSnapshot,
    ) {
        request.execution_snapshot_capture = Some(
            crate::application::service::planning::PlanningTurnExecutionSnapshotCapture::ready(
                request.workspace_directory.clone(),
                crate::application::service::planning::PlanningExecutionSnapshot::default(),
            )
            .with_parallel_slot_lease(Some(lease)),
        );
    }

    fn replace_slot_generation(
        workspace_directory: &str,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> ParallelModeSlotLeaseSnapshot {
        let replacement = replacement_slot_generation(expected_lease);
        SqlitePlanningAuthorityAdapter::upsert_runtime_slot_lease(
            workspace_directory,
            &replacement,
        )
        .expect("replacement slot generation should persist");
        let mut detail = ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
            &replacement,
            ParallelModeLiveSessionDetailDefaults {
                validation_summary: "replacement validation pending",
                authority_refresh_outcome: "replacement authority refresh pending",
            },
        );
        detail.state_label = "running".to_string();
        detail.completion_state_label = "in_progress".to_string();
        detail.latest_summary = "replacement session is running".to_string();
        SqlitePlanningAuthorityAdapter::upsert_runtime_session_detail(workspace_directory, &detail)
            .expect("replacement session detail should persist");
        replacement
    }

    fn replacement_slot_generation(
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> ParallelModeSlotLeaseSnapshot {
        let mut replacement = expected_lease.clone();
        replacement.task_id = "replacement-task".to_string();
        replacement.task_title = "Replacement task".to_string();
        replacement.agent_id = "replacement-agent".to_string();
        replacement.state = ParallelModeSlotLeaseState::Running;
        replacement.leased_at = "2026-07-12T01:00:00Z".to_string();
        replacement.running_started_at = Some("2026-07-12T01:00:01Z".to_string());
        replacement.lease_generation = Some("b".repeat(64));
        replacement
    }

    fn assert_replacement_generation_unmodified(
        workspace_directory: &str,
        replacement: &ParallelModeSlotLeaseSnapshot,
    ) {
        let projections =
            SqlitePlanningAuthorityAdapter::load_runtime_projections(workspace_directory)
                .expect("replacement authority projections should load");
        assert_eq!(
            projections.slot_leases.get(&replacement.slot_id),
            Some(replacement)
        );
        let detail = projections
            .session_details
            .iter()
            .find(|detail| detail.session_key == replacement.session_key())
            .expect("replacement session detail should remain present");
        assert_eq!(detail.state_label, "running");
        assert_eq!(detail.completion_state_label, "in_progress");
        assert_eq!(
            detail.authority_refresh_outcome,
            "replacement authority refresh pending"
        );
        assert!(projections.distributor_queue_records.is_empty());
    }

    fn ready_projection(queue_head: Option<PriorityQueueTask>) -> PlanningRuntimeProjection {
        PlanningRuntimeProjection::ready(
            "Planning Context".to_string(),
            "queue summary".to_string(),
            queue_head,
        )
    }

    fn queue_task() -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: "task-1".to_string(),
            direction_id: "general-workstream".to_string(),
            direction_title: "General".to_string(),
            task_title: "Queue head".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-05-12T00:00:00Z".to_string(),
            rank_reasons: vec!["ready".to_string()],
        }
    }

    fn queue_handoff() -> PlanningTaskHandoff {
        PlanningTaskHandoff {
            task_id: "task-1".to_string(),
            task_title: "Queue head".to_string(),
            direction_id: "general-workstream".to_string(),
            combined_priority: 80,
            updated_at: "2026-05-12T00:00:00Z".to_string(),
            status_label: "ready".to_string(),
        }
    }

    fn seed_ready_queue_authority(workspace_directory: &str) {
        seed_authority(
            workspace_directory,
            QueueIdleConfig::default(),
            vec![task_definition("task-1", "Queue head", TaskStatus::Ready)],
        );
    }

    fn seed_queue_idle_review_authority(workspace_directory: &str) {
        let prompt_path = ".codex-exec-loop/planning/prompts/queue-idle-review.md";
        fs::create_dir_all(format!(
            "{workspace_directory}/.codex-exec-loop/planning/prompts"
        ))
        .expect("queue idle prompt directory should be seeded");
        fs::write(
            format!("{workspace_directory}/{prompt_path}"),
            "# Queue Idle Review\nSuggest a justified follow-up task when the queue is empty.\n",
        )
        .expect("queue idle prompt should be seeded");
        seed_authority(
            workspace_directory,
            QueueIdleConfig {
                policy: QueueIdlePolicy::ReviewAndEnqueue,
                prompt_path: prompt_path.to_string(),
            },
            Vec::new(),
        );
    }

    fn seed_authority(
        workspace_directory: &str,
        queue_idle: QueueIdleConfig,
        tasks: Vec<TaskDefinition>,
    ) {
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle,
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General".to_string(),
                summary: "General workstream".to_string(),
                success_criteria: vec!["Queue head is done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        };
        let task_authority = TaskAuthorityDocument { version: 1, tasks };
        let queue_projection = PriorityQueueService::new()
            .build_projection(&directions, &task_authority)
            .expect("seeded authority should build queue projection");
        let repository = NoopPlanningTaskRepositoryPort;
        repository
            .commit_direction_authority_snapshot(
                workspace_directory,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions: &directions,
                    authority_mutation_owner_token: None,
                },
            )
            .expect("direction authority should be seeded");
        repository
            .commit_task_authority_snapshot(
                workspace_directory,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority: &task_authority,
                    queue_projection: &queue_projection,
                },
            )
            .expect("task authority should be seeded");
    }

    fn task_definition(id: &str, title: &str, status: TaskStatus) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: "general-workstream".to_string(),
            direction_relation_note: "fits the general workstream".to_string(),
            title: title.to_string(),
            description: format!("Continue {title}"),
            status,
            base_priority: 80,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: Default::default(),
            updated_at: "2026-05-12T00:00:00Z".to_string(),
        }
    }

    fn with_test_event_logging<T>(action: impl FnOnce() -> T) -> T {
        use tracing_subscriber::prelude::*;

        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(format!(
                "{}=debug",
                crate::diagnostics::trace_event_log::AKRA_EVENT_TARGET
            )))
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink));
        tracing::subscriber::with_default(subscriber, action)
    }

    fn official_completion_contract() -> PlanningOfficialCompletionRefreshContract {
        PlanningOfficialCompletionRefreshContract::new(
            "turn-1",
            7,
            PlanningOfficialCompletionRefreshPayload::new(
                "agent-1",
                "task-1",
                "Queue head",
                "agent/task-1",
                "/tmp/slot-worktree",
                "abc123",
                "validation passed",
                "agent finished queue head",
                Some("agent finished queue head".to_string()),
                None,
                "2026-05-12T00:00:00Z",
            ),
        )
    }

    struct FailingPlanningWorkerPort;
    struct PanickingPlanningWorkerPort;

    struct ToolOnlyMutationWorkerPort {
        repository: Arc<NoopPlanningTaskRepositoryPort>,
    }

    impl PlanningWorkerPort for ToolOnlyMutationWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> anyhow::Result<PlanningWorkerResponse> {
            let mut tool_request =
                PlanningTaskToolRequest::CreateTask(PlanningTaskToolCreateRequest {
                    version: 1,
                    apply: true,
                    legacy_source_turn_id: Some("spoof-source".to_string()),
                    origin_session_kind: Some(OriginSessionKind::Main),
                    thread_id: Some("spoof-thread".to_string()),
                    turn_id: Some("spoof-turn".to_string()),
                    parent_thread_id: Some("spoof-parent-thread".to_string()),
                    parent_turn_id: Some("spoof-parent-turn".to_string()),
                    input: PlanningTaskCreatePayload {
                        direction_id: None,
                        direction_relation_note: None,
                        title: "Tool-only follow-up".to_string(),
                        description: None,
                        status: Some(TaskStatus::Proposed),
                        base_priority: None,
                        dynamic_priority_delta: None,
                        priority_reason: None,
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                    },
                });
            tool_request.apply_cli_host_context(
                request.parent_thread_id.clone(),
                request.parent_turn_id.clone(),
            );
            PlanningTaskToolService::new(self.repository.clone(), PriorityQueueService::new())
                .handle_request(&request.workspace_directory, tool_request)?;
            Ok(PlanningWorkerResponse {
                operation: request.operation,
                thread_id: Some("tool-worker-thread".to_string()),
                turn_id: Some("tool-worker-turn".to_string()),
                runtime_envelope: Some(test_planning_worker_runtime_envelope()),
                final_agent_message: Some(empty_task_commands_worker_message().to_string()),
                changed_planning_file_paths: Vec::new(),
            })
        }
    }

    struct BlockingPlanningWorkerPort {
        started_tx: std::sync::mpsc::Sender<()>,
        release_rx: Mutex<std::sync::mpsc::Receiver<()>>,
        returned_tx: std::sync::mpsc::Sender<()>,
    }

    impl PlanningWorkerPort for BlockingPlanningWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> anyhow::Result<PlanningWorkerResponse> {
            self.started_tx
                .send(())
                .map_err(|error| anyhow::anyhow!("failed to report worker start: {error}"))?;
            self.release_rx
                .lock()
                .map_err(|_| anyhow::anyhow!("blocking worker release mutex poisoned"))?
                .recv()
                .map_err(|error| anyhow::anyhow!("failed to await worker release: {error}"))?;
            self.returned_tx
                .send(())
                .map_err(|error| anyhow::anyhow!("failed to report worker return: {error}"))?;
            Ok(PlanningWorkerResponse {
                operation: request.operation,
                thread_id: None,
                turn_id: None,
                runtime_envelope: Some(test_planning_worker_runtime_envelope()),
                final_agent_message: Some("late worker result".to_string()),
                changed_planning_file_paths: Vec::new(),
            })
        }
    }

    #[derive(Default)]
    struct CountingPlanningWorkerPort {
        calls: Mutex<usize>,
    }

    impl CountingPlanningWorkerPort {
        fn call_count(&self) -> usize {
            *self
                .calls
                .lock()
                .expect("worker call count should not be poisoned")
        }
    }

    impl PlanningWorkerPort for CountingPlanningWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> anyhow::Result<PlanningWorkerResponse> {
            *self
                .calls
                .lock()
                .expect("worker call count should not be poisoned") += 1;
            Ok(PlanningWorkerResponse {
                operation: request.operation,
                thread_id: None,
                turn_id: None,
                runtime_envelope: Some(test_planning_worker_runtime_envelope()),
                final_agent_message: Some("counted".to_string()),
                changed_planning_file_paths: Vec::new(),
            })
        }
    }

    impl PlanningWorkerPort for FailingPlanningWorkerPort {
        fn run_planning_session(
            &self,
            _request: PlanningWorkerRequest,
        ) -> anyhow::Result<PlanningWorkerResponse> {
            Err(anyhow::anyhow!("worker boom"))
        }
    }

    struct ReuseSlotBeforeCommitReadyWorkerPort {
        workspace_directory: String,
        expected_lease: ParallelModeSlotLeaseSnapshot,
        calls: Mutex<usize>,
    }

    impl ReuseSlotBeforeCommitReadyWorkerPort {
        fn new(
            workspace_directory: impl Into<String>,
            expected_lease: ParallelModeSlotLeaseSnapshot,
        ) -> Self {
            Self {
                workspace_directory: workspace_directory.into(),
                expected_lease,
                calls: Mutex::new(0),
            }
        }
    }

    impl PlanningWorkerPort for ReuseSlotBeforeCommitReadyWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> anyhow::Result<PlanningWorkerResponse> {
            let mut calls = self.calls.lock().expect("reuse worker mutex should hold");
            if *calls == 0 {
                replace_slot_generation(&self.workspace_directory, &self.expected_lease);
            }
            *calls += 1;
            Ok(PlanningWorkerResponse {
                operation: request.operation,
                thread_id: Some("replacement-worker-thread".to_string()),
                turn_id: Some("replacement-worker-turn".to_string()),
                runtime_envelope: Some(test_planning_worker_runtime_envelope()),
                final_agent_message: Some("planning worker disabled".to_string()),
                changed_planning_file_paths: Vec::new(),
            })
        }
    }

    impl PlanningWorkerPort for PanickingPlanningWorkerPort {
        fn run_planning_session(
            &self,
            _request: PlanningWorkerRequest,
        ) -> anyhow::Result<PlanningWorkerResponse> {
            panic!("planning worker panic fixture")
        }
    }

    struct StaticPlanningWorkerPort {
        final_agent_message: &'static str,
    }

    impl StaticPlanningWorkerPort {
        fn new(final_agent_message: &'static str) -> Self {
            Self {
                final_agent_message,
            }
        }
    }

    impl PlanningWorkerPort for StaticPlanningWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> anyhow::Result<PlanningWorkerResponse> {
            Ok(PlanningWorkerResponse {
                operation: request.operation,
                thread_id: Some("worker-thread-1".to_string()),
                turn_id: Some("worker-turn-1".to_string()),
                runtime_envelope: Some(test_planning_worker_runtime_envelope()),
                final_agent_message: Some(self.final_agent_message.to_string()),
                changed_planning_file_paths: Vec::new(),
            })
        }
    }

    struct SequencedPlanningWorkerPort {
        final_agent_messages: Mutex<VecDeque<&'static str>>,
    }

    impl SequencedPlanningWorkerPort {
        fn new<const N: usize>(messages: [&'static str; N]) -> Self {
            Self {
                final_agent_messages: Mutex::new(VecDeque::from(messages)),
            }
        }
    }

    impl PlanningWorkerPort for SequencedPlanningWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> anyhow::Result<PlanningWorkerResponse> {
            let final_agent_message = self
                .final_agent_messages
                .lock()
                .expect("sequenced worker mutex should not be poisoned")
                .pop_front()
                .expect("sequenced worker should have a response for every request");
            Ok(PlanningWorkerResponse {
                operation: request.operation,
                thread_id: Some("worker-thread-1".to_string()),
                turn_id: Some("worker-turn-1".to_string()),
                runtime_envelope: Some(test_planning_worker_runtime_envelope()),
                final_agent_message: Some(final_agent_message.to_string()),
                changed_planning_file_paths: Vec::new(),
            })
        }
    }

    fn invalid_task_command_worker_message() -> &'static str {
        r#"The worker tried to update planning.

```json
{"planning_task_commands":{"version":1,"commands":[{"create_task":{"title":"Missing op"}}]}}
```"#
    }

    fn done_task_command_worker_message() -> &'static str {
        r#"The repair worker corrected planning.

```json
{"planning_task_commands":{"version":1,"commands":[{"op":"update_task","task_id":"task-1","status":"done","priority_reason":"Completed by official completion repair."}]}}
```"#
    }

    fn empty_task_commands_worker_message() -> &'static str {
        r#"The worker found no planning changes.

```json
{"planning_task_commands":{"version":1,"commands":[]}}
```"#
    }

    fn proposed_followup_worker_message() -> &'static str {
        r#"The worker completed the current queue head and proposed a follow-up.

```json
{"planning_task_commands":{"version":1,"commands":[{"op":"update_task","task_id":"task-1","status":"done","priority_reason":"Completed by the latest main reply."},{"op":"create_task","title":"Review follow-up proposal","description":"Review the completed work and decide whether to continue.","direction_id":"general-workstream","direction_relation_note":"follows the general workstream after queue head completion","status":"proposed","base_priority":70,"priority_reason":"Potential follow-up after the completed queue head."}]}}
```"#
    }

    struct TempPlanningWorkspace {
        path: String,
    }

    impl TempPlanningWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&path).expect("temp planning workspace should be created");
            Self {
                path: path.display().to_string(),
            }
        }

        fn new_git(prefix: &str) -> Self {
            let workspace = Self::new(prefix);
            run_git(&workspace.path, &["init", "-q"]);
            run_git(&workspace.path, &["config", "user.name", "RefinedStone"]);
            run_git(
                &workspace.path,
                &["config", "user.email", "chem.en.9273@gmail.com"],
            );
            fs::write(format!("{}/README.md", workspace.path), "seed\n")
                .expect("seed file should write");
            run_git(&workspace.path, &["add", "README.md"]);
            run_git(&workspace.path, &["commit", "-qm", "init"]);
            run_git(&workspace.path, &["branch", "akra"]);
            run_git(&workspace.path, &["branch", "prerelease"]);
            let origin = format!("{}/.git/test-origin.git", workspace.path);
            run_git(&workspace.path, &["init", "--bare", "-q", &origin]);
            run_git(&workspace.path, &["remote", "add", "origin", &origin]);
            run_git(
                &workspace.path,
                &["push", "-q", "-u", "origin", "prerelease"],
            );
            workspace
        }
    }

    impl Drop for TempPlanningWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn run_git(workspace_dir: &str, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(workspace_dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git command should succeed: git {:?}\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    struct TempPlanningWorkspaceBlocker {
        path: String,
    }

    impl TempPlanningWorkspaceBlocker {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::write(&path, "not a directory")
                .expect("temp planning workspace blocker file should be created");
            Self {
                path: path.display().to_string(),
            }
        }
    }

    impl Drop for TempPlanningWorkspaceBlocker {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
