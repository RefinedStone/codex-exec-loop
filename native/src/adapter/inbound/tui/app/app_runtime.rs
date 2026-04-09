use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::followup_template_service::FollowupTemplateService;
use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
use crate::application::service::planning_init_service::PlanningInitService;
use crate::application::service::planning_prompt_service::PlanningPromptService;
use crate::application::service::planning_reconciliation_service::{
    PlanningReconciliationResult, PlanningReconciliationService, PlanningRepairRetryReason,
    build_planning_repair_prompt,
};
use crate::application::service::planning_validation_service::PlanningValidationService;
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::github_review::GithubPullRequestPollResult;
use crate::domain::planning::{DIRECTIONS_FILE_PATH, TASK_LEDGER_FILE_PATH};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::startup_diagnostics::StartupDiagnostics;

use super::conversation_model::PlanningRepairState;
use super::{
    ActiveTurnPlanningSnapshot, AutoFollowupSubmitContext, ConversationInputEvent,
    ConversationIntentEffect, ConversationIntentEvent, ConversationIntentMode,
    ConversationIntentState, ConversationLifecycleEffect, ConversationLifecycleEvent,
    ConversationLifecycleState, ConversationRuntimeEffect, ConversationRuntimeEvent,
    ConversationState, ConversationViewModel, ExitConfirmationState, FollowupControlEffect,
    FollowupControlEvent, FollowupOverlayUiEvent, FollowupOverlayUiState, InlineShellCommand,
    NativeTuiApp, PlanningRepairSubmitContext, PromptOrigin, SESSION_PAGE_SIZE,
    SessionOverlayUiState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
    ShellOverlay, StartupState, TranscriptViewportState, reduce_conversation_input,
    reduce_conversation_intents, reduce_conversation_lifecycle, reduce_conversation_runtime,
    reduce_followup_controls, reduce_followup_overlay_ui, reduce_shell_chrome,
    startup_ascii_art_enabled_from_environment,
};
use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};

#[derive(Debug, Clone)]
pub(super) enum BackgroundMessage {
    StartupLoaded(Result<StartupDiagnostics, String>),
    SessionsLoaded(Result<RecentSessions, String>),
    ConversationLoaded(Result<ConversationSnapshot, String>),
    ConversationStream(ConversationStreamEvent),
    GithubReviewPollLoaded(Result<GithubPullRequestPollResult, String>),
}

const MAX_PLANNING_REPAIR_ATTEMPTS: usize = 2;

#[derive(Debug, Clone)]
struct QueuedPlanningRepairPrompt {
    prompt: String,
    queued_from_turn_id: String,
    attempt_number: usize,
    max_attempts: usize,
}

#[derive(Debug, Clone, Default)]
struct PlanningRepairResolution {
    queued_prompt: Option<QueuedPlanningRepairPrompt>,
    notices: Vec<String>,
    block_reason: Option<String>,
}

