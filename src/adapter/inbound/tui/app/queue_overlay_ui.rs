use std::collections::BTreeMap;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;
use crate::application::port::inbound::planning_projection_port::{
    PlanningApplicationProjection, PlanningApplicationQueueTask, PlanningApplicationSkippedTask,
};
pub(super) use crate::core::app::QueueMutationKind;
use crate::core::app::{
    QueueAuthorityLoadCorrelation, QueueMutationCorrelation, QueueMutationIntent,
};
use crate::domain::planning::{
    RuntimeProjection as PlanningRuntimeProjection, TaskDefinition, TaskStatus,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningQueueAuthoritySnapshot {
    pub(super) planning_revision: i64,
    pub(super) tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueMutationAuthoritySnapshot {
    pub(super) runtime_projection: PlanningRuntimeProjection,
    pub(super) queue_authority: PlanningQueueAuthoritySnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayAuthorityLoadRequest {
    pub(super) correlation: QueueAuthorityLoadCorrelation,
}

impl QueueOverlayAuthorityLoadRequest {
    pub(super) fn matches_context(&self, context: &QueueMutationContext) -> bool {
        self.correlation.workspace_directory == context.workspace_directory
            && self.correlation.active_thread_id == context.active_thread_id
    }
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
    pub(super) armed_remove_task_id: Option<String>,
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
    pending: Option<QueueMutationCorrelation>,
    authority_refresh_required: bool,
}

impl QueueMutationUiState {
    pub(super) fn record_started(&mut self, correlation: QueueMutationCorrelation) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some(correlation);
        true
    }

    pub(super) fn pending_operation_id(&self) -> Option<u64> {
        self.pending
            .as_ref()
            .map(|correlation| correlation.generation)
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
        completed: &QueueMutationCorrelation,
    ) -> Option<QueueMutationCorrelation> {
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
    authority_projection: QueueOverlayAuthorityProjectionState,
    selected_task_id: Option<String>,
    armed_remove_intent: Option<QueueMutationIntent>,
    feedback: Option<String>,
    receipt_undo_hit_area: Option<Rect>,
}

impl Default for QueueOverlayUiState {
    fn default() -> Self {
        Self {
            authority_projection: QueueOverlayAuthorityProjectionState::Idle,
            selected_task_id: None,
            armed_remove_intent: None,
            feedback: None,
            receipt_undo_hit_area: None,
        }
    }
}

impl QueueOverlayUiState {
    pub(super) fn begin_authority_load(
        &mut self,
        correlation: QueueAuthorityLoadCorrelation,
    ) -> QueueOverlayAuthorityLoadRequest {
        let request = QueueOverlayAuthorityLoadRequest { correlation };
        self.authority_projection = QueueOverlayAuthorityProjectionState::Loading(request.clone());
        self.armed_remove_intent = None;
        self.feedback = None;
        request
    }

    pub(super) fn is_loading_request(&self, request: &QueueOverlayAuthorityLoadRequest) -> bool {
        matches!(
            &self.authority_projection,
            QueueOverlayAuthorityProjectionState::Loading(pending) if pending == request
        )
    }

    pub(super) fn loading_request(
        &self,
        correlation: &QueueAuthorityLoadCorrelation,
    ) -> Option<&QueueOverlayAuthorityLoadRequest> {
        match &self.authority_projection {
            QueueOverlayAuthorityProjectionState::Loading(request)
                if request.correlation == *correlation =>
            {
                Some(request)
            }
            QueueOverlayAuthorityProjectionState::Idle
            | QueueOverlayAuthorityProjectionState::Loading(_)
            | QueueOverlayAuthorityProjectionState::Ready { .. }
            | QueueOverlayAuthorityProjectionState::Failed { .. } => None,
        }
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
                !request.matches_context(context)
            }
            QueueOverlayAuthorityProjectionState::Ready {
                request,
                planning_revision,
                ..
            } => {
                !request.matches_context(context)
                    || Some(*planning_revision) != visible_planning_revision
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
        self.armed_remove_intent = None;
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
                    request_id: request.correlation.generation,
                }
            }
            QueueOverlayAuthorityProjectionState::Ready {
                request,
                planning_revision,
                ..
            } => QueueOverlayAuthorityScreenModel::Ready {
                request_id: request.correlation.generation,
                planning_revision: *planning_revision,
            },
            QueueOverlayAuthorityProjectionState::Failed { request, error } => {
                QueueOverlayAuthorityScreenModel::Failed {
                    request_id: request.correlation.generation,
                    error: error.clone(),
                }
            }
        }
    }

    pub(super) fn selected_task_id(&self) -> Option<&str> {
        self.selected_task_id.as_deref()
    }

    pub(super) fn armed_remove_task_id(&self) -> Option<&str> {
        let intent = self.armed_remove_intent.as_ref()?;
        (intent.kind == QueueMutationKind::RemoveSelected && intent.targets.len() == 1)
            .then(|| intent.targets[0].task_id.as_str())
    }

    pub(super) fn arm_remove_task(&mut self, intent: QueueMutationIntent) {
        self.armed_remove_intent = Some(intent);
        self.feedback = None;
    }

    pub(super) fn remove_is_armed_for(&self, intent: &QueueMutationIntent) -> bool {
        self.armed_remove_intent.as_ref() == Some(intent)
    }

    pub(super) fn disarm_remove_task(&mut self) {
        self.armed_remove_intent = None;
    }

    pub(super) fn feedback(&self) -> Option<&str> {
        self.feedback.as_deref()
    }

    pub(super) fn set_feedback(&mut self, feedback: impl Into<String>) {
        self.feedback = Some(feedback.into());
    }

    pub(super) fn clear_feedback_if(&mut self, feedback: &str) {
        if self.feedback.as_deref() == Some(feedback) {
            self.feedback = None;
        }
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
            self.armed_remove_intent = None;
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
        self.armed_remove_intent = None;
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
        *self = Self::default();
    }

    pub(super) fn sync(&mut self, task_ids: &[String]) {
        let previous_selection = self.selected_task_id.clone();
        if task_ids.is_empty() {
            self.selected_task_id = None;
            self.armed_remove_intent = None;
            return;
        }
        if self
            .selected_task_id
            .as_ref()
            .is_none_or(|selected| !task_ids.contains(selected))
        {
            self.selected_task_id = task_ids.first().cloned();
        }
        if self.selected_task_id != previous_selection {
            self.armed_remove_intent = None;
        }
    }

    pub(super) fn move_selection(&mut self, task_ids: &[String], delta: isize) {
        self.armed_remove_intent = None;
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
        correlation: QueueAuthorityLoadCorrelation,
    ) -> QueueOverlayAuthorityLoadRequest {
        self.planning
            .queue_overlay_ui_state
            .begin_authority_load(correlation)
    }

    pub(super) fn queue_overlay_authority_load_required(&self) -> bool {
        self.shell.chrome.shell_overlay == ShellOverlay::Queue
            && self.pending_queue_mutation_operation_id().is_none()
            && self
                .planning
                .queue_overlay_ui_state
                .requires_authority_load_for(
                    &self.current_queue_mutation_context(),
                    self.planning_runtime_projection_snapshot()
                        .planning_revision(),
                )
    }

    #[cfg(test)]
    pub(super) fn queue_overlay_screen_model(&self) -> QueueOverlayScreenModel {
        let runtime_projection = self.planning_runtime_projection_snapshot();
        self.queue_overlay_screen_model_from_projection(
            &runtime_projection,
            self.parallel_mode_enabled(),
        )
    }

    pub(super) fn queue_overlay_screen_model_from_projection(
        &self,
        runtime_projection: &PlanningRuntimeProjection,
        parallel_mode_enabled: bool,
    ) -> QueueOverlayScreenModel {
        let pending_operation_id = self.pending_queue_mutation_operation_id();
        let authority_refresh_required = self.queue_mutation_requires_authority_refresh();
        let authority = self
            .planning
            .queue_overlay_ui_state
            .authority_screen_model();
        let authority_snapshot_changed =
            matches!(&authority, QueueOverlayAuthorityScreenModel::Ready { .. })
                && self
                    .planning
                    .queue_overlay_ui_state
                    .requires_authority_load_for(
                        &self.current_queue_mutation_context(),
                        runtime_projection.planning_revision(),
                    );
        let remove_block_reason = authority_snapshot_changed
            .then_some(QueueActionBlockReason::AuthoritySnapshotChanged)
            .or_else(|| self.queue_mutation_block_reason_for_parallel_mode(parallel_mode_enabled))
            .or_else(|| {
                (matches!(&authority, QueueOverlayAuthorityScreenModel::Ready { .. })
                    && self
                        .planning
                        .queue_overlay_ui_state
                        .selected_authority_token()
                        .is_none())
                .then_some(QueueActionBlockReason::SelectedItemUnavailable)
            });
        let undo_block_reason = authority_snapshot_changed
            .then_some(QueueActionBlockReason::AuthoritySnapshotChanged)
            .or_else(|| {
                self.queue_receipt_undo_block_reason_for_parallel_mode(parallel_mode_enabled)
            });
        let latest_registration_undo_available = matches!(
            (&self.conversation.lifecycle.conversation_state, &authority),
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
        let conversation = match &self.conversation.lifecycle.conversation_state {
            ConversationState::Loading => QueueOverlayConversationScreenModel::Loading,
            ConversationState::Failed(message) => {
                QueueOverlayConversationScreenModel::Failed(message.clone())
            }
            ConversationState::Ready(conversation) => QueueOverlayConversationScreenModel::Ready {
                runtime_projection: Box::new(
                    self.planning
                        .queue_overlay_ui_state
                        .ready_runtime_projection()
                        .cloned()
                        .unwrap_or_else(|| runtime_projection.clone()),
                ),
                planning_notice: conversation
                    .planning_notice_summary(QUEUE_OVERLAY_SCREEN_DETAIL_LIMIT),
            },
        };
        QueueOverlayScreenModel {
            conversation,
            authority,
            selected_task_id: self
                .planning
                .queue_overlay_ui_state
                .selected_task_id()
                .map(str::to_string),
            armed_remove_task_id: self
                .planning
                .queue_overlay_ui_state
                .armed_remove_task_id()
                .map(str::to_string),
            feedback: self
                .planning
                .queue_overlay_ui_state
                .feedback()
                .map(str::to_string),
            pending_operation_id,
            authority_refresh_required,
            latest_registration_undo_available,
            remove_block_reason,
            undo_block_reason,
            planning_worker_host_detail: self
                .planning
                .planning_worker_panel_state
                .current()
                .last_host_detail
                .as_deref()
                .map(|detail| {
                    detail
                        .chars()
                        .take(QUEUE_OVERLAY_SCREEN_DETAIL_LIMIT)
                        .collect()
                }),
            tui_language: self.shell.tui_language,
        }
    }

    pub(super) fn pending_queue_mutation_operation_id(&self) -> Option<u64> {
        self.planning.queue_mutation_ui_state.pending_operation_id()
    }

    pub(super) fn queue_mutation_requires_authority_refresh(&self) -> bool {
        self.planning
            .queue_mutation_ui_state
            .authority_refresh_required()
    }

    pub(super) fn current_queue_mutation_context(&self) -> QueueMutationContext {
        let active_thread_id = match &self.conversation.lifecycle.conversation_state {
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
        self.queue_mutation_block_reason_for_parallel_mode(self.parallel_mode_enabled())
    }

    fn queue_mutation_block_reason_for_parallel_mode(
        &self,
        parallel_mode_enabled: bool,
    ) -> Option<QueueActionBlockReason> {
        if parallel_mode_enabled {
            return Some(QueueActionBlockReason::ParallelModeOwnsTaskLeases);
        }
        match &self.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation)
                if conversation.has_post_turn_settlement_in_flight() =>
            {
                Some(QueueActionBlockReason::PostTurnPlanningInFlight)
            }
            ConversationState::Ready(conversation)
                if conversation.auto_follow_state().has_live_activity() =>
            {
                Some(QueueActionBlockReason::PostTurnPlanningInFlight)
            }
            ConversationState::Ready(conversation)
                if matches!(
                    conversation.input_state(),
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
        self.queue_receipt_undo_block_reason_for_parallel_mode(self.parallel_mode_enabled())
    }

    fn queue_receipt_undo_block_reason_for_parallel_mode(
        &self,
        parallel_mode_enabled: bool,
    ) -> Option<QueueActionBlockReason> {
        if parallel_mode_enabled {
            return Some(QueueActionBlockReason::ParallelModeOwnsTaskLeases);
        }
        match &self.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation)
                if conversation.has_post_turn_settlement_in_flight()
                    || conversation.auto_follow_state().has_live_activity() =>
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
        self.queue_receipt_undo_task_count_for_parallel_mode(self.parallel_mode_enabled())
    }

    pub(super) fn queue_receipt_undo_task_count_for_parallel_mode(
        &self,
        parallel_mode_enabled: bool,
    ) -> Option<usize> {
        if self.pending_queue_mutation_operation_id().is_some()
            || self.queue_mutation_requires_authority_refresh()
            || self.shell.chrome.shell_overlay != ShellOverlay::Hidden
            || self.is_exit_confirmation_visible()
            || self.is_turn_steer_confirmation_visible()
            || self
                .queue_receipt_undo_block_reason_for_parallel_mode(parallel_mode_enabled)
                .is_some()
        {
            return None;
        }
        let ConversationState::Ready(conversation) =
            &self.conversation.lifecycle.conversation_state
        else {
            return None;
        };
        let receipt = conversation.latest_queue_mutation_receipt.as_ref()?;
        receipt
            .created_batch_is_cancellable()
            .then(|| receipt.created_entries().count())
    }

    pub(super) fn clear_queue_receipt_undo_hit_area(&mut self) {
        self.planning
            .queue_overlay_ui_state
            .clear_receipt_undo_hit_area();
    }

    pub(super) fn handle_queue_receipt_mouse_event(&mut self, mouse: MouseEvent) -> bool {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left)
            || mouse.modifiers != KeyModifiers::NONE
            || self.queue_receipt_undo_task_count().is_none()
            || !self
                .planning
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
            .planning
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
        self.planning.queue_overlay_ui_state.sync(&task_ids);
    }

    #[cfg(test)]
    pub(super) fn bind_queue_overlay_authority_for_test(
        &mut self,
        planning_revision: i64,
        authority_tokens: BTreeMap<String, QueueOverlayAuthorityToken>,
    ) {
        let context = self.current_queue_mutation_context();
        let request = self.begin_queue_overlay_authority_load(QueueAuthorityLoadCorrelation::new(
            u64::MAX,
            context.workspace_directory,
            context.active_thread_id,
        ));
        let runtime_projection = self.planning_runtime_projection_snapshot();
        assert!(
            self.planning.queue_overlay_ui_state.apply_authority_loaded(
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
    use crate::application::service::planning::PlanningRuntimeProjection;
    use crate::core::app::{
        QueueAuthorityLoadCorrelation, QueueMutationCorrelation, QueueMutationIntent,
        QueueMutationTarget,
    };
    use crate::domain::planning::TaskStatus;

    fn context(workspace_directory: &str, active_thread_id: Option<&str>) -> QueueMutationContext {
        QueueMutationContext {
            workspace_directory: workspace_directory.to_string(),
            active_thread_id: active_thread_id.map(str::to_string),
        }
    }

    fn correlation_for(
        generation: u64,
        context: &QueueMutationContext,
    ) -> QueueAuthorityLoadCorrelation {
        QueueAuthorityLoadCorrelation::new(
            generation,
            context.workspace_directory.clone(),
            context.active_thread_id.clone(),
        )
    }

    fn begin_load(
        state: &mut QueueOverlayUiState,
        generation: u64,
        context: QueueMutationContext,
    ) -> super::QueueOverlayAuthorityLoadRequest {
        state.begin_authority_load(correlation_for(generation, &context))
    }

    fn mutation_correlation(
        generation: u64,
        context: &QueueMutationContext,
        kind: QueueMutationKind,
        expected_planning_revision: i64,
    ) -> QueueMutationCorrelation {
        QueueMutationCorrelation::new(
            generation,
            QueueMutationIntent {
                workspace_directory: context.workspace_directory.clone(),
                active_thread_id: context.active_thread_id.clone(),
                kind,
                expected_planning_revision,
                targets: Vec::new(),
                receipt_at_start: None,
            },
        )
    }

    fn remove_intent(task_id: &str, updated_at: &str) -> QueueMutationIntent {
        QueueMutationIntent {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-a".to_string()),
            kind: QueueMutationKind::RemoveSelected,
            expected_planning_revision: 7,
            targets: vec![QueueMutationTarget {
                task_id: task_id.to_string(),
                expected_status: TaskStatus::Ready,
                expected_updated_at: updated_at.to_string(),
            }],
            receipt_at_start: None,
        }
    }

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
    fn remove_confirmation_is_bound_to_the_exact_authority_intent() {
        let mut state = QueueOverlayUiState::default();
        let tasks = vec!["task-a".to_string(), "task-b".to_string()];
        state.sync(&tasks);
        let armed = remove_intent("task-a", "2026-07-15T00:00:00Z");
        state.arm_remove_task(armed.clone());

        assert_eq!(state.armed_remove_task_id(), Some("task-a"));
        assert!(state.remove_is_armed_for(&armed));
        let mut changed_context = armed.clone();
        changed_context.active_thread_id = Some("thread-b".to_string());
        let mut changed_revision = armed.clone();
        changed_revision.expected_planning_revision = 8;
        let mut changed_status = armed.clone();
        changed_status.targets[0].expected_status = TaskStatus::Proposed;
        for changed in [
            changed_context,
            changed_revision,
            changed_status,
            remove_intent("task-a", "2026-07-15T00:00:01Z"),
        ] {
            assert!(!state.remove_is_armed_for(&changed));
        }

        state.move_selection(&tasks, 1);
        assert_eq!(state.selected_task_id(), Some("task-b"));
        assert_eq!(state.armed_remove_task_id(), None);
    }

    #[test]
    fn authority_refresh_and_selection_repair_disarm_remove_confirmation() {
        let mut state = QueueOverlayUiState::default();
        state.sync(&["task-a".to_string()]);
        state.arm_remove_task(remove_intent("task-a", "2026-07-15T00:00:00Z"));

        let _request = begin_load(&mut state, 1, context("/tmp/workspace", Some("thread-a")));
        assert_eq!(state.armed_remove_task_id(), None);

        state.arm_remove_task(remove_intent("task-a", "2026-07-15T00:00:00Z"));
        state.sync(&["task-b".to_string()]);
        assert_eq!(state.selected_task_id(), Some("task-b"));
        assert_eq!(state.armed_remove_task_id(), None);
    }

    #[test]
    fn move_selection_never_panics_on_empty_or_unknown_task_lists() {
        // The queue overlay can be keyed while a catalog refresh empties the task
        // list or replaces it with different ids. Selection movement must stay a
        // safe no-op instead of panicking on the empty-slice subtraction.
        let mut state = QueueOverlayUiState::default();
        state.move_selection(&[], 1);
        assert_eq!(state.selected_task_id(), None);

        let tasks = vec!["task-a".to_string()];
        state.sync(&tasks);
        state.arm_remove_task(remove_intent("task-a", "2026-07-15T00:00:00Z"));

        // Unknown selected id falls back to row 0 and clamps at the last row.
        state.move_selection(&tasks, 5);
        assert_eq!(state.selected_task_id(), Some("task-a"));
        state.move_selection(&tasks, -5);
        assert_eq!(state.selected_task_id(), Some("task-a"));
    }

    #[test]
    fn reset_drops_displayed_authority_tokens() {
        let mut state = QueueOverlayUiState::default();
        let request = begin_load(&mut state, 1, context("/tmp/workspace", Some("thread-a")));
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
    fn authority_load_accepts_only_the_exact_correlation() {
        let mut state = QueueOverlayUiState::default();
        let pending = begin_load(&mut state, 1, context("/tmp/workspace", Some("thread-a")));
        let stale_context = context("/tmp/other", Some("thread-a"));
        let stale = super::QueueOverlayAuthorityLoadRequest {
            correlation: correlation_for(1, &stale_context),
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
                if request_id == pending.correlation.generation
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
            } if request_id == pending.correlation.generation
        ));
    }

    #[test]
    fn reset_and_same_context_reopen_reject_the_stale_generation() {
        let mut state = QueueOverlayUiState::default();
        let stale_context = context("/tmp/workspace", None);
        let stale = begin_load(&mut state, 1, stale_context.clone());
        state.reset();
        assert!(!state.apply_authority_loaded(
            stale.clone(),
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(7)),
            7,
            BTreeMap::new(),
        ));

        let current = begin_load(&mut state, 2, stale_context);
        assert!(!state.apply_authority_loaded(
            stale,
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(7)),
            7,
            BTreeMap::new(),
        ));
        assert!(state.is_loading_request(&current));
    }

    #[test]
    fn failed_load_is_an_immutable_read_only_screen_state() {
        let mut state = QueueOverlayUiState::default();
        let request_context = context("/tmp/workspace", None);
        let request = begin_load(&mut state, 1, request_context.clone());

        assert!(
            state.apply_authority_load_failed(request.clone(), "database unavailable".to_string())
        );
        assert!(matches!(
            state.authority_screen_model(),
            QueueOverlayAuthorityScreenModel::Failed {
                request_id,
                ref error,
            } if request_id == request.correlation.generation && error == "database unavailable"
        ));
        assert!(!state.requires_authority_load_for(&request_context, None));
    }

    #[test]
    fn ready_authority_reloads_when_the_visible_planning_revision_drifts() {
        let mut state = QueueOverlayUiState::default();
        let request_context = context("/tmp/workspace", Some("thread-a"));
        let request = begin_load(&mut state, 1, request_context.clone());
        assert!(state.apply_authority_loaded(
            request.clone(),
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(7)),
            7,
            BTreeMap::new(),
        ));

        assert!(!state.requires_authority_load_for(&request_context, Some(7)));
        assert!(state.requires_authority_load_for(&request_context, Some(8)));
        let mut next_thread = request_context;
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
        let request = begin_load(&mut state, 1, context("/tmp/workspace", Some("thread-a")));

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
                if request_id == request.correlation.generation
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
    fn queue_mutation_projection_accepts_one_core_correlation_and_ignores_stale_completion() {
        let mut state = QueueMutationUiState::default();
        let context = QueueMutationContext {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-a".to_string()),
        };
        let first = mutation_correlation(1, &context, QueueMutationKind::RemoveSelected, 7);
        assert!(state.record_started(first.clone()));
        assert_eq!(state.pending_operation_id(), Some(1));
        assert!(!state.record_started(mutation_correlation(
            2,
            &context,
            QueueMutationKind::UndoLatestRegistration,
            7,
        )));

        let stale = mutation_correlation(99, &context, QueueMutationKind::RemoveSelected, 7);
        assert!(state.take_matching(&stale).is_none());
        let mut forged = first.clone();
        forged.intent.expected_planning_revision = 8;
        assert!(state.take_matching(&forged).is_none());
        assert_eq!(state.pending_operation_id(), Some(1));
        assert_eq!(state.take_matching(&first), Some(first.clone()));
        assert_eq!(state.pending_operation_id(), None);

        let second =
            mutation_correlation(2, &context, QueueMutationKind::UndoLatestRegistration, 7);
        assert!(state.record_started(second.clone()));
        assert!(state.take_matching(&first).is_none());
        assert_eq!(state.pending_operation_id(), Some(2));
        assert_eq!(state.take_matching(&second), Some(second));
    }
}
