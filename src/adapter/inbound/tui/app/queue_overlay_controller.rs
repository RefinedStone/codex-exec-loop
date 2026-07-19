use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::application::service::planning::{
    PlanningQueueAuthorityProjection, PlanningQueueAuthoritySnapshot,
};
use crate::core::app::{
    AppCommand, AppEvent, QueueAuthorityLoadCorrelation, QueueMutationCorrelation,
    QueueMutationIntent, QueueMutationResult, QueueMutationTarget,
};

use super::{ConversationState, NativeTuiApp, ShellChromeEvent, ShellOverlay, queue_overlay_ui};

impl NativeTuiApp {
    pub(super) fn show_queue_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);
        self.start_queue_overlay_authority_load();
    }

    pub(super) fn start_queue_overlay_authority_load(&mut self) {
        if self.shell_overlay != ShellOverlay::Queue
            || self.pending_queue_mutation_operation_id().is_some()
        {
            return;
        }
        let context = self.current_queue_mutation_context();
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::LoadQueueAuthority {
                workspace_directory: context.workspace_directory,
                active_thread_id: context.active_thread_id,
            });
        let correlation = outcome.events.iter().find_map(|event| match event {
            AppEvent::QueueAuthorityLoadStarted { correlation } => Some(correlation.clone()),
            _ => None,
        });
        if let Some(correlation) = correlation {
            self.begin_queue_overlay_authority_load(correlation);
        }
        // Bind adapter-local context before an immediate completion is applied.
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn reconcile_queue_overlay_authority_context(&mut self) -> bool {
        if !self.queue_overlay_authority_load_required() {
            return false;
        }
        self.start_queue_overlay_authority_load();
        true
    }

    pub(super) fn apply_queue_overlay_authority_loaded(
        &mut self,
        correlation: QueueAuthorityLoadCorrelation,
        result: Result<
            Box<crate::core::app::QueueAuthoritySnapshot>,
            crate::core::app::QueueAuthorityLoadError,
        >,
    ) -> queue_overlay_ui::QueueOverlayAuthorityLoadCompletion {
        if self.shell_overlay != ShellOverlay::Queue {
            return queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Ignored;
        }
        let Some(request) = self
            .queue_overlay_ui_state
            .loading_request(&correlation)
            .cloned()
        else {
            return queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Ignored;
        };
        if !request.matches_context(&self.current_queue_mutation_context()) {
            return queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::ReloadRequired;
        }

        let authority = match result {
            Ok(authority) => *authority,
            Err(error) => {
                let error = self.tui_language.queue_overlay_authority_load_error(&error);
                self.queue_overlay_ui_state
                    .apply_authority_load_failed(request, error);
                return queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Applied;
            }
        };
        let authority = PlanningQueueAuthorityProjection {
            runtime_projection: authority.runtime_projection,
            queue_authority: PlanningQueueAuthoritySnapshot {
                planning_revision: authority.planning_revision,
                tasks: authority.tasks,
            },
        };
        let Some(planning_revision) = authority.runtime_projection.planning_revision() else {
            self.queue_overlay_ui_state.apply_authority_load_failed(
                request,
                self.tui_language
                    .queue_mutation_projection_revision_missing()
                    .to_string(),
            );
            return queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Applied;
        };
        let visible_revision = self
            .planning_runtime_projection_snapshot()
            .planning_revision();
        let receipt_revision = match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .latest_queue_mutation_receipt
                .as_ref()
                .map(|receipt| receipt.planning_revision),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        };
        if visible_revision.is_some_and(|revision| revision > planning_revision)
            || receipt_revision.is_some_and(|revision| revision > planning_revision)
        {
            return queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::ReloadRequired;
        }

        let (planning_revision, authority_tokens) =
            match self.reconcile_queue_authority_snapshot(&authority) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.queue_overlay_ui_state
                        .apply_authority_load_failed(request, error);
                    return queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Applied;
                }
            };
        self.reconcile_latest_queue_receipt_with_authority(&authority.queue_authority);
        if !self.queue_overlay_ui_state.apply_authority_loaded(
            request,
            authority.runtime_projection,
            planning_revision,
            authority_tokens,
        ) {
            return queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Ignored;
        }
        self.queue_mutation_ui_state.record_authority_refresh();
        self.sync_queue_overlay_selection();
        queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Applied
    }

    pub(super) fn handle_queue_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        let task_ids = self
            .queue_action_tasks()
            .into_iter()
            .map(|task| task.task_id)
            .collect::<Vec<_>>();
        match (key.code, key.modifiers) {
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.queue_overlay_ui_state.move_selection(&task_ids, -1);
            }
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.queue_overlay_ui_state.move_selection(&task_ids, 1);
            }
            (KeyCode::Char('x') | KeyCode::Delete, KeyModifiers::NONE) => {
                self.cancel_selected_queue_task();
            }
            (KeyCode::Char('u'), KeyModifiers::NONE) => {
                self.undo_latest_queue_registration();
            }
            _ => {}
        }
        true
    }

    pub(super) fn cancel_selected_queue_task(&mut self) {
        if let Some(operation_id) = self.pending_queue_mutation_operation_id() {
            self.queue_overlay_ui_state.set_feedback(
                self.tui_language
                    .queue_mutation_pending_feedback(operation_id),
            );
            return;
        }
        if self.reload_queue_overlay_authority_before_action() {
            return;
        }
        if let Some(feedback) = self.queue_overlay_authority_action_feedback() {
            self.queue_overlay_ui_state.set_feedback(feedback);
            return;
        }
        if self.queue_mutation_requires_authority_refresh() {
            self.queue_overlay_ui_state
                .set_feedback(self.tui_language.queue_mutation_refresh_required_feedback());
            return;
        }
        if let Some(reason) = self.queue_mutation_block_reason() {
            self.queue_overlay_ui_state
                .set_feedback(self.tui_language.queue_action_block_reason(reason));
            return;
        }
        let selected = self.queue_overlay_ui_state.selected_authority_token().map(
            |(revision, task_id, token)| {
                (
                    revision,
                    task_id.to_string(),
                    token.status,
                    token.updated_at.clone(),
                )
            },
        );
        let Some((planning_revision, task_id, status, updated_at)) = selected else {
            self.queue_overlay_ui_state.set_feedback(
                self.tui_language
                    .queue_mutation_selected_item_changed_feedback(),
            );
            return;
        };
        let context = self.current_queue_mutation_context();
        self.submit_queue_mutation(QueueMutationIntent {
            workspace_directory: context.workspace_directory,
            active_thread_id: context.active_thread_id,
            kind: queue_overlay_ui::QueueMutationKind::RemoveSelected,
            expected_planning_revision: planning_revision,
            targets: vec![QueueMutationTarget {
                task_id,
                expected_status: status,
                expected_updated_at: updated_at,
            }],
            receipt_at_start: self.latest_queue_mutation_receipt(),
        });
    }

    pub(super) fn undo_latest_queue_registration(&mut self) -> bool {
        if let Some(operation_id) = self.pending_queue_mutation_operation_id() {
            self.queue_overlay_ui_state.set_feedback(
                self.tui_language
                    .queue_mutation_pending_feedback(operation_id),
            );
            return false;
        }
        if self.reload_queue_overlay_authority_before_action() {
            return false;
        }
        if let Some(feedback) = self.queue_overlay_authority_action_feedback() {
            self.queue_overlay_ui_state.set_feedback(feedback);
            return false;
        }
        if self.queue_mutation_requires_authority_refresh() {
            self.queue_overlay_ui_state
                .set_feedback(self.tui_language.queue_mutation_refresh_required_feedback());
            return false;
        }
        if let Some(reason) = self.queue_receipt_undo_block_reason() {
            self.queue_overlay_ui_state
                .set_feedback(self.tui_language.queue_action_block_reason(reason));
            return false;
        }
        let receipt = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.latest_queue_mutation_receipt.clone()
            }
            ConversationState::Loading | ConversationState::Failed(_) => None,
        };
        let Some(receipt) = receipt else {
            self.queue_overlay_ui_state
                .set_feedback("No recent queue registration is available to undo.");
            return false;
        };
        let created_count = receipt.created_entries().count();
        if created_count == 0 {
            self.queue_overlay_ui_state
                .set_feedback("The latest receipt did not add removable queue items.");
            return false;
        }
        if !receipt.created_batch_is_cancellable() {
            self.queue_overlay_ui_state.set_feedback(
                "The latest registration changed after it was shown; review the queue before removing items.",
            );
            return false;
        }
        let targets = receipt
            .created_entries()
            .map(|entry| QueueMutationTarget {
                task_id: entry.task_id.clone(),
                expected_status: entry.after_status,
                expected_updated_at: entry.after_updated_at.clone(),
            })
            .collect::<Vec<_>>();
        let context = self.current_queue_mutation_context();
        self.submit_queue_mutation(QueueMutationIntent {
            workspace_directory: context.workspace_directory,
            active_thread_id: context.active_thread_id,
            kind: queue_overlay_ui::QueueMutationKind::UndoLatestRegistration,
            expected_planning_revision: receipt.planning_revision,
            targets,
            receipt_at_start: Some(receipt),
        })
    }

    fn reload_queue_overlay_authority_before_action(&mut self) -> bool {
        if !self.queue_overlay_authority_load_required() {
            return false;
        }
        self.start_queue_overlay_authority_load();
        self.queue_overlay_ui_state
            .set_feedback(self.tui_language.queue_overlay_authority_loading_feedback());
        true
    }

    fn queue_overlay_authority_action_feedback(&self) -> Option<String> {
        if self.shell_overlay != ShellOverlay::Queue {
            return None;
        }
        match self.queue_overlay_ui_state.authority_screen_model() {
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Idle
            | queue_overlay_ui::QueueOverlayAuthorityScreenModel::Loading { .. } => Some(
                self.tui_language
                    .queue_overlay_authority_loading_feedback()
                    .to_string(),
            ),
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Failed { request_id, error } => {
                Some(
                    self.tui_language
                        .queue_overlay_authority_failed_summary(request_id, &error),
                )
            }
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Ready { .. } => None,
        }
    }

    fn latest_queue_mutation_receipt(
        &self,
    ) -> Option<crate::domain::planning::PlanningQueueMutationReceipt> {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.latest_queue_mutation_receipt.clone()
            }
            ConversationState::Loading | ConversationState::Failed(_) => None,
        }
    }

    fn submit_queue_mutation(&mut self, intent: QueueMutationIntent) -> bool {
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::SubmitQueueMutation(Box::new(intent)));
        let started = outcome
            .events
            .iter()
            .any(|event| matches!(event, AppEvent::QueueMutationStarted { .. }));
        self.apply_core_dispatch_outcome(outcome);
        started
    }

    pub(super) fn apply_queue_mutation_started(&mut self, correlation: QueueMutationCorrelation) {
        if !self
            .queue_mutation_ui_state
            .record_started(correlation.clone())
        {
            return;
        }
        self.queue_overlay_ui_state.set_feedback(
            self.tui_language
                .queue_mutation_pending_feedback(correlation.generation),
        );
        self.clear_queue_receipt_undo_hit_area();
    }

    pub(super) fn apply_queue_mutation_completion(
        &mut self,
        correlation: QueueMutationCorrelation,
        completion: QueueMutationResult,
    ) {
        let Some(correlation) = self.queue_mutation_ui_state.take_matching(&correlation) else {
            return;
        };
        let operation_context = queue_overlay_ui::QueueMutationContext {
            workspace_directory: correlation.intent.workspace_directory.clone(),
            active_thread_id: correlation.intent.active_thread_id.clone(),
        };
        let current_context = self.current_queue_mutation_context();
        if !matches!(self.conversation_state, ConversationState::Ready(_))
            || current_context != operation_context
        {
            self.queue_overlay_ui_state.clear_feedback_if(
                &self
                    .tui_language
                    .queue_mutation_pending_feedback(correlation.generation),
            );
            if current_context.workspace_directory == operation_context.workspace_directory {
                self.queue_mutation_ui_state.require_authority_refresh();
                self.queue_overlay_ui_state.clear_authority_binding();
            }
            return;
        }

        let operation_id = correlation.generation;
        let authority = match completion.authority {
            Ok(authority) => PlanningQueueAuthorityProjection {
                runtime_projection: authority.runtime_projection,
                queue_authority: PlanningQueueAuthoritySnapshot {
                    planning_revision: authority.planning_revision,
                    tasks: authority.tasks,
                },
            },
            Err(refresh_error) => {
                self.queue_mutation_ui_state.require_authority_refresh();
                self.queue_overlay_ui_state.clear_authority_binding();
                let refresh_error = self
                    .tui_language
                    .queue_overlay_authority_load_error(&refresh_error);
                let feedback = match completion.mutation {
                    Ok(_) => self
                        .tui_language
                        .queue_mutation_committed_refresh_failed(operation_id, &refresh_error),
                    Err(mutation_error) => {
                        self.tui_language.queue_mutation_unresolved_refresh_failed(
                            operation_id,
                            &mutation_error,
                            &refresh_error,
                        )
                    }
                };
                self.surface_queue_mutation_feedback(feedback);
                return;
            }
        };

        let authority_confirms_cancellation = !correlation.intent.targets.is_empty()
            && correlation.intent.targets.iter().all(|target| {
                authority.queue_authority.tasks.iter().any(|task| {
                    task.id == target.task_id
                        && task.status == crate::domain::planning::TaskStatus::Cancelled
                })
            });
        if let Err(error) = self.apply_queue_mutation_authority_snapshot(&authority) {
            self.queue_mutation_ui_state.require_authority_refresh();
            self.queue_overlay_ui_state.clear_authority_binding();
            self.surface_queue_mutation_feedback(
                self.tui_language
                    .queue_mutation_reconcile_failed(operation_id, &error),
            );
            return;
        }

        let feedback = match completion.mutation {
            Ok(result) if authority_confirms_cancellation => {
                self.settle_correlated_queue_receipt(&correlation, &authority.queue_authority);
                self.tui_language.queue_mutation_acknowledged(
                    operation_id,
                    self.tui_language
                        .queue_mutation_success_label(correlation.intent.kind),
                    result.committed_task_ids.len(),
                    result.committed_planning_revision,
                )
            }
            Ok(_) => {
                self.reconcile_correlated_queue_receipt(&correlation, &authority.queue_authority);
                self.tui_language
                    .queue_mutation_acknowledged_without_confirmation(operation_id)
            }
            Err(error) if authority_confirms_cancellation => {
                self.settle_correlated_queue_receipt(&correlation, &authority.queue_authority);
                self.tui_language
                    .queue_mutation_authority_confirmed_after_error(operation_id, &error)
            }
            Err(error) => {
                self.reconcile_correlated_queue_receipt(&correlation, &authority.queue_authority);
                self.tui_language
                    .queue_mutation_rejected(operation_id, &error)
            }
        };
        self.surface_queue_mutation_feedback(feedback);
    }

    fn apply_queue_mutation_authority_snapshot(
        &mut self,
        authority: &queue_overlay_ui::QueueMutationAuthoritySnapshot,
    ) -> Result<(), String> {
        let (projection_revision, tokens) = self.reconcile_queue_authority_snapshot(authority)?;
        if !self.queue_overlay_ui_state.bind_authority_snapshot(
            authority.runtime_projection.clone(),
            projection_revision,
            tokens,
        ) {
            return Err(self
                .tui_language
                .queue_mutation_rows_mismatch_authority()
                .to_string());
        }
        self.queue_mutation_ui_state.record_authority_refresh();
        self.sync_queue_overlay_selection();
        Ok(())
    }

    fn reconcile_queue_authority_snapshot(
        &mut self,
        authority: &queue_overlay_ui::QueueMutationAuthoritySnapshot,
    ) -> Result<
        (
            i64,
            std::collections::BTreeMap<String, queue_overlay_ui::QueueOverlayAuthorityToken>,
        ),
        String,
    > {
        let projection_revision = authority
            .runtime_projection
            .planning_revision()
            .ok_or_else(|| {
                self.tui_language
                    .queue_mutation_projection_revision_missing()
                    .to_string()
            })?;
        if self
            .planning_runtime_projection_snapshot()
            .planning_revision()
            .is_some_and(|current_revision| current_revision > projection_revision)
        {
            return Err(self
                .tui_language
                .queue_mutation_completion_older_than_planning(projection_revision));
        }
        if matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if conversation
                    .latest_queue_mutation_receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.planning_revision > projection_revision)
        ) {
            return Err(self
                .tui_language
                .queue_mutation_completion_older_than_receipt(projection_revision));
        }
        if projection_revision != authority.queue_authority.planning_revision {
            return Err(self
                .tui_language
                .queue_mutation_projection_authority_revision_mismatch(
                    projection_revision,
                    authority.queue_authority.planning_revision,
                ));
        }
        let tokens = Self::queue_authority_tokens_for_projection(
            &authority.runtime_projection,
            &authority.queue_authority,
        )
        .ok_or_else(|| {
            self.tui_language
                .queue_mutation_rows_mismatch_authority()
                .to_string()
        })?;
        self.sync_ready_conversation_planning_runtime_projection(
            authority.runtime_projection.clone(),
        );
        Ok((projection_revision, tokens))
    }

    pub(super) fn settle_correlated_queue_receipt(
        &mut self,
        correlation: &QueueMutationCorrelation,
        authority: &crate::application::service::planning::PlanningQueueAuthoritySnapshot,
    ) {
        let ConversationState::Ready(conversation) = &mut self.conversation_state else {
            return;
        };
        let Some(current_receipt) = conversation.latest_queue_mutation_receipt.clone() else {
            return;
        };
        if correlation.intent.receipt_at_start.as_ref() != Some(&current_receipt) {
            if authority.planning_revision >= current_receipt.planning_revision {
                conversation.latest_queue_mutation_receipt = Some(
                    Self::queue_receipt_reconciled_with_authority(&current_receipt, authority),
                );
            }
            return;
        }
        let receipt_at_start = correlation
            .intent
            .receipt_at_start
            .as_ref()
            .expect("matching captured receipt should exist");
        let invalidates_receipt = correlation.intent.kind
            == queue_overlay_ui::QueueMutationKind::UndoLatestRegistration
            || correlation.intent.targets.iter().any(|target| {
                receipt_at_start
                    .created_entries()
                    .any(|entry| entry.task_id == target.task_id)
            });
        if invalidates_receipt {
            conversation.latest_queue_mutation_receipt = None;
        } else {
            conversation.latest_queue_mutation_receipt = Some(
                Self::queue_receipt_reconciled_with_authority(receipt_at_start, authority),
            );
        }
    }

    pub(super) fn reconcile_correlated_queue_receipt(
        &mut self,
        correlation: &QueueMutationCorrelation,
        authority: &crate::application::service::planning::PlanningQueueAuthoritySnapshot,
    ) {
        let ConversationState::Ready(conversation) = &mut self.conversation_state else {
            return;
        };
        let Some(current_receipt) = conversation.latest_queue_mutation_receipt.clone() else {
            return;
        };
        if correlation.intent.receipt_at_start.as_ref() != Some(&current_receipt) {
            if authority.planning_revision >= current_receipt.planning_revision {
                conversation.latest_queue_mutation_receipt = Some(
                    Self::queue_receipt_reconciled_with_authority(&current_receipt, authority),
                );
            }
            return;
        }
        let receipt_at_start = correlation
            .intent
            .receipt_at_start
            .as_ref()
            .expect("matching captured receipt should exist");

        conversation.latest_queue_mutation_receipt = Some(
            Self::queue_receipt_reconciled_with_authority(receipt_at_start, authority),
        );
    }

    fn surface_queue_mutation_feedback(&mut self, feedback: String) {
        self.queue_overlay_ui_state.set_feedback(feedback.clone());
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.status_text = feedback.clone();
            conversation.append_status_message(feedback);
        }
    }

    pub(super) fn reconcile_latest_queue_receipt_with_authority(
        &mut self,
        authority: &crate::application::service::planning::PlanningQueueAuthoritySnapshot,
    ) {
        let ConversationState::Ready(conversation) = &mut self.conversation_state else {
            return;
        };
        let Some(receipt) = conversation.latest_queue_mutation_receipt.as_ref() else {
            return;
        };
        conversation.latest_queue_mutation_receipt = Some(
            Self::queue_receipt_reconciled_with_authority(receipt, authority),
        );
    }

    fn queue_receipt_reconciled_with_authority(
        receipt: &crate::domain::planning::PlanningQueueMutationReceipt,
        authority: &crate::application::service::planning::PlanningQueueAuthoritySnapshot,
    ) -> crate::domain::planning::PlanningQueueMutationReceipt {
        let mut reconciled = receipt.clone();
        reconciled.planning_revision = authority.planning_revision;
        for entry in &mut reconciled.entries {
            if entry.mutation_kind != crate::domain::planning::PlanningQueueMutationKind::Created {
                continue;
            }
            entry.unchanged_since_mutation = authority.tasks.iter().any(|task| {
                task.id == entry.task_id
                    && task.status == entry.after_status
                    && task.updated_at == entry.after_updated_at
            });
        }
        reconciled
    }
}