impl NativeTuiApp {
    pub(super) fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        followup_template_service: FollowupTemplateService,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let workspace_directory = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let planning_workspace_port = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let planning_prompt_service = PlanningPromptService::new(
            planning_workspace_port.clone(),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        );
        let planning_reconciliation_service = PlanningReconciliationService::new(
            planning_workspace_port.clone(),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        );
        let mut initial_conversation = ConversationViewModel::new_draft(
            workspace_directory.clone(),
            followup_template_service.load_catalog(&workspace_directory),
        );
        initial_conversation.replace_planning_prompt_context(
            planning_prompt_service
                .load_prompt_context(&workspace_directory)
                .unwrap_or_else(|error| {
                    super::shell_controller::planning_prompt_context_load_failed(error.to_string())
                }),
        );
        Self {
            shell_overlay: ShellOverlay::Hidden,
            exit_confirmation_state: ExitConfirmationState::Hidden,
            startup_state: StartupState::Idle,
            session_state: SessionState::Idle,
            conversation_state: ConversationState::Ready(initial_conversation),
            selected_session_index: 0,
            session_overlay_ui_state: SessionOverlayUiState::new(SESSION_PAGE_SIZE),
            followup_overlay_ui_state: FollowupOverlayUiState::default(),
            transcript_viewport_state: TranscriptViewportState::default(),
            active_session: None,
            startup_service,
            session_service,
            conversation_service,
            followup_template_service,
            planning_init_service: PlanningInitService::new(
                planning_workspace_port.clone(),
                PlanningBootstrapService::new(),
                PlanningValidationService::new(),
            ),
            planning_prompt_service,
            planning_reconciliation_service,
            active_turn_planning_snapshot: None,
            github_review_poller_service: None,
            github_review_polling_state: super::GithubReviewPollingState::Disabled,
            show_startup_ascii_art: startup_ascii_art_enabled_from_environment(),
            tx,
            rx,
        }
    }

    fn take_shell_chrome_state(&mut self) -> ShellChromeState {
        ShellChromeState {
            shell_overlay: self.shell_overlay,
            exit_confirmation_state: self.exit_confirmation_state,
            startup_state: std::mem::replace(&mut self.startup_state, StartupState::Idle),
            session_state: std::mem::replace(&mut self.session_state, SessionState::Idle),
            selected_session_index: self.selected_session_index,
        }
    }

    fn apply_shell_chrome_state(&mut self, state: ShellChromeState) {
        self.shell_overlay = state.shell_overlay;
        self.exit_confirmation_state = state.exit_confirmation_state;
        self.startup_state = state.startup_state;
        self.session_state = state.session_state;
        self.selected_session_index = state.selected_session_index;
    }

    pub(super) fn dispatch_shell_chrome(&mut self, event: ShellChromeEvent) {
        let reduction = reduce_shell_chrome(self.take_shell_chrome_state(), event);
        self.apply_shell_chrome_state(reduction.state);
        for effect in reduction.effects {
            self.execute_shell_chrome_effect(effect);
        }
    }

    fn execute_shell_chrome_effect(&mut self, effect: ShellChromeEffect) {
        match effect {
            ShellChromeEffect::RunStartupChecks => {
                let tx = self.tx.clone();
                let service = self.startup_service.clone();
                thread::spawn(move || {
                    let result = service.run_checks().map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::StartupLoaded(result));
                });
            }
            ShellChromeEffect::LoadRecentSessions { limit } => {
                let tx = self.tx.clone();
                let service = self.session_service.clone();
                thread::spawn(move || {
                    let result = service
                        .load_recent_sessions(limit)
                        .map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::SessionsLoaded(result));
                });
            }
        }
    }

    fn take_conversation_lifecycle_state(&mut self) -> ConversationLifecycleState {
        ConversationLifecycleState {
            conversation_state: std::mem::replace(
                &mut self.conversation_state,
                ConversationState::Loading,
            ),
            active_session: self.active_session.take(),
        }
    }

    fn apply_conversation_lifecycle_state(&mut self, state: ConversationLifecycleState) {
        self.conversation_state = state.conversation_state;
        self.active_session = state.active_session;
    }

    pub(super) fn dispatch_conversation_lifecycle(&mut self, event: ConversationLifecycleEvent) {
        let reduction =
            reduce_conversation_lifecycle(self.take_conversation_lifecycle_state(), event);
        self.apply_conversation_lifecycle_state(reduction.state);
        self.reset_transcript_viewport();
        for effect in reduction.effects {
            self.execute_conversation_lifecycle_effect(effect);
        }
    }

    fn execute_conversation_lifecycle_effect(&mut self, effect: ConversationLifecycleEffect) {
        match effect {
            ConversationLifecycleEffect::LoadConversation { thread_id } => {
                let tx = self.tx.clone();
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let result = service
                        .load_snapshot(&thread_id)
                        .map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::ConversationLoaded(result));
                });
            }
        }
    }

    pub(super) fn start_turn_submission(&mut self) {
        let inline_command = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                InlineShellCommand::parse(&conversation.input_buffer)
            }
            _ => None,
        };
        if let Some(command) = inline_command {
            self.execute_inline_shell_command(command);
            return;
        }

        let prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.can_submit_prompt() => {
                conversation.input_buffer.trim().to_string()
            }
            _ => return,
        };
        if prompt.is_empty() {
            return;
        }
        self.submit_prompt(prompt, PromptOrigin::Manual);
    }

    pub(super) fn take_ready_conversation_state(&mut self) -> Option<ConversationViewModel> {
        let state = std::mem::replace(&mut self.conversation_state, ConversationState::Loading);
        match state {
            ConversationState::Ready(conversation) => Some(conversation),
            other => {
                self.conversation_state = other;
                None
            }
        }
    }

    pub(super) fn dispatch_conversation_runtime(&mut self, event: ConversationRuntimeEvent) {
        let clear_turn_snapshot = matches!(
            &event,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::Failed { .. })
        );
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_conversation_runtime(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
        if clear_turn_snapshot {
            self.active_turn_planning_snapshot = None;
        }
        for effect in reduction.effects {
            self.execute_conversation_runtime_effect(effect);
        }
    }

    pub(super) fn dispatch_conversation_input(&mut self, event: ConversationInputEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_conversation_input(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
    }

    pub(super) fn clear_input_buffer(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::InputCleared);
    }

    fn conversation_intent_state(&self) -> ConversationIntentState {
        let mode = match &self.conversation_state {
            ConversationState::Loading => ConversationIntentMode::Loading,
            ConversationState::Failed(_) => ConversationIntentMode::Failed,
            ConversationState::Ready(conversation) if conversation.is_blank_draft() => {
                ConversationIntentMode::BlankDraft
            }
            ConversationState::Ready(_) => ConversationIntentMode::Ready,
        };

        ConversationIntentState {
            has_running_turn: self.conversation_has_running_turn(),
            mode,
        }
    }

    pub(super) fn dispatch_conversation_intent(&mut self, event: ConversationIntentEvent) {
        let reduction = reduce_conversation_intents(self.conversation_intent_state(), event);
        for effect in reduction.effects {
            self.execute_conversation_intent_effect(effect);
        }
    }

    fn execute_conversation_intent_effect(&mut self, effect: ConversationIntentEffect) {
        match effect {
            ConversationIntentEffect::ShowStatus { status_text } => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            ConversationIntentEffect::OpenNewDraft => {
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                let workspace_directory = self.current_workspace_directory();
                let template_load_result =
                    self.load_followup_template_catalog(&workspace_directory);
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
                    workspace_directory: workspace_directory.clone(),
                    template_load_result,
                });
                self.refresh_ready_conversation_planning_prompt_context();
                self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                    stop_keyword: self.current_stop_keyword_value(),
                    max_auto_turns: self.current_max_auto_turns_value().to_string(),
                });
            }
            ConversationIntentEffect::OpenSession { session } => {
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
                    session,
                });
            }
            ConversationIntentEffect::ShowExitConfirmation => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);
            }
        }
    }

    fn execute_conversation_runtime_effect(&mut self, effect: ConversationRuntimeEffect) {
        match effect {
            ConversationRuntimeEffect::StartStream {
                cwd,
                thread_id,
                prompt,
            } => {
                self.active_turn_planning_snapshot =
                    Some(self.load_planning_execution_snapshot(&cwd));
                let outer_tx = self.tx.clone();
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let (event_tx, event_rx) = mpsc::channel();

                    let service_thread = thread::spawn(move || {
                        let result = match thread_id {
                            Some(thread_id) => {
                                service.run_turn_stream(&thread_id, &prompt, event_tx)
                            }
                            None => service.run_new_thread_stream(&cwd, &prompt, event_tx),
                        };
                        let _ = result;
                    });

                    while let Ok(event) = event_rx.recv() {
                        let should_stop = matches!(
                            event,
                            ConversationStreamEvent::TurnCompleted { .. }
                                | ConversationStreamEvent::Failed { .. }
                        );
                        let _ = outer_tx.send(BackgroundMessage::ConversationStream(event));
                        if should_stop {
                            break;
                        }
                    }

                    let _ = service_thread.join();
                });
            }
            ConversationRuntimeEffect::EvaluateAutoFollowup {
                queued_from_turn_id,
                changed_planning_file_paths,
            } => self.evaluate_auto_followup_after_turn(
                queued_from_turn_id,
                changed_planning_file_paths,
            ),
            ConversationRuntimeEffect::QueueAutoPrompt {
                prompt,
                queued_from_turn_id,
                template_label,
            } => {
                self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(AutoFollowupSubmitContext {
                        queued_from_turn_id,
                        template_label,
                    }),
                );
            }
            ConversationRuntimeEffect::QueuePlanningRepairPrompt {
                prompt,
                queued_from_turn_id,
                attempt_number,
                max_attempts,
            } => {
                self.submit_prompt(
                    prompt,
                    PromptOrigin::PlanningRepair(PlanningRepairSubmitContext {
                        queued_from_turn_id,
                        attempt_number,
                        max_attempts,
                    }),
                );
            }
        }
    }

    fn evaluate_auto_followup_after_turn(
        &mut self,
        queued_from_turn_id: String,
        changed_planning_file_paths: Vec<String>,
    ) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reconciliation_result = self.reconcile_planning_after_turn(
            &conversation.cwd,
            &queued_from_turn_id,
            &changed_planning_file_paths,
        );
        let planning_repair_resolution = self.resolve_planning_repair_after_turn(
            &mut conversation,
            &queued_from_turn_id,
            &changed_planning_file_paths,
            &reconciliation_result,
        );
        let planning_prompt_context = if let Some(block_reason) = planning_repair_resolution
            .block_reason
            .clone()
            .or_else(|| reconciliation_result.auto_followup_block_reason.clone())
        {
            crate::application::service::planning_prompt_service::PlanningPromptContextLoadResult::blocked(
                block_reason,
            )
        } else if changed_planning_file_paths.is_empty() {
            conversation.planning_prompt_context.clone()
        } else {
            self.load_planning_prompt_context(&conversation.cwd)
        };
        for notice in reconciliation_result.notices {
            if !conversation.runtime_notices.contains(&notice) {
                conversation.runtime_notices.push(notice);
            }
        }
        for notice in planning_repair_resolution.notices {
            if !conversation.runtime_notices.contains(&notice) {
                conversation.runtime_notices.push(notice);
            }
        }
        conversation.replace_planning_prompt_context(planning_prompt_context);

        if let Some(queued_prompt) = planning_repair_resolution.queued_prompt {
            if !conversation.input_buffer.trim().is_empty() {
                let pause_notice = format!(
                    "planning repair retry {}/{} is waiting because manual input is buffered",
                    queued_prompt.attempt_number, queued_prompt.max_attempts
                );
                if !conversation.runtime_notices.contains(&pause_notice) {
                    conversation.runtime_notices.push(pause_notice);
                }
                conversation.status_text =
                    "turn completed / planning repair paused: manual input buffered".to_string();
                let should_refresh_lines =
                    conversation.append_status_message(conversation.status_text.clone());
                if should_refresh_lines {
                    conversation.refresh_conversation_lines();
                }
                self.conversation_state = ConversationState::Ready(conversation);
                return;
            }
            conversation.record_planning_repair_queue(
                queued_prompt.attempt_number,
                queued_prompt.max_attempts,
            );
            conversation.status_text = format!(
                "turn completed / queued planning repair {}/{}",
                queued_prompt.attempt_number, queued_prompt.max_attempts
            );
            let should_refresh_lines =
                conversation.append_status_message(conversation.status_text.clone());
            if should_refresh_lines {
                conversation.refresh_conversation_lines();
            }
            self.conversation_state = ConversationState::Ready(conversation);
            self.execute_conversation_runtime_effect(
                ConversationRuntimeEffect::QueuePlanningRepairPrompt {
                    prompt: queued_prompt.prompt,
                    queued_from_turn_id: queued_prompt.queued_from_turn_id,
                    attempt_number: queued_prompt.attempt_number,
                    max_attempts: queued_prompt.max_attempts,
                },
            );
            return;
        }

        match conversation.decide_auto_followup() {
            super::AutoFollowupDecision::QueuePrompt(prompt) => {
                conversation.clear_auto_followup_skip();
                let template_label = conversation.auto_follow_state.template_label().to_string();
                conversation.record_auto_followup_queue(&queued_from_turn_id, &template_label);
                conversation.status_text = format!(
                    "turn completed / queued auto follow-up with template {template_label}"
                );
                let should_refresh_lines =
                    conversation.append_status_message(conversation.status_text.clone());
                if should_refresh_lines {
                    conversation.refresh_conversation_lines();
                }
                self.conversation_state = ConversationState::Ready(conversation);
                self.execute_conversation_runtime_effect(
                    ConversationRuntimeEffect::QueueAutoPrompt {
                        prompt,
                        queued_from_turn_id,
                        template_label,
                    },
                );
            }
            super::AutoFollowupDecision::Skip(skip_reason) => {
                conversation.record_auto_followup_skip(skip_reason);
                conversation.status_text =
                    skip_reason.runtime_status(&conversation.auto_follow_state);
                let should_refresh_lines =
                    conversation.append_status_message(conversation.status_text.clone());
                if should_refresh_lines {
                    conversation.refresh_conversation_lines();
                }
                self.conversation_state = ConversationState::Ready(conversation);
            }
        }
    }

    fn load_planning_execution_snapshot(
        &self,
        workspace_directory: &str,
    ) -> ActiveTurnPlanningSnapshot {
        match self
            .planning_reconciliation_service
            .load_execution_snapshot(workspace_directory)
        {
            Ok(snapshot) => ActiveTurnPlanningSnapshot::Ready(snapshot),
            Err(error) => ActiveTurnPlanningSnapshot::CaptureFailed(format!(
                "planning reconciliation could not capture the accepted planning snapshot before the turn started: {error}"
            )),
        }
    }

    fn reconcile_planning_after_turn(
        &mut self,
        workspace_directory: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
    ) -> PlanningReconciliationResult {
        let requires_execution_snapshot = changed_planning_file_paths
            .iter()
            .any(|path| path == DIRECTIONS_FILE_PATH || path == TASK_LEDGER_FILE_PATH);

        if !requires_execution_snapshot {
            self.active_turn_planning_snapshot = None;
            return PlanningReconciliationResult::default();
        }

        let Some(snapshot_state) = self.active_turn_planning_snapshot.take() else {
            return PlanningReconciliationResult {
                notices: vec![
                    "planning reconciliation could not restore protected planning files because the turn snapshot was unavailable"
                        .to_string(),
                ],
                auto_followup_block_reason: Some(
                    "planning reconciliation could not restore protected planning files because the turn snapshot was unavailable"
                        .to_string(),
                ),
                ..PlanningReconciliationResult::default()
            };
        };

        let execution_snapshot = match snapshot_state {
            ActiveTurnPlanningSnapshot::Ready(snapshot) => snapshot,
            ActiveTurnPlanningSnapshot::CaptureFailed(error_message) => {
                return PlanningReconciliationResult {
                    notices: vec![error_message.clone()],
                    auto_followup_block_reason: Some(error_message),
                    ..PlanningReconciliationResult::default()
                };
            }
        };

        match self.planning_reconciliation_service.reconcile_after_turn(
            workspace_directory,
            turn_id,
            changed_planning_file_paths,
            &execution_snapshot,
        ) {
            Ok(result) => result,
            Err(error) => PlanningReconciliationResult {
                notices: vec![format!("planning reconciliation failed: {error}")],
                auto_followup_block_reason: Some(
                    "planning reconciliation failed; auto follow-up stays paused until the planning workspace is repaired"
                        .to_string(),
                ),
                ..PlanningReconciliationResult::default()
            },
        }
    }

    fn resolve_planning_repair_after_turn(
        &self,
        conversation: &mut ConversationViewModel,
        queued_from_turn_id: &str,
        changed_planning_file_paths: &[String],
        reconciliation_result: &PlanningReconciliationResult,
    ) -> PlanningRepairResolution {
        if let Some(repair_request) = reconciliation_result.repair_request.as_ref() {
            let retry_reason = conversation
                .planning_repair_state
                .as_ref()
                .map(|_| PlanningRepairRetryReason::TaskLedgerStillInvalid);
            return self.queue_planning_repair_attempt(
                conversation,
                queued_from_turn_id,
                repair_request,
                retry_reason,
            );
        }

        let Some(active_repair_state) = conversation.planning_repair_state.clone() else {
            return PlanningRepairResolution::default();
        };

        if reconciliation_result.auto_followup_block_reason.is_some() {
            conversation.planning_repair_state = None;
            return PlanningRepairResolution::default();
        }

        let task_ledger_changed = changed_planning_file_paths
            .iter()
            .any(|path| path == TASK_LEDGER_FILE_PATH);
        if task_ledger_changed && !reconciliation_result.rejected_task_ledger {
            conversation.planning_repair_state = None;
            return PlanningRepairResolution {
                notices: vec![format!(
                    "planning repair accepted task-ledger.json on retry {}/{}",
                    active_repair_state.attempts_used, active_repair_state.max_attempts
                )],
                ..PlanningRepairResolution::default()
            };
        }

        self.queue_planning_repair_attempt(
            conversation,
            active_repair_state.root_turn_id.as_str(),
            &active_repair_state.latest_request,
            Some(if task_ledger_changed {
                PlanningRepairRetryReason::TaskLedgerStillInvalid
            } else {
                PlanningRepairRetryReason::TaskLedgerUnchanged
            }),
        )
    }

    fn queue_planning_repair_attempt(
        &self,
        conversation: &mut ConversationViewModel,
        root_turn_id: &str,
        repair_request: &crate::application::service::planning_reconciliation_service::PlanningRepairRequest,
        retry_reason: Option<PlanningRepairRetryReason>,
    ) -> PlanningRepairResolution {
        let (next_attempt, max_attempts) =
            if let Some(state) = conversation.planning_repair_state.as_ref() {
                (state.attempts_used + 1, state.max_attempts)
            } else {
                (1, MAX_PLANNING_REPAIR_ATTEMPTS)
            };

        if next_attempt > max_attempts {
            conversation.planning_repair_state = None;
            return PlanningRepairResolution {
                notices: vec![format!(
                    "planning repair exhausted after {max_attempts} attempts; operator intervention is required"
                )],
                block_reason: Some(format!(
                    "planning repair exhausted after {max_attempts} attempts; auto follow-up stays paused until the operator repairs task-ledger.json"
                )),
                queued_prompt: None,
            };
        }

        let prompt =
            build_planning_repair_prompt(repair_request, next_attempt, max_attempts, retry_reason);
        conversation.planning_repair_state = Some(PlanningRepairState {
            root_turn_id: root_turn_id.to_string(),
            attempts_used: next_attempt,
            max_attempts,
            latest_request: repair_request.clone(),
        });
        PlanningRepairResolution {
            notices: vec![format!(
                "planning repair queued retry {next_attempt}/{max_attempts} for task-ledger.json"
            )],
            queued_prompt: Some(QueuedPlanningRepairPrompt {
                prompt,
                queued_from_turn_id: root_turn_id.to_string(),
                attempt_number: next_attempt,
                max_attempts,
            }),
            block_reason: None,
        }
    }

    pub(super) fn dispatch_followup_controls(&mut self, event: FollowupControlEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_followup_controls(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
        if !self.is_max_auto_turns_editing() {
            self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsValueSynced {
                value: self.current_max_auto_turns_value().to_string(),
            });
        }
        if !self.is_stop_keyword_editing() {
            self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordValueSynced {
                value: self.current_stop_keyword_value(),
            });
        }
        for effect in reduction.effects {
            self.execute_followup_control_effect(effect);
        }
    }

    fn execute_followup_control_effect(&mut self, effect: FollowupControlEffect) {
        match effect {
            FollowupControlEffect::SyncTemplateOverlayUi => {
                self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::TemplateChanged);
            }
            FollowupControlEffect::SyncMaxAutoTurnsEditor { value } => {
                self.dispatch_followup_overlay_ui(
                    FollowupOverlayUiEvent::MaxAutoTurnsEditCommitted {
                        current_value: value,
                    },
                );
            }
            FollowupControlEffect::SyncStopKeywordEditor { value } => {
                self.dispatch_followup_overlay_ui(
                    FollowupOverlayUiEvent::StopKeywordEditCommitted {
                        current_value: value,
                    },
                );
            }
        }
    }

    pub(super) fn dispatch_followup_overlay_ui(&mut self, event: FollowupOverlayUiEvent) {
        let state = std::mem::take(&mut self.followup_overlay_ui_state);
        self.followup_overlay_ui_state = reduce_followup_overlay_ui(state, event);
    }

    pub(super) fn resolve_startup_submit_queue(&mut self) {
        let (startup_submit_armed, prompt) = match &self.conversation_state {
            ConversationState::Ready(conversation) => (
                conversation.startup_submit_armed,
                conversation.input_buffer.trim().to_string(),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };
        if !startup_submit_armed {
            return;
        }

        match self.shell_action_availability() {
            super::ShellActionAvailability::Ready if prompt.is_empty() => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: None,
                });
            }
            super::ShellActionAvailability::Ready => {
                self.submit_prompt(prompt, PromptOrigin::Manual);
            }
            super::ShellActionAvailability::Pending => {}
            super::ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: Some(format!(
                        "{}; queued prompt kept in buffer",
                        self.submission_blocked_status(PromptOrigin::Manual)
                    )),
                });
            }
        }
    }

    pub(super) fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) {
        if matches!(prompt_origin, PromptOrigin::Manual)
            && matches!(
                self.shell_action_availability(),
                super::ShellActionAvailability::Pending
            )
        {
            self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            });
            return;
        }

        if !self.shell_action_availability().allows_actions() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.submission_blocked_status(prompt_origin),
            });
            return;
        }

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            origin: prompt_origin,
        });
    }
}
