use std::collections::BTreeMap;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;
use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningApplicationQueueTask, PlanningApplicationSkippedTask,
    PlanningQueueAuthoritySnapshot, PlanningQueueCancellationRequest, PlanningRuntimeProjection,
    PlanningTaskMutationCommitResult,
};
use crate::domain::planning::{PlanningQueueMutationReceipt, TaskStatus};

use super::{ConversationInputState, ConversationState, NativeTuiApp, TuiLanguage};

const QUEUE_OVERLAY_SCREEN_DETAIL_LIMIT: usize = 56;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayActionTask {
    pub(super) task_id: String,
    pub(super) status: TaskStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayAuthorityToken {
    pub(super) status: TaskStatus,
    pub(super) updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueMutationContext {
    pub(super) workspace_directory: String,
    pub(super) active_thread_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueueMutationKind {
    RemoveSelected,
    UndoLatestRegistration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueMutationOperation {
    pub(super) operation_id: u64,
    pub(super) context: QueueMutationContext,
    pub(super) kind: QueueMutationKind,
    pub(super) request: PlanningQueueCancellationRequest,
    pub(super) receipt_at_start: Option<PlanningQueueMutationReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueMutationAuthoritySnapshot {
    pub(super) runtime_projection: PlanningRuntimeProjection,
    pub(super) queue_authority: PlanningQueueAuthoritySnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum QueueMutationAuthorityRefreshError {
    AuthorityUnavailable(String),
    RevisionsKeptChanging {
        projection_revision: i64,
        authority_revision: i64,
    },
    RuntimeProjectionUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueMutationWorkerResult {
    pub(super) operation: QueueMutationOperation,
    pub(super) mutation: Result<PlanningTaskMutationCommitResult, String>,
    pub(super) authority:
        Result<QueueMutationAuthoritySnapshot, QueueMutationAuthorityRefreshError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayAuthorityLoadRequest {
    pub(super) request_id: u64,
    pub(super) context: QueueMutationContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayAuthorityLoadResult {
    pub(super) request: QueueOverlayAuthorityLoadRequest,
    pub(super) authority:
        Result<QueueMutationAuthoritySnapshot, QueueMutationAuthorityRefreshError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueueOverlayAuthorityLoadCompletion {
    Applied,
    Ignored,
    ReloadRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QueueOverlayAuthorityProjectionState {
    Idle,
    Loading(QueueOverlayAuthorityLoadRequest),
    Ready {
        request: QueueOverlayAuthorityLoadRequest,
        runtime_projection: Box<PlanningRuntimeProjection>,
        planning_revision: i64,
        authority_tokens: BTreeMap<String, QueueOverlayAuthorityToken>,
    },
    Failed {
        request: QueueOverlayAuthorityLoadRequest,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum QueueOverlayAuthorityScreenModel {
    Idle,
    Loading {
        request_id: u64,
    },
    Ready {
        request_id: u64,
        planning_revision: i64,
    },
    Failed {
        request_id: u64,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum QueueOverlayConversationScreenModel {
    Loading,
    Failed(String),
    Ready {
        runtime_projection: Box<PlanningRuntimeProjection>,
        planning_notice: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueueActionBlockReason {
    ParallelModeOwnsTaskLeases,
    PostTurnPlanningInFlight,
    ActiveTurnInFlight,
    ConversationNotReady,
    AuthoritySnapshotChanged,
    SelectedItemUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayScreenModel {
    pub(super) conversation: QueueOverlayConversationScreenModel,
    pub(super) authority: QueueOverlayAuthorityScreenModel,
    pub(super) selected_task_id: Option<String>,
    pub(super) feedback: Option<String>,
    pub(super) pending_operation_id: Option<u64>,
    pub(super) authority_refresh_required: bool,
    pub(super) latest_registration_undo_available: bool,
    pub(super) remove_block_reason: Option<QueueActionBlockReason>,
    pub(super) undo_block_reason: Option<QueueActionBlockReason>,
    pub(super) planning_worker_host_detail: Option<String>,
    pub(super) tui_language: TuiLanguage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct QueueMutationUiState {
    next_operation_id: u64,
    pending: Option<QueueMutationOperation>,
    authority_refresh_required: bool,
}

impl QueueMutationUiState {
    pub(super) fn begin(
        &mut self,
        context: QueueMutationContext,
        kind: QueueMutationKind,
        request: PlanningQueueCancellationRequest,
        receipt_at_start: Option<PlanningQueueMutationReceipt>,
    ) -> Option<QueueMutationOperation> {
        if self.pending.is_some()
            || self.authority_refresh_required
            || request.workspace_directory != context.workspace_directory
        {
            return None;
        }
        self.next_operation_id = self.next_operation_id.wrapping_add(1).max(1);
        let operation = QueueMutationOperation {
            operation_id: self.next_operation_id,
            context,
            kind,
            request,
            receipt_at_start,
        };
        self.pending = Some(operation.clone());
        Some(operation)
    }

    pub(super) fn pending_operation_id(&self) -> Option<u64> {
        self.pending
            .as_ref()
            .map(|operation| operation.operation_id)
    }

    pub(super) fn authority_refresh_required(&self) -> bool {
        self.authority_refresh_required
    }

    pub(super) fn require_authority_refresh(&mut self) {
        self.authority_refresh_required = true;
    }

    pub(super) fn record_authority_refresh(&mut self) {
        self.authority_refresh_required = false;
    }

    pub(super) fn take_matching(
        &mut self,
        completed: &QueueMutationOperation,
    ) -> Option<QueueMutationOperation> {
        if self.pending.as_ref() != Some(completed) {
            return None;
        }
        self.pending.take()
    }
}

impl From<PlanningApplicationQueueTask> for QueueOverlayActionTask {
    fn from(task: PlanningApplicationQueueTask) -> Self {
        Self {
            task_id: task.task_id,
            status: task.status,
        }
    }
}

impl From<PlanningApplicationSkippedTask> for QueueOverlayActionTask {
    fn from(task: PlanningApplicationSkippedTask) -> Self {
        Self {
            task_id: task.task_id,
            status: task.status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayUiState {
    next_authority_request_id: u64,
    authority_projection: QueueOverlayAuthorityProjectionState,
    selected_task_id: Option<String>,
    feedback: Option<String>,
    receipt_undo_hit_area: Option<Rect>,
}

impl Default for QueueOverlayUiState {
    fn default() -> Self {
        Self {
            next_authority_request_id: 0,
            authority_projection: QueueOverlayAuthorityProjectionState::Idle,
            selected_task_id: None,
            feedback: None,
            receipt_undo_hit_area: None,
        }
    }
}

impl QueueOverlayUiState {
    pub(super) fn begin_authority_load(
        &mut self,
        context: QueueMutationContext,
    ) -> QueueOverlayAuthorityLoadRequest {
        self.next_authority_request_id = self.next_authority_request_id.wrapping_add(1).max(1);
        let request = QueueOverlayAuthorityLoadRequest {
            request_id: self.next_authority_request_id,
            context,
        };
        self.authority_projection = QueueOverlayAuthorityProjectionState::Loading(request.clone());
        self.feedback = None;
        request
    }

    pub(super) fn is_loading_request(&self, request: &QueueOverlayAuthorityLoadRequest) -> bool {
        matches!(
            &self.authority_projection,
            QueueOverlayAuthorityProjectionState::Loading(pending) if pending == request
        )
    }

    pub(super) fn requires_authority_load_for(
        &self,
        context: &QueueMutationContext,
        visible_planning_revision: Option<i64>,
    ) -> bool {
        match &self.authority_projection {
            QueueOverlayAuthorityProjectionState::Idle => true,
            QueueOverlayAuthorityProjectionState::Loading(request)
            | QueueOverlayAuthorityProjectionState::Failed { request, .. } => {
                request.context != *context
            }
            QueueOverlayAuthorityProjectionState::Ready {
                request,
                planning_revision,
                ..
            } => {
                request.context != *context || Some(*planning_revision) != visible_planning_revision
            }
        }
    }

    pub(super) fn apply_authority_loaded(
        &mut self,
        request: QueueOverlayAuthorityLoadRequest,
        runtime_projection: PlanningRuntimeProjection,
        planning_revision: i64,
        authority_tokens: BTreeMap<String, QueueOverlayAuthorityToken>,
    ) -> bool {
        if !self.is_loading_request(&request)
            || !queue_authority_binding_is_coherent(
                &runtime_projection,
                planning_revision,
                &authority_tokens,
            )
        {
            return false;
        }
        self.authority_projection = QueueOverlayAuthorityProjectionState::Ready {
            request,
            runtime_projection: Box::new(runtime_projection),
            planning_revision,
            authority_tokens,
        };
        true
    }

    pub(super) fn apply_authority_load_failed(
        &mut self,
        request: QueueOverlayAuthorityLoadRequest,
        error: String,
    ) -> bool {
        if !self.is_loading_request(&request) {
            return false;
        }
        self.authority_projection = QueueOverlayAuthorityProjectionState::Failed { request, error };
        true
    }

    pub(super) fn authority_screen_model(&self) -> QueueOverlayAuthorityScreenModel {
        match &self.authority_projection {
            QueueOverlayAuthorityProjectionState::Idle => QueueOverlayAuthorityScreenModel::Idle,
            QueueOverlayAuthorityProjectionState::Loading(request) => {
                QueueOverlayAuthorityScreenModel::Loading {
                    request_id: request.request_id,
                }
            }
            QueueOverlayAuthorityProjectionState::Ready {
                request,
                planning_revision,
                ..
            } => QueueOverlayAuthorityScreenModel::Ready {
                request_id: request.request_id,
                planning_revision: *planning_revision,
            },
            QueueOverlayAuthorityProjectionState::Failed { request, error } => {
                QueueOverlayAuthorityScreenModel::Failed {
                    request_id: request.request_id,
                    error: error.clone(),
                }
            }
        }
    }

    pub(super) fn selected_task_id(&self) -> Option<&str> {
        self.selected_task_id.as_deref()
    }

    pub(super) fn feedback(&self) -> Option<&str> {
        self.feedback.as_deref()
    }

    pub(super) fn set_feedback(&mut self, feedback: impl Into<String>) {
        self.feedback = Some(feedback.into());
    }

    pub(super) fn bind_authority_snapshot(
        &mut self,
        runtime_projection: PlanningRuntimeProjection,
        planning_revision: i64,
        authority_tokens: BTreeMap<String, QueueOverlayAuthorityToken>,
    ) -> bool {
        if !queue_authority_binding_is_coherent(
            &runtime_projection,
            planning_revision,
            &authority_tokens,
        ) {
            return false;
        }
        if let QueueOverlayAuthorityProjectionState::Ready { request, .. } =
            &self.authority_projection
        {
            self.authority_projection = QueueOverlayAuthorityProjectionState::Ready {
                request: request.clone(),
                runtime_projection: Box::new(runtime_projection),
                planning_revision,
                authority_tokens,
            };
        }
        true
    }

    pub(super) fn ready_runtime_projection(&self) -> Option<&PlanningRuntimeProjection> {
        match &self.authority_projection {
            QueueOverlayAuthorityProjectionState::Ready {
                runtime_projection, ..
            } => Some(runtime_projection.as_ref()),
            QueueOverlayAuthorityProjectionState::Idle
            | QueueOverlayAuthorityProjectionState::Loading(_)
            | QueueOverlayAuthorityProjectionState::Failed { .. } => None,
        }
    }

    pub(super) fn clear_authority_binding(&mut self) {
        self.authority_projection = QueueOverlayAuthorityProjectionState::Idle;
    }

    pub(super) fn bind_receipt_undo_hit_area(&mut self, hit_area: Option<Rect>) {
        self.receipt_undo_hit_area = hit_area;
    }

    pub(super) fn clear_receipt_undo_hit_area(&mut self) {
        self.receipt_undo_hit_area = None;
    }

    pub(super) fn receipt_undo_hit_area(&self) -> Option<Rect> {
        self.receipt_undo_hit_area
    }

    fn take_receipt_undo_hit(&mut self, column: u16, row: u16) -> bool {
        let clicked = self
            .receipt_undo_hit_area
            .is_some_and(|area| area.contains(Position::new(column, row)));
        if clicked {
            self.receipt_undo_hit_area = None;
        }
        clicked
    }

    pub(super) fn selected_authority_token(
        &self,
    ) -> Option<(i64, &str, &QueueOverlayAuthorityToken)> {
        let task_id = self.selected_task_id.as_deref()?;
        let QueueOverlayAuthorityProjectionState::Ready {
            planning_revision,
            authority_tokens,
            ..
        } = &self.authority_projection
        else {
            return None;
        };
        Some((*planning_revision, task_id, authority_tokens.get(task_id)?))
    }

    pub(super) fn reset(&mut self) {
        let next_authority_request_id = self.next_authority_request_id;
        *self = Self {
            next_authority_request_id,
            ..Self::default()
        };
    }

    pub(super) fn sync(&mut self, task_ids: &[String]) {
        if task_ids.is_empty() {
            self.selected_task_id = None;
            return;
        }
        if self
            .selected_task_id
            .as_ref()
            .is_none_or(|selected| !task_ids.contains(selected))
        {
            self.selected_task_id = task_ids.first().cloned();
        }
    }

    pub(super) fn move_selection(&mut self, task_ids: &[String], delta: isize) {
        self.sync(task_ids);
        let Some(selected) = self.selected_task_id.as_ref() else {
            return;
        };
        let current = task_ids
            .iter()
            .position(|task_id| task_id == selected)
            .unwrap_or(0);
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize)
        }
        .min(task_ids.len().saturating_sub(1));
        self.selected_task_id = task_ids.get(next).cloned();
        self.feedback = None;
    }
}

impl NativeTuiApp {
    pub(super) fn begin_queue_overlay_authority_load(
        &mut self,
    ) -> QueueOverlayAuthorityLoadRequest {
        let context = self.current_queue_mutation_context();
        self.queue_overlay_ui_state.begin_authority_load(context)
    }

    pub(super) fn queue_overlay_authority_load_required(&self) -> bool {
        self.shell_overlay == ShellOverlay::Queue
            && self.pending_queue_mutation_operation_id().is_none()
            && self.queue_overlay_ui_state.requires_authority_load_for(
                &self.current_queue_mutation_context(),
                self.planning_runtime_projection_snapshot()
                    .planning_revision(),
            )
    }

    pub(super) fn queue_overlay_screen_model(&self) -> QueueOverlayScreenModel {
        let pending_operation_id = self.pending_queue_mutation_operation_id();
        let authority_refresh_required = self.queue_mutation_requires_authority_refresh();
        let authority = self.queue_overlay_ui_state.authority_screen_model();
        let authority_snapshot_changed =
            matches!(&authority, QueueOverlayAuthorityScreenModel::Ready { .. })
                && self.queue_overlay_ui_state.requires_authority_load_for(
                    &self.current_queue_mutation_context(),
                    self.planning_runtime_projection_snapshot()
                        .planning_revision(),
                );
        let remove_block_reason = authority_snapshot_changed
            .then_some(QueueActionBlockReason::AuthoritySnapshotChanged)
            .or_else(|| self.queue_mutation_block_reason())
            .or_else(|| {
                (matches!(&authority, QueueOverlayAuthorityScreenModel::Ready { .. })
                    && self
                        .queue_overlay_ui_state
                        .selected_authority_token()
                        .is_none())
                .then_some(QueueActionBlockReason::SelectedItemUnavailable)
            });
        let undo_block_reason = authority_snapshot_changed
            .then_some(QueueActionBlockReason::AuthoritySnapshotChanged)
            .or_else(|| self.queue_receipt_undo_block_reason());
        let latest_registration_undo_available = matches!(
            (&self.conversation_state, &authority),
            (
                ConversationState::Ready(conversation),
                QueueOverlayAuthorityScreenModel::Ready {
                    planning_revision,
                    ..
                }
            ) if conversation
                .latest_queue_mutation_receipt
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.created_batch_is_cancellable()
                        && *planning_revision == receipt.planning_revision
                })
        );
        let conversation = match &self.conversation_state {
            ConversationState::Loading => QueueOverlayConversationScreenModel::Loading,
            ConversationState::Failed(message) => {
                QueueOverlayConversationScreenModel::Failed(message.clone())
            }
            ConversationState::Ready(conversation) => QueueOverlayConversationScreenModel::Ready {
                runtime_projection: Box::new(
                    self.queue_overlay_ui_state
                        .ready_runtime_projection()
                        .cloned()
                        .unwrap_or_else(|| self.planning_runtime_projection_snapshot()),
                ),
                planning_notice: conversation
                    .planning_notice_summary(QUEUE_OVERLAY_SCREEN_DETAIL_LIMIT),
            },
        };
        QueueOverlayScreenModel {
            conversation,
            authority,
            selected_task_id: self
                .queue_overlay_ui_state
                .selected_task_id()
                .map(str::to_string),
            feedback: self.queue_overlay_ui_state.feedback().map(str::to_string),
            pending_operation_id,
            authority_refresh_required,
            latest_registration_undo_available,
            remove_block_reason,
            undo_block_reason,
            planning_worker_host_detail: self
                .planning_worker_panel_state
                .last_host_detail
                .as_deref()
                .map(|detail| {
                    detail
                        .chars()
                        .take(QUEUE_OVERLAY_SCREEN_DETAIL_LIMIT)
                        .collect()
                }),
            tui_language: self.tui_language,
        }
    }

    pub(super) fn pending_queue_mutation_operation_id(&self) -> Option<u64> {
        self.queue_mutation_ui_state.pending_operation_id()
    }

    pub(super) fn queue_mutation_requires_authority_refresh(&self) -> bool {
        self.queue_mutation_ui_state.authority_refresh_required()
    }

    pub(super) fn current_queue_mutation_context(&self) -> QueueMutationContext {
        let active_thread_id = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.has_active_thread() => {
                Some(conversation.thread_id.clone())
            }
            ConversationState::Loading
            | ConversationState::Ready(_)
            | ConversationState::Failed(_) => None,
        };
        QueueMutationContext {
            workspace_directory: self.planning_workspace_directory(),
            active_thread_id,
        }
    }

    pub(super) fn queue_mutation_block_reason(&self) -> Option<QueueActionBlockReason> {
        if self.parallel_mode_enabled() {
            return Some(QueueActionBlockReason::ParallelModeOwnsTaskLeases);
        }
        match &self.conversation_state {
            ConversationState::Ready(conversation)
                if conversation.has_post_turn_settlement_in_flight() =>
            {
                Some(QueueActionBlockReason::PostTurnPlanningInFlight)
            }
            ConversationState::Ready(conversation)
                if conversation.auto_follow_state.has_live_activity() =>
            {
                Some(QueueActionBlockReason::PostTurnPlanningInFlight)
            }
            ConversationState::Ready(conversation)
                if matches!(
                    conversation.input_state,
                    ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue
                ) =>
            {
                None
            }
            ConversationState::Ready(_) => Some(QueueActionBlockReason::ActiveTurnInFlight),
            ConversationState::Loading | ConversationState::Failed(_) => {
                Some(QueueActionBlockReason::ConversationNotReady)
            }
        }
    }

    pub(super) fn queue_receipt_undo_block_reason(&self) -> Option<QueueActionBlockReason> {
        if self.parallel_mode_enabled() {
            return Some(QueueActionBlockReason::ParallelModeOwnsTaskLeases);
        }
        match &self.conversation_state {
            ConversationState::Ready(conversation)
                if conversation.has_post_turn_settlement_in_flight()
                    || conversation.auto_follow_state.has_live_activity() =>
            {
                Some(QueueActionBlockReason::PostTurnPlanningInFlight)
            }
            ConversationState::Ready(_) => None,
            ConversationState::Loading | ConversationState::Failed(_) => {
                Some(QueueActionBlockReason::ConversationNotReady)
            }
        }
    }

    pub(super) fn queue_receipt_undo_task_count(&self) -> Option<usize> {
        if self.pending_queue_mutation_operation_id().is_some()
            || self.queue_mutation_requires_authority_refresh()
            || self.shell_overlay != ShellOverlay::Hidden
            || self.is_exit_confirmation_visible()
            || self.is_turn_steer_confirmation_visible()
            || self.queue_receipt_undo_block_reason().is_some()
        {
            return None;
        }
        let ConversationState::Ready(conversation) = &self.conversation_state else {
            return None;
        };
        let receipt = conversation.latest_queue_mutation_receipt.as_ref()?;
        receipt
            .created_batch_is_cancellable()
            .then(|| receipt.created_entries().count())
    }

    pub(super) fn clear_queue_receipt_undo_hit_area(&mut self) {
        self.queue_overlay_ui_state.clear_receipt_undo_hit_area();
    }

    pub(super) fn queue_receipt_undo_mouse_capture_requested(&self) -> bool {
        self.queue_overlay_ui_state
            .receipt_undo_hit_area()
            .is_some()
            && self.queue_receipt_undo_task_count().is_some()
    }

    pub(super) fn handle_queue_receipt_mouse_event(&mut self, mouse: MouseEvent) -> bool {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left)
            || mouse.modifiers != KeyModifiers::NONE
            || self.queue_receipt_undo_task_count().is_none()
            || !self
                .queue_overlay_ui_state
                .take_receipt_undo_hit(mouse.column, mouse.row)
        {
            return false;
        }

        self.undo_latest_queue_registration();
        true
    }

    pub(super) fn queue_action_tasks(&self) -> Vec<QueueOverlayActionTask> {
        let runtime_projection = self
            .queue_overlay_ui_state
            .ready_runtime_projection()
            .cloned()
            .unwrap_or_else(|| self.planning_runtime_projection_snapshot());
        queue_action_tasks_from_projection(&runtime_projection)
    }

    pub(super) fn queue_authority_tokens_for_projection(
        projection: &PlanningRuntimeProjection,
        authority: &PlanningQueueAuthoritySnapshot,
    ) -> Option<BTreeMap<String, QueueOverlayAuthorityToken>> {
        queue_action_tasks_from_projection(projection)
            .iter()
            .map(|action_task| {
                authority
                    .tasks
                    .iter()
                    .find(|task| {
                        task.id == action_task.task_id && task.status == action_task.status
                    })
                    .map(|authority_task| {
                        (
                            action_task.task_id.clone(),
                            QueueOverlayAuthorityToken {
                                status: authority_task.status,
                                updated_at: authority_task.updated_at.clone(),
                            },
                        )
                    })
            })
            .collect()
    }

    pub(super) fn sync_queue_overlay_selection(&mut self) {
        let task_ids = self
            .queue_action_tasks()
            .into_iter()
            .map(|task| task.task_id)
            .collect::<Vec<_>>();
        self.queue_overlay_ui_state.sync(&task_ids);
    }

    #[cfg(test)]
    pub(super) fn bind_queue_overlay_authority_for_test(
        &mut self,
        planning_revision: i64,
        authority_tokens: BTreeMap<String, QueueOverlayAuthorityToken>,
    ) {
        let request = self.begin_queue_overlay_authority_load();
        let runtime_projection = self.planning_runtime_projection_snapshot();
        assert!(
            self.queue_overlay_ui_state.apply_authority_loaded(
                request,
                runtime_projection,
                planning_revision,
                authority_tokens,
            ),
            "test Queue authority must be a coherent ready snapshot"
        );
        self.sync_queue_overlay_selection();
    }
}

fn queue_authority_binding_is_coherent(
    runtime_projection: &PlanningRuntimeProjection,
    planning_revision: i64,
    authority_tokens: &BTreeMap<String, QueueOverlayAuthorityToken>,
) -> bool {
    let action_tasks = queue_action_tasks_from_projection(runtime_projection);
    runtime_projection.planning_revision() == Some(planning_revision)
        && action_tasks.len() == authority_tokens.len()
        && action_tasks.iter().all(|task| {
            authority_tokens
                .get(&task.task_id)
                .is_some_and(|token| token.status == task.status)
        })
}

fn queue_action_tasks_from_projection(
    runtime_projection: &PlanningRuntimeProjection,
) -> Vec<QueueOverlayActionTask> {
    let projection = PlanningApplicationProjection::from_runtime_projection(runtime_projection);
    projection
        .visible_tasks
        .into_iter()
        .map(QueueOverlayActionTask::from)
        .chain(
            projection
                .proposed_tasks
                .into_iter()
                .map(QueueOverlayActionTask::from),
        )
        .chain(
            projection
                .skipped_tasks
                .into_iter()
                .map(QueueOverlayActionTask::from),
        )
        .filter(|task| matches!(task.status, TaskStatus::Ready | TaskStatus::Proposed))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        QueueMutationContext, QueueMutationKind, QueueMutationUiState,
        QueueOverlayAuthorityScreenModel, QueueOverlayAuthorityToken, QueueOverlayUiState,
    };
    use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_projection;
    use crate::application::service::planning::{
        PlanningQueueCancellationRequest, PlanningRuntimeProjection,
    };
    use crate::domain::planning::TaskStatus;

    #[test]
    fn selection_tracks_task_identity_when_rows_change() {
        let mut state = QueueOverlayUiState::default();
        let tasks = vec!["task-a".to_string(), "task-b".to_string()];
        state.sync(&tasks);
        assert_eq!(state.selected_task_id(), Some("task-a"));

        state.set_feedback("old failure");
        state.move_selection(&tasks, 1);
        assert_eq!(state.selected_task_id(), Some("task-b"));
        assert_eq!(state.feedback(), None);

        state.sync(&["task-b".to_string(), "task-c".to_string()]);
        assert_eq!(state.selected_task_id(), Some("task-b"));

        state.sync(&["task-c".to_string()]);
        assert_eq!(state.selected_task_id(), Some("task-c"));

        state.sync(&[]);
        assert_eq!(state.selected_task_id(), None);
    }

    #[test]
    fn reset_drops_displayed_authority_tokens() {
        let mut state = QueueOverlayUiState::default();
        let request = state.begin_authority_load(QueueMutationContext {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        });
        state.sync(&["task-1".to_string(), "task-2".to_string()]);
        assert!(state.apply_authority_loaded(
            request,
            sample_planning_runtime_projection("context", "queue").with_planning_revision(Some(7)),
            7,
            BTreeMap::from([
                (
                    "task-1".to_string(),
                    QueueOverlayAuthorityToken {
                        status: TaskStatus::Ready,
                        updated_at: "2026-07-15T00:00:00Z".to_string(),
                    },
                ),
                (
                    "task-2".to_string(),
                    QueueOverlayAuthorityToken {
                        status: TaskStatus::Ready,
                        updated_at: "2026-07-15T00:00:01Z".to_string(),
                    },
                ),
            ]),
        ));
        assert!(state.selected_authority_token().is_some());

        state.reset();

        assert!(state.selected_authority_token().is_none());
    }

    #[test]
    fn authority_load_accepts_only_the_exact_request_and_context() {
        let mut state = QueueOverlayUiState::default();
        let pending = state.begin_authority_load(QueueMutationContext {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        });
        let stale = super::QueueOverlayAuthorityLoadRequest {
            request_id: pending.request_id,
            context: QueueMutationContext {
                workspace_directory: "/tmp/other".to_string(),
                active_thread_id: Some("thread-a".to_string()),
            },
        };

        assert!(!state.apply_authority_loaded(
            stale,
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(7)),
            7,
            BTreeMap::new(),
        ));
        assert!(matches!(
            state.authority_screen_model(),
            QueueOverlayAuthorityScreenModel::Loading { request_id }
                if request_id == pending.request_id
        ));
        assert!(state.apply_authority_loaded(
            pending.clone(),
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(7)),
            7,
            BTreeMap::new(),
        ));
        assert!(matches!(
            state.authority_screen_model(),
            QueueOverlayAuthorityScreenModel::Ready {
                request_id,
                planning_revision: 7,
            } if request_id == pending.request_id
        ));
    }

    #[test]
    fn reset_invalidates_in_flight_load_without_reusing_request_ids() {
        let mut state = QueueOverlayUiState::default();
        let stale = state.begin_authority_load(QueueMutationContext {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: None,
        });
        state.reset();
        assert!(!state.apply_authority_loaded(
            stale.clone(),
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(7)),
            7,
            BTreeMap::new(),
        ));

        let current = state.begin_authority_load(stale.context.clone());
        assert!(current.request_id > stale.request_id);
    }

    #[test]
    fn failed_load_is_an_immutable_read_only_screen_state() {
        let mut state = QueueOverlayUiState::default();
        let request = state.begin_authority_load(QueueMutationContext {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: None,
        });

        assert!(
            state.apply_authority_load_failed(request.clone(), "database unavailable".to_string())
        );
        assert!(matches!(
            state.authority_screen_model(),
            QueueOverlayAuthorityScreenModel::Failed {
                request_id,
                ref error,
            } if request_id == request.request_id && error == "database unavailable"
        ));
        assert!(!state.requires_authority_load_for(&request.context, None));
    }

    #[test]
    fn ready_authority_reloads_when_the_visible_planning_revision_drifts() {
        let mut state = QueueOverlayUiState::default();
        let request = state.begin_authority_load(QueueMutationContext {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        });
        assert!(state.apply_authority_loaded(
            request.clone(),
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(7)),
            7,
            BTreeMap::new(),
        ));

        assert!(!state.requires_authority_load_for(&request.context, Some(7)));
        assert!(state.requires_authority_load_for(&request.context, Some(8)));
        let mut next_thread = request.context.clone();
        next_thread.active_thread_id = Some("thread-b".to_string());
        assert!(state.requires_authority_load_for(&next_thread, Some(7)));
        assert_eq!(
            state
                .ready_runtime_projection()
                .and_then(PlanningRuntimeProjection::planning_revision),
            Some(7)
        );
    }

    #[test]
    fn ready_authority_rejects_revision_and_action_token_mismatches() {
        let mut state = QueueOverlayUiState::default();
        let request = state.begin_authority_load(QueueMutationContext {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        });

        assert!(!state.apply_authority_loaded(
            request.clone(),
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(8)),
            7,
            BTreeMap::new(),
        ));
        assert!(!state.apply_authority_loaded(
            request.clone(),
            sample_planning_runtime_projection("context", "queue").with_planning_revision(Some(7)),
            7,
            BTreeMap::new(),
        ));
        assert!(matches!(
            state.authority_screen_model(),
            QueueOverlayAuthorityScreenModel::Loading { request_id }
                if request_id == request.request_id
        ));
    }

    #[test]
    fn receipt_undo_hit_area_is_single_use() {
        let mut state = QueueOverlayUiState::default();
        state.bind_receipt_undo_hit_area(Some(ratatui::layout::Rect::new(4, 7, 14, 1)));

        assert!(!state.take_receipt_undo_hit(3, 7));
        assert!(state.take_receipt_undo_hit(4, 7));
        assert!(!state.take_receipt_undo_hit(4, 7));
        assert_eq!(state.receipt_undo_hit_area(), None);
    }

    #[test]
    fn queue_mutation_gate_allows_one_operation_and_ignores_stale_completion() {
        let mut state = QueueMutationUiState::default();
        let context = QueueMutationContext {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        };
        let request = PlanningQueueCancellationRequest {
            workspace_directory: context.workspace_directory.clone(),
            expected_planning_revision: 7,
            targets: Vec::new(),
        };
        let first = state
            .begin(
                context.clone(),
                QueueMutationKind::RemoveSelected,
                request.clone(),
                None,
            )
            .expect("first mutation should start");
        assert_eq!(first.operation_id, 1);
        assert_eq!(state.pending_operation_id(), Some(1));
        assert!(
            state
                .begin(
                    context.clone(),
                    QueueMutationKind::UndoLatestRegistration,
                    request.clone(),
                    None,
                )
                .is_none()
        );

        let mut stale = first.clone();
        stale.operation_id = 99;
        assert!(state.take_matching(&stale).is_none());
        let mut forged = first.clone();
        forged.request.expected_planning_revision = 8;
        assert!(state.take_matching(&forged).is_none());
        assert_eq!(state.pending_operation_id(), Some(1));
        assert_eq!(state.take_matching(&first), Some(first.clone()));
        assert_eq!(state.pending_operation_id(), None);

        let second = state
            .begin(
                context.clone(),
                QueueMutationKind::UndoLatestRegistration,
                request.clone(),
                None,
            )
            .expect("gate should reopen after matching completion");
        assert_eq!(second.operation_id, 2);
        assert!(state.take_matching(&first).is_none());
        assert_eq!(state.pending_operation_id(), Some(2));
        assert_eq!(state.take_matching(&second), Some(second));

        state.require_authority_refresh();
        assert!(
            state
                .begin(
                    context.clone(),
                    QueueMutationKind::RemoveSelected,
                    request.clone(),
                    None,
                )
                .is_none()
        );
        state.record_authority_refresh();
        let third = state
            .begin(context, QueueMutationKind::RemoveSelected, request, None)
            .expect("authority refresh should reopen the mutation gate");
        assert_eq!(third.operation_id, 3);
    }
}
