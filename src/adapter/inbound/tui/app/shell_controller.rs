use super::*;
use crate::core::app::{
    AppCommand, AppEvent, ApprovalDecisionAdmission, StopRequestAdmission, StopRequestAttempt,
    StopRequestCorrelation, TurnSteerAdmission, TurnSteerCorrelation,
};
// Startup diagnostics gate user actions differently from rendering. The
// controller keeps the three user-facing states here so prompt submission,
// auto-follow, and overlays report the same readiness reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellActionAvailability {
    Ready,
    Pending,
    Blocked,
}
impl ShellActionAvailability {
    pub(super) fn allows_actions(self) -> bool {
        self == Self::Ready
    }
    pub(super) fn status_text(self) -> &'static str {
        match self {
            Self::Ready => "startup ready",
            Self::Pending => "startup checks still running",
            Self::Blocked => "startup diagnostics need attention",
        }
    }
}

// Shell controller methods translate keystrokes and inline commands into the
// smaller reducer events owned by conversation, chrome, planning, and follow-up
// modules. The controller should stay thin: route intent, update transient
// overlay UI state, and leave domain work to services.
impl NativeTuiApp {
    pub(super) fn can_open_session_list(&self) -> bool {
        matches!(
            &self.startup_state,
            StartupState::Ready(ready) if ready.can_continue
        )
    }
    pub(super) fn shell_action_availability(&self) -> ShellActionAvailability {
        match &self.startup_state {
            StartupState::Ready(ready) if ready.can_continue => ShellActionAvailability::Ready,
            StartupState::Idle | StartupState::Loading => ShellActionAvailability::Pending,
            StartupState::Ready(_) | StartupState::Failed(_) => ShellActionAvailability::Blocked,
        }
    }
    pub(super) fn submission_blocked_status(&self, prompt_origin: PromptOrigin) -> String {
        // Manual prompts can point the operator to diagnostics; auto-follow
        // needs a non-interactive pause reason that can be surfaced in status.
        match (prompt_origin, self.shell_action_availability()) {
            (_, ShellActionAvailability::Ready) => "ready".to_string(),
            (PromptOrigin::Manual | PromptOrigin::ManualIntake(_), state) => {
                format!("{}; open diagnostics with Ctrl+d", state.status_text())
            }
            (PromptOrigin::AutoFollow(_), ShellActionAvailability::Pending) => {
                "auto-follow paused while startup checks are still running".to_string()
            }
            (PromptOrigin::AutoFollow(_), ShellActionAvailability::Blocked) => {
                "auto-follow paused because startup diagnostics need attention".to_string()
            }
        }
    }
    pub(super) fn conversation_has_running_turn(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation) if conversation.has_running_turn()
        )
    }
    pub(super) fn show_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayShown);
    }
    pub(super) fn show_session_overlay(&mut self) {
        if self.parallel_mode_enabled() {
            // In parallel mode the session shortcut is repurposed to the
            // supersession control surface because session selection would fight
            // the slot orchestration view.
            self.inspect_parallel_mode_shell();
            return;
        }

        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayShown {
            limit: SESSION_PAGE_SIZE,
        });
    }
    pub(super) fn show_reviews_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::ReviewsOverlayShown);
        self.start_reviews_overlay_authority_load();
    }

    pub(super) fn start_reviews_overlay_authority_load(&mut self) {
        if self.shell_overlay != ShellOverlay::Reviews {
            return;
        }
        let context = self.current_reviews_overlay_context();
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::LoadReviewCenter {
                workspace_directory: context.workspace_directory,
                active_thread_id: context.active_thread.map(|thread| thread.thread_id),
            });
        let correlation = outcome.events.iter().find_map(|event| match event {
            AppEvent::ReviewCenterLoadStarted { correlation } => Some(correlation.clone()),
            _ => None,
        });
        if let Some(correlation) = correlation {
            self.begin_reviews_overlay_load(correlation);
        }
        // Bind the adapter-local display context before an immediate completion
        // is projected back through the core event stream.
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn reconcile_reviews_overlay_authority_context(&mut self) -> bool {
        if !self.reviews_overlay_authority_load_required() {
            return false;
        }
        self.start_reviews_overlay_authority_load();
        true
    }

    pub(super) fn toggle_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayToggled);
    }
    pub(super) fn toggle_session_overlay(&mut self) {
        if self.parallel_mode_enabled() {
            self.toggle_supersession_overlay();
            return;
        }

        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayToggled {
            limit: SESSION_PAGE_SIZE,
        });
    }
    pub(super) fn close_shell_overlay(&mut self) {
        // Closing shell chrome also drops editor-local draft buffers for
        // overlays that stage multi-step planning changes. Plain list/detail
        // overlays do not own such scratch state.
        match self.shell_overlay {
            ShellOverlay::DirectionsMaintenance => {
                self.directions_maintenance_overlay_ui_state.reset();
                self.planning_draft_editor_ui_state.reset();
            }
            ShellOverlay::PlanningInit => {
                self.planning_runtime_refresh_ui_state.clear_loading();
                self.planning_init_overlay_ui_state.reset();
                self.planning_draft_editor_ui_state.reset();
            }
            ShellOverlay::ModelSelection => {
                self.model_selection_overlay_ui_state = ModelSelectionOverlayUiState::default();
            }
            ShellOverlay::ViewSelection => {
                self.view_selection_overlay_ui_state = ViewSelectionOverlayUiState::default();
            }
            ShellOverlay::LanguageSelection => {
                self.language_selection_overlay_ui_state =
                    LanguageSelectionOverlayUiState::default();
            }
            ShellOverlay::ParallelPeek => {
                self.parallel_peek_overlay_ui_state.reset();
            }
            ShellOverlay::Activity => {
                self.progressive_activity_overlay_ui_state.reset();
            }
            _ => {}
        }
        self.dispatch_shell_chrome(ShellChromeEvent::OverlayClosed);
    }
    pub(super) fn open_new_conversation_shell(&mut self) {
        self.dispatch_conversation_intent(ConversationIntentEvent::NewDraftRequested);
    }
    pub(super) fn execute_inline_shell_command_input(
        &mut self,
        command_input: InlineShellCommandInput,
    ) {
        // Inline commands are executed by semantic command, not raw text, so the
        // same path is used for palette acceptance and typed slash commands.
        match command_input.command() {
            InlineShellCommand::Diagnostics => self.show_startup_overlay(),
            InlineShellCommand::Parallel => {
                self.handle_parallel_shell_command(command_input.argument())
            }
            InlineShellCommand::Peek => self.open_parallel_peek_overlay(command_input.argument()),
            InlineShellCommand::Activity => {
                self.handle_activity_shell_command(command_input.argument())
            }
            InlineShellCommand::Sessions => self.show_session_overlay(),
            InlineShellCommand::Reviews => self.show_reviews_overlay(),
            InlineShellCommand::Queue => self.handle_queue_shell_command(command_input.argument()),
            InlineShellCommand::Directions => {
                self.handle_directions_shell_command(command_input.argument())
            }
            InlineShellCommand::Turns => self.handle_turns_shell_command(command_input.argument()),
            InlineShellCommand::Stop => self.handle_stop_shell_command(),
            InlineShellCommand::Model => self.handle_model_shell_command(command_input.argument()),
            InlineShellCommand::View => self.handle_view_shell_command(command_input.argument()),
            InlineShellCommand::Language => {
                self.handle_language_shell_command(command_input.argument())
            }
            InlineShellCommand::Think => self.handle_think_shell_command(command_input.argument()),
            InlineShellCommand::Doctor => self.run_planning_doctor(),
            InlineShellCommand::PlanningInit => {
                self.handle_planning_shell_command(command_input.argument())
            }
            InlineShellCommand::Reset => self.handle_reset_shell_command(command_input.argument()),
            InlineShellCommand::NewDraft => self.open_new_conversation_shell(),
            InlineShellCommand::Help => self.show_help_overlay(),
        }
        let status_text = match command_input.command() {
            InlineShellCommand::Sessions if self.parallel_mode_enabled() => Some(
                self.tui_language
                    .parallel_control_tower_opened_status()
                    .to_string(),
            ),
            _ => command_input.localized_execution_status(self.tui_language),
        };
        // Command execution consumes the prompt buffer after any command-specific
        // status is emitted; commands that need arguments insert text before
        // reaching this path.
        if let Some(status_text) = status_text {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text,
            });
        }
        self.clear_input_buffer();
    }
    fn show_help_overlay(&mut self) {
        self.help_scroll_offset = 0;
        self.dispatch_shell_chrome(ShellChromeEvent::HelpOverlayShown);
    }
    fn handle_activity_shell_command(&mut self, argument: Option<&str>) {
        match argument {
            None => {
                self.show_progressive_activity_overlay_all();
            }
            Some(argument) => {
                if let Some(selected_kind) = parse_progressive_activity_detail_kind(argument) {
                    self.show_progressive_activity_overlay(selected_kind);
                    return;
                }
                if let Some(card_filter) = parse_progressive_activity_card_filter(argument) {
                    self.show_progressive_activity_overlay_filter(card_filter);
                    return;
                }
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "activity unchanged; supported values: all, diff, output, command, patch, mcp, plan, reason, agent, terminal, token, guardian, moderation, unknown"
                        .to_string(),
                });
            }
        }
    }
    pub(super) fn show_progressive_activity_overlay(
        &mut self,
        selected_kind: ProgressiveActivityDetailKind,
    ) -> bool {
        if self.approval_overlay_active() {
            return false;
        }
        self.progressive_activity_overlay_ui_state
            .reset_for_kind(selected_kind);
        self.dispatch_shell_chrome(ShellChromeEvent::ActivityOverlayShown);
        true
    }
    pub(super) fn show_progressive_activity_overlay_all(&mut self) -> bool {
        self.show_progressive_activity_overlay_filter(None)
    }
    pub(super) fn show_progressive_activity_overlay_filter(
        &mut self,
        card_filter: Option<super::ProgressiveActivityCardKind>,
    ) -> bool {
        if self.approval_overlay_active() {
            return false;
        }
        self.progressive_activity_overlay_ui_state
            .reset_for_card_filter(card_filter);
        self.dispatch_shell_chrome(ShellChromeEvent::ActivityOverlayShown);
        true
    }
    pub(super) fn show_model_selection_overlay(&mut self) {
        self.model_selection_overlay_ui_state
            .reset_from_turn_options(&self.turn_options);
        self.dispatch_shell_chrome(ShellChromeEvent::ModelSelectionOverlayShown);
    }
    pub(super) fn show_view_selection_overlay(&mut self) {
        self.view_selection_overlay_ui_state
            .reset_from_mode(self.conversation_view_mode);
        self.dispatch_shell_chrome(ShellChromeEvent::ViewSelectionOverlayShown);
    }
    pub(super) fn show_language_selection_overlay(&mut self) {
        self.language_selection_overlay_ui_state
            .reset_from_language(self.tui_language);
        self.dispatch_shell_chrome(ShellChromeEvent::LanguageSelectionOverlayShown);
    }
    fn handle_turns_shell_command(&mut self, argument: Option<&str>) {
        self.dispatch_auto_follow_controls(AutoFollowControlEvent::MaxAutoTurnsUpdated {
            value: argument.unwrap_or_default().to_string(),
        });
    }
    fn handle_model_shell_command(&mut self, argument: Option<&str>) {
        if argument.is_some_and(is_turn_option_clear_argument) {
            self.turn_options.model = None;
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "model reset to app-server default".to_string(),
            });
            return;
        }
        self.show_model_selection_overlay();
        if argument.is_some() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "`:model` ignored the typed argument; choose from the picker instead"
                    .to_string(),
            });
        }
    }
    fn handle_view_shell_command(&mut self, argument: Option<&str>) {
        let Some(argument) = argument else {
            self.show_view_selection_overlay();
            return;
        };

        match ConversationViewMode::parse(argument) {
            Some(mode) => self.apply_conversation_view_mode(mode),
            None => {
                self.show_view_selection_overlay();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "view unchanged; supported values: {}",
                        ConversationViewMode::SUPPORTED_LABELS
                    ),
                });
            }
        }
    }
    fn handle_language_shell_command(&mut self, argument: Option<&str>) {
        let Some(argument) = argument else {
            self.show_language_selection_overlay();
            return;
        };

        match TuiLanguage::parse(argument) {
            Some(language) => self.apply_tui_language(language),
            None => {
                self.show_language_selection_overlay();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "language unchanged; supported values: {}",
                        TuiLanguage::SUPPORTED_LABELS
                    ),
                });
            }
        }
    }
    fn handle_think_shell_command(&mut self, argument: Option<&str>) {
        let status_text = match argument {
            None => format!(
                "think override unchanged / current: {} / use :think <{}>",
                self.turn_options
                    .reasoning_effort
                    .map(ConversationReasoningEffort::label)
                    .unwrap_or("default"),
                ConversationReasoningEffort::SUPPORTED_LABELS
            ),
            Some(value) if is_turn_option_clear_argument(value) => {
                self.turn_options.reasoning_effort = None;
                "think reset to app-server default".to_string()
            }
            Some(value) => match ConversationReasoningEffort::parse(value) {
                Some(effort) => {
                    self.turn_options.reasoning_effort = Some(effort);
                    format!("think override set to {}", effort.label())
                }
                None => format!(
                    "think override unchanged; supported values: {}",
                    ConversationReasoningEffort::SUPPORTED_LABELS
                ),
            },
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }
    fn handle_stop_shell_command(&mut self) {
        self.dispatch_auto_follow_controls(AutoFollowControlEvent::AutoFollowPaused);
        self.close_parallel_mode_automation_epoch();
        self.invalidate_parallel_mode_supervisor_snapshot();
        // Stop is both a local mode transition and an app-server control request:
        // disable future automation immediately, then let Core correlate the
        // global runtime signal with the active turn generation.
        self.dispatch_core_command(AppCommand::RequestStopAllSessions);
    }
    pub(super) fn apply_stop_request_admission(&mut self, admission: StopRequestAdmission) {
        let status_text = match admission {
            StopRequestAdmission::Accepted { correlation }
                if correlation.turn_submission.is_some() =>
            {
                "stop requested / active app-server sessions will be interrupted / auto-follow disarmed until :turns re-enables it".to_string()
            }
            StopRequestAdmission::Accepted { .. } => {
                "stop requested / no active turn is running / auto-follow disarmed until :turns re-enables it".to_string()
            }
            StopRequestAdmission::RejectedActive { .. } => {
                "stop already requested / waiting for the active app-server turn / auto-follow remains disarmed until :turns re-enables it".to_string()
            }
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }
    pub(super) fn apply_stop_request_attempt_completion(
        &mut self,
        _correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
        result: Result<(), String>,
    ) {
        let status_text = match (attempt, result) {
            (StopRequestAttempt::Initial, Ok(())) => return,
            (StopRequestAttempt::AfterTurnStarted, Ok(())) => {
                "stop synchronized / active app-server turn will be interrupted".to_string()
            }
            (StopRequestAttempt::Initial, Err(error)) => format!(
                "stop request failed: {error} / auto-follow remains disarmed until :turns re-enables it"
            ),
            (StopRequestAttempt::AfterTurnStarted, Err(error)) => {
                format!("stop request failed after turn start: {error}")
            }
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }
    pub(super) fn push_input_character(&mut self, character: char) {
        self.dispatch_conversation_input(ConversationInputEvent::CharacterTyped { character });
    }
    pub(super) fn insert_input_text(&mut self, text: String) -> bool {
        if text.is_empty() || !self.can_edit_prompt_input() {
            return false;
        }

        self.dispatch_conversation_input(ConversationInputEvent::TextInserted { text });
        true
    }
    pub(super) fn can_edit_prompt_input(&self) -> bool {
        self.prompt_input_has_focus()
    }
    pub(super) fn prompt_input_has_focus(&self) -> bool {
        self.shell_overlay.prompt_input_has_focus(
            self.is_exit_confirmation_visible() || self.is_turn_steer_confirmation_visible(),
            self.parallel_mode_prompt_input_locked(),
        )
    }
    pub(super) fn is_inline_command_palette_active(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if conversation.inline_shell_command_palette_state.is_active()
        )
    }
    pub(super) fn move_inline_command_palette_selection(&mut self, delta: isize) -> bool {
        if !self.is_inline_command_palette_active() {
            return false;
        }

        self.dispatch_conversation_input(
            ConversationInputEvent::InlineCommandPaletteSelectionMoved { delta },
        );
        true
    }
    pub(super) fn dismiss_inline_command_palette(&mut self) -> bool {
        if !self.is_inline_command_palette_active() {
            return false;
        }

        self.dispatch_conversation_input(ConversationInputEvent::InlineCommandPaletteDismissed);
        true
    }
    pub(super) fn accept_inline_command_palette_selection(&mut self) -> bool {
        let selected_command = match &self.conversation_state {
            ConversationState::Ready(conversation)
                if conversation.inline_shell_command_palette_state.is_active() =>
            {
                conversation
                    .inline_shell_command_palette_state
                    .selected_command()
            }
            _ => None,
        };
        let Some(command) = selected_command else {
            return false;
        };
        // Commands with arguments stay in the prompt for editing; argument-free
        // commands execute immediately through the same inline command handler.
        if command.requires_argument() {
            self.dispatch_conversation_input(
                ConversationInputEvent::InlineCommandPaletteCommandInserted { command },
            );
            return true;
        }

        self.execute_inline_shell_command_input(InlineShellCommandInput::from_command(command));
        true
    }
    pub(super) fn insert_input_newline(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::NewlineInserted);
    }
    pub(super) fn pop_input_character(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::BackspacePressed);
    }
    pub(super) fn delete_next_input_character(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::DeletePressed);
    }
    pub(super) fn delete_previous_input_word(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::PreviousWordDeleted);
    }
    pub(super) fn move_input_cursor(&mut self, movement: InputCursorMovement) {
        self.dispatch_conversation_input(ConversationInputEvent::CursorMoved { movement });
    }
    pub(super) fn clear_prompt_input(&mut self) {
        self.clear_input_buffer();
    }
    pub(super) fn is_shell_overlay_visible(&self) -> bool {
        self.shell_overlay != ShellOverlay::Hidden
    }
    pub(super) fn approval_overlay_active(&self) -> bool {
        self.shell_overlay == ShellOverlay::Approval
    }
    pub(super) fn is_exit_confirmation_visible(&self) -> bool {
        self.exit_confirmation_state == ExitConfirmationState::Visible
    }

    pub(super) fn is_turn_steer_confirmation_visible(&self) -> bool {
        self.turn_steer_confirmation.is_some()
            && self.shell_overlay == ShellOverlay::Hidden
            && !self.is_exit_confirmation_visible()
    }

    pub(super) fn show_turn_steer_confirmation(&mut self) -> bool {
        if self.pending_manual_prompt_preparation.is_some() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self
                    .tui_language
                    .manual_prompt_queue_pending_status()
                    .to_string(),
            });
            return true;
        }
        if self.pending_turn_steer.is_some() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.tui_language.turn_steer_pending_status().to_string(),
            });
            return true;
        }
        let (request, source_input_buffer) = match &self.conversation_state {
            ConversationState::Ready(conversation)
                if conversation.has_running_turn()
                    && !conversation.input_buffer.trim().is_empty() =>
            {
                let Some(expected_turn_id) = conversation.active_turn_id.clone() else {
                    self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: self
                            .tui_language
                            .turn_steer_unavailable_status()
                            .to_string(),
                    });
                    return true;
                };
                if conversation.thread_id.trim().is_empty() {
                    self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: self
                            .tui_language
                            .turn_steer_unavailable_status()
                            .to_string(),
                    });
                    return true;
                }
                (
                    ConversationTurnSteerRequest {
                        thread_id: conversation.thread_id.clone(),
                        expected_turn_id,
                        prompt: conversation.input_buffer.trim().to_string(),
                    },
                    conversation.input_buffer.clone(),
                )
            }
            ConversationState::Ready(conversation)
                if conversation.has_running_turn()
                    && conversation.input_buffer.trim().is_empty() =>
            {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self
                        .tui_language
                        .turn_steer_needs_prompt_status()
                        .to_string(),
                });
                return true;
            }
            _ => return false,
        };
        self.turn_steer_confirmation = Some(TurnSteerUiIntent {
            input_revision: self.prompt_input_revision,
            source_input_buffer,
            request,
        });
        true
    }

    pub(super) fn handle_turn_steer_confirmation_key(&mut self, key: event::KeyEvent) -> bool {
        if !self.is_turn_steer_confirmation_visible() {
            return false;
        }
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            self.turn_steer_confirmation = None;
            return false;
        }
        if !key.modifiers.is_empty() {
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                self.turn_steer_confirmation = None;
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self.tui_language.turn_steer_cancelled_status().to_string(),
                });
            }
            KeyCode::Enter | KeyCode::Tab => self.confirm_turn_steer(),
            _ => {}
        }
        true
    }

    fn confirm_turn_steer(&mut self) {
        let Some(intent) = self.turn_steer_confirmation.take() else {
            return;
        };
        let still_exact = matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if conversation.thread_id == intent.request.thread_id
                    && conversation.active_turn_id.as_deref()
                        == Some(intent.request.expected_turn_id.as_str())
                    && conversation.input_buffer == intent.source_input_buffer
        ) && self.prompt_input_revision == intent.input_revision;
        if !still_exact {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self
                    .tui_language
                    .turn_steer_unavailable_status()
                    .to_string(),
            });
            return;
        }

        let request = intent.request.clone();
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::SteerTurn(request));
        let admission = outcome.events.iter().find_map(|event| match event {
            AppEvent::TurnSteerAdmissionResolved(admission) => Some(*admission),
            _ => None,
        });
        match admission {
            Some(TurnSteerAdmission::Accepted { correlation }) => {
                self.pending_turn_steer = Some(PendingTurnSteerUiIntent {
                    correlation,
                    intent,
                });
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self.tui_language.turn_steer_pending_status().to_string(),
                });
            }
            Some(TurnSteerAdmission::RejectedActive { .. }) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self.tui_language.turn_steer_pending_status().to_string(),
                });
            }
            Some(TurnSteerAdmission::RejectedUnavailable) | None => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self
                        .tui_language
                        .turn_steer_unavailable_status()
                        .to_string(),
                });
            }
        }
        // Install the adapter-local draft binding before applying a completion
        // from an immediate test executor.
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn apply_turn_steer_completion(
        &mut self,
        correlation: TurnSteerCorrelation,
        result: Result<crate::domain::conversation::ConversationTurnSteerReceipt, String>,
    ) {
        let Some(pending) = self
            .pending_turn_steer
            .as_ref()
            .filter(|pending| pending.correlation == correlation)
            .cloned()
        else {
            return;
        };
        self.pending_turn_steer = None;
        let intent = pending.intent;
        match result {
            Ok(receipt) => {
                let input_revision = self.prompt_input_revision;
                let draft_is_current = matches!(
                    &self.conversation_state,
                    ConversationState::Ready(conversation)
                        if conversation.thread_id == intent.request.thread_id
                            && conversation.input_buffer == intent.source_input_buffer
                ) && input_revision == intent.input_revision;
                if draft_is_current {
                    self.dispatch_conversation_input(ConversationInputEvent::InputCleared);
                }
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self
                        .tui_language
                        .turn_steer_succeeded_status(&receipt.turn_id),
                });
            }
            Err(reason) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self.tui_language.turn_steer_failed_status(&reason),
                });
            }
        }
    }

    pub(super) fn handle_exit_confirmation_key(&mut self, key: event::KeyEvent) -> Option<bool> {
        if !self.is_exit_confirmation_visible() {
            return None;
        }
        // Shift is allowed so uppercase Y/N works, but other modifiers should
        // fall through to the caller rather than accidentally confirming exit.
        if !key.modifiers.is_empty() && key.modifiers != KeyModifiers::SHIFT {
            return None;
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationHidden);
                Some(true)
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationHidden);
                Some(false)
            }
            _ => Some(false),
        }
    }
    pub(super) fn handle_shell_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay == ShellOverlay::Hidden {
            return false;
        }
        if self.shell_overlay == ShellOverlay::Approval {
            return self.handle_approval_overlay_key(key);
        }
        let is_startup_overlay = self.shell_overlay == ShellOverlay::Startup;
        // Text-field handlers get first refusal because their shortcuts must not
        // leak into overlay navigation while the cursor is inside an editor.
        if self.handle_max_auto_turns_editor_key(key) {
            return true;
        }
        if self.handle_session_rename_editor_key(key) {
            return true;
        }
        if self.handle_session_search_query_editor_key(key) {
            return true;
        }
        if self.handle_parallel_peek_overlay_key(key) {
            return true;
        }
        if key.code == KeyCode::Esc
            || (key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c'))
        {
            let closing_directions_manual_editor = self.shell_overlay
                == ShellOverlay::DirectionsMaintenance
                && self.directions_maintenance_overlay_ui_state.step()
                    == DirectionsMaintenanceOverlayStep::ManualEditor;
            let closing_planning_manual_editor = self.shell_overlay == ShellOverlay::PlanningInit
                && self.planning_init_overlay_ui_state.step()
                    == PlanningInitOverlayStep::ManualEditor;
            // Manual editors have their own close guards for unsaved staged
            // content; other overlays can close directly through shell chrome.
            if closing_directions_manual_editor {
                self.request_close_directions_manual_editor();
            } else if closing_planning_manual_editor {
                self.request_close_planning_manual_editor();
            } else {
                self.close_shell_overlay();
            }
            return true;
        }
        if is_startup_overlay {
            match key.code {
                KeyCode::Char('r') if key.modifiers.is_empty() => {
                    self.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested)
                }
                KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                    self.show_session_overlay()
                }
                _ => {}
            }
            return true;
        }
        if self.handle_supersession_overlay_key(key) {
            return true;
        }
        if self.shell_overlay == ShellOverlay::Supersession {
            // Supersession only owns ordinary prompt keys while its loading
            // pipeline is active. Once the board has a concrete snapshot, prompt
            // editing falls through so the operator can keep working while the
            // board remains visible.
            return self.parallel_mode_prompt_input_locked();
        }
        if self.shell_overlay == ShellOverlay::ModelSelection {
            return self.handle_model_selection_overlay_key(key);
        }
        if self.shell_overlay == ShellOverlay::ViewSelection {
            return self.handle_view_selection_overlay_key(key);
        }
        if self.shell_overlay == ShellOverlay::LanguageSelection {
            return self.handle_language_selection_overlay_key(key);
        }
        if self.shell_overlay == ShellOverlay::DirectionsMaintenance {
            return self.handle_directions_overlay_key(key);
        }
        if self.shell_overlay == ShellOverlay::PlanningInit {
            return self.handle_planning_init_overlay_key(key);
        }
        if self.shell_overlay == ShellOverlay::Activity {
            return self.handle_progressive_activity_overlay_key(key);
        }
        if self.shell_overlay == ShellOverlay::Help {
            return self.handle_help_overlay_key(key);
        }
        if self.shell_overlay == ShellOverlay::Queue {
            return self.handle_queue_overlay_key(key);
        }

        self.handle_session_overlay_key(key);
        true
    }

    fn handle_help_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match (key.code, key.modifiers) {
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.help_scroll_offset = self.help_scroll_offset.saturating_sub(1);
            }
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.help_scroll_offset = self.help_scroll_offset.saturating_add(1);
            }
            (KeyCode::PageUp, KeyModifiers::NONE) => {
                self.help_scroll_offset = self.help_scroll_offset.saturating_sub(5);
            }
            (KeyCode::PageDown, KeyModifiers::NONE) => {
                self.help_scroll_offset = self.help_scroll_offset.saturating_add(5);
            }
            (KeyCode::Home, KeyModifiers::NONE) => self.help_scroll_offset = 0,
            (KeyCode::End, KeyModifiers::NONE) => self.help_scroll_offset = usize::MAX,
            _ => {}
        }
        true
    }

    fn handle_progressive_activity_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        let filtered_len = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                let cards = conversation.progressive_activity_detail.cards();
                super::filter_cards_by_kind(
                    &cards,
                    self.progressive_activity_overlay_ui_state.card_filter(),
                )
                .len()
            }
            ConversationState::Loading | ConversationState::Failed(_) => 0,
        };
        match (key.code, key.modifiers) {
            (
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right,
                KeyModifiers::NONE | KeyModifiers::SHIFT,
            ) => self.progressive_activity_overlay_ui_state.cycle_kind(),
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.progressive_activity_overlay_ui_state
                    .move_card_selection(-1, filtered_len);
            }
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.progressive_activity_overlay_ui_state
                    .move_card_selection(1, filtered_len);
            }
            (KeyCode::PageUp, KeyModifiers::NONE) => {
                self.progressive_activity_overlay_ui_state
                    .move_to_previous_page();
            }
            (KeyCode::PageDown, KeyModifiers::NONE) => {
                self.progressive_activity_overlay_ui_state
                    .move_to_next_page();
            }
            (KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('l'), KeyModifiers::NONE) => {
                if let ConversationState::Ready(conversation) = &self.conversation_state {
                    let cards = conversation.progressive_activity_detail.cards();
                    let filtered = super::filter_cards_by_kind(
                        &cards,
                        self.progressive_activity_overlay_ui_state.card_filter(),
                    );
                    if let Some(card_index) = filtered.get(
                        self.progressive_activity_overlay_ui_state
                            .selected_card_index(),
                    ) && let Some(card) = cards.get(*card_index)
                    {
                        self.progressive_activity_overlay_ui_state
                            .expand_state_mut()
                            .expand_card(card.key);
                    }
                }
                self.progressive_activity_overlay_ui_state.focus_detail();
            }
            (KeyCode::Char('h'), KeyModifiers::NONE) => {
                self.progressive_activity_overlay_ui_state.focus_list();
            }
            (KeyCode::Home, KeyModifiers::NONE) => self
                .progressive_activity_overlay_ui_state
                .reset_navigation(),
            _ => {}
        }
        true
    }
    fn handle_approval_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match (key.code, key.modifiers) {
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.move_pending_approval_detail_scroll(-1);
                return true;
            }
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.move_pending_approval_detail_scroll(1);
                return true;
            }
            (KeyCode::PageUp, KeyModifiers::NONE) => {
                self.move_pending_approval_detail_scroll(-5);
                return true;
            }
            (KeyCode::PageDown, KeyModifiers::NONE) => {
                self.move_pending_approval_detail_scroll(5);
                return true;
            }
            _ => {}
        }
        if self.pending_approval_decision_submitted() {
            if matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('c'), KeyModifiers::CONTROL)
            ) {
                self.handle_stop_shell_command();
            }
            return true;
        }
        let decision = match (key.code, key.modifiers) {
            (KeyCode::Char('y' | 'Y'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Some(crate::domain::conversation::ConversationApprovalDecision::Accept)
            }
            (KeyCode::Esc, KeyModifiers::NONE)
            | (KeyCode::Char('n' | 'N'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Some(crate::domain::conversation::ConversationApprovalDecision::Decline)
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.submit_pending_approval_decision(
                    crate::domain::conversation::ConversationApprovalDecision::Decline,
                );
                self.handle_stop_shell_command();
                return true;
            }
            _ => None,
        };
        if let Some(decision) = decision {
            self.submit_pending_approval_decision(decision);
        }
        true
    }
    fn move_pending_approval_detail_scroll(&mut self, delta: isize) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.move_approval_detail_scroll(delta);
        }
    }
    fn pending_approval_decision_submitted(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if conversation.pending_approval_decision().is_some()
        )
    }
    fn submit_pending_approval_decision(
        &mut self,
        decision: crate::domain::conversation::ConversationApprovalDecision,
    ) {
        let approval_id = match &self.conversation_state {
            ConversationState::Ready(conversation)
                if conversation.pending_approval_decision().is_none() =>
            {
                conversation
                    .pending_approval_request
                    .as_ref()
                    .map(|request| request.approval_id.clone())
            }
            ConversationState::Loading | ConversationState::Failed(_) => None,
            ConversationState::Ready(_) => None,
        };
        if let Some(approval_id) = approval_id {
            let outcome = self
                .core_runtime
                .dispatch_command(AppCommand::SubmitApprovalDecision {
                    approval_id: approval_id.clone(),
                    decision,
                });
            let admitted = outcome.events.iter().any(|event| {
                matches!(
                    event,
                    AppEvent::ApprovalDecisionAdmissionResolved(
                        ApprovalDecisionAdmission::Accepted { .. }
                    )
                )
            });
            if admitted {
                self.dispatch_conversation_runtime(
                    ConversationRuntimeEvent::ApprovalDecisionSubmitted {
                        approval_id,
                        decision,
                    },
                );
            }
            // Commit the adapter-local pending projection before an immediate
            // completion can reopen it for retry.
            self.apply_core_dispatch_outcome(outcome);
        }
    }
    pub(super) fn handle_ctrl_c(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationHidden);
        if self.approval_overlay_active() {
            self.submit_pending_approval_decision(
                crate::domain::conversation::ConversationApprovalDecision::Decline,
            );
            self.handle_stop_shell_command();
            return;
        }
        if self.is_shell_overlay_visible() {
            self.close_shell_overlay();
            return;
        }

        if self.conversation_has_running_turn() {
            self.handle_stop_shell_command();
            return;
        }

        self.dispatch_conversation_intent(ConversationIntentEvent::CtrlCPressed);
    }

    pub(super) fn handle_model_selection_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::ModelSelection {
            return false;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.model_selection_overlay_ui_state.move_selection(-1);
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.model_selection_overlay_ui_state.move_selection(1);
            }
            KeyCode::Char(number)
                if key.modifiers.is_empty() && number.is_ascii_digit() && number != '0' =>
            {
                let index = number.to_digit(10).unwrap_or(0).saturating_sub(1) as usize;
                if self
                    .model_selection_overlay_ui_state
                    .select_active_index(index)
                {
                    self.confirm_model_selection_overlay_step();
                }
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                self.confirm_model_selection_overlay_step()
            }
            KeyCode::Left | KeyCode::Backspace
                if key.modifiers.is_empty()
                    && self.model_selection_overlay_ui_state.step()
                        == ModelSelectionStep::Effort =>
            {
                self.model_selection_overlay_ui_state
                    .return_to_model_selection();
            }
            _ => {}
        }
        true
    }

    pub(super) fn handle_view_selection_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::ViewSelection {
            return false;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.view_selection_overlay_ui_state.move_selection(-1);
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.view_selection_overlay_ui_state.move_selection(1);
            }
            KeyCode::Char(number)
                if key.modifiers.is_empty() && number.is_ascii_digit() && number != '0' =>
            {
                let index = number.to_digit(10).unwrap_or(0).saturating_sub(1) as usize;
                if self.view_selection_overlay_ui_state.select_index(index) {
                    self.apply_view_selection_overlay();
                }
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.apply_view_selection_overlay(),
            _ => {}
        }
        true
    }

    pub(super) fn handle_language_selection_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::LanguageSelection {
            return false;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.language_selection_overlay_ui_state.move_selection(-1);
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.language_selection_overlay_ui_state.move_selection(1);
            }
            KeyCode::Char(number)
                if key.modifiers.is_empty() && number.is_ascii_digit() && number != '0' =>
            {
                let index = number.to_digit(10).unwrap_or(0).saturating_sub(1) as usize;
                if self.language_selection_overlay_ui_state.select_index(index) {
                    self.apply_language_selection_overlay();
                }
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.apply_language_selection_overlay(),
            _ => {}
        }
        true
    }

    fn confirm_model_selection_overlay_step(&mut self) {
        match self.model_selection_overlay_ui_state.step() {
            ModelSelectionStep::Model => {
                self.model_selection_overlay_ui_state
                    .advance_from_model_selection();
            }
            ModelSelectionStep::Effort => self.apply_model_selection_overlay(),
        }
    }

    fn apply_model_selection_overlay(&mut self) {
        let model_option = self.model_selection_overlay_ui_state.staged_model();
        let effort_option = self.model_selection_overlay_ui_state.selected_effort();
        self.turn_options.model = model_option.model.map(str::to_string);
        self.turn_options.reasoning_effort = effort_option.effort;
        self.close_shell_overlay();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: format!(
                "model set to {}; think set to {}",
                model_option.label, effort_option.label
            ),
        });
    }

    fn apply_view_selection_overlay(&mut self) {
        let mode = self.view_selection_overlay_ui_state.selected_mode();
        self.apply_conversation_view_mode(mode);
    }

    fn apply_language_selection_overlay(&mut self) {
        let language = self.language_selection_overlay_ui_state.selected_language();
        self.apply_tui_language(language);
    }

    fn apply_conversation_view_mode(&mut self, mode: ConversationViewMode) {
        self.conversation_view_mode = mode;
        self.close_shell_overlay();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: format!("conversation view set to {}", mode.label()),
        });
    }

    fn apply_tui_language(&mut self, language: TuiLanguage) {
        self.tui_language = language;
        self.close_shell_overlay();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: language.language_set_status().to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread::ThreadId;
    use std::time::{Duration, Instant};

    use crate::adapter::inbound::tui::app::test_helpers::{
        sample_planning_runtime_projection, sample_queue_head, test_native_tui_app,
        test_native_tui_app_with_approval_resolution_error, test_native_tui_app_with_planning,
        test_native_tui_app_with_services, test_planning_services_with_task_repository,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningAuthoritySnapshotCommit,
        PlanningDirectionAuthorityCommit, PlanningDirectionAuthoritySnapshot,
        PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult,
        PlanningTaskAuthorityMutationAudit, PlanningTaskAuthorityMutationRecord,
        PlanningTaskAuthoritySnapshot, PlanningTaskRepositoryPort,
    };
    use crate::application::service::planning::{
        PlanningBootstrapMode, PlanningInitStageResult, PlanningRuntimeProjection,
        PlanningTaskToolRequest,
    };
    use crate::core::app::{
        QueueAuthorityLoadCorrelation, QueueAuthorityLoadError, QueueAuthoritySnapshot,
        QueueMutationCorrelation, QueueMutationIntent, QueueMutationResult, QueueMutationTarget,
        StartupReadySnapshot,
    };
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
    };
    use crate::domain::planning::{
        PlanningQueueMutationKind, PlanningQueueMutationReceipt, PlanningQueueMutationReceiptEntry,
        PlanningValidationReport, PriorityQueueProjection, PriorityQueueSkippedTask,
        PriorityQueueTask, TaskStatus,
    };
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    fn key(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> event::KeyEvent {
        event::KeyEvent::new(code, modifiers)
    }

    fn arm_pending_approval(
        app: &mut NativeTuiApp,
        request: ConversationApprovalRequest,
    ) -> crate::core::app::TurnSubmissionCorrelation {
        let turn_submission = app.core_runtime.begin_test_turn_submission();
        app.dispatch_core_input(crate::core::app::CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: crate::core::app::TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-approval".to_string(),
                title: "Approval test".to_string(),
                cwd: "/tmp/root".to_string(),
                runtime_envelope: Box::default(),
            },
        });
        app.dispatch_core_input(crate::core::app::CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: crate::core::app::TurnStreamEvent::TurnStarted {
                turn_id: "turn-approval".to_string(),
                runtime_request: Box::default(),
            },
        });
        app.dispatch_core_input(crate::core::app::CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: crate::core::app::TurnStreamEvent::ApprovalRequested { request },
        });
        turn_submission
    }

    fn open_simple_review(app: &mut NativeTuiApp) {
        app.shell_overlay = ShellOverlay::PlanningInit;
        app.planning_init_overlay_ui_state
            .open_simple_review(PlanningInitStageResult {
                mode: PlanningBootstrapMode::Simple,
                draft_name: "bootstrap-1".to_string(),
                draft_directory: "/tmp/bootstrap-1".to_string(),
                staged_files: Vec::new(),
                staged_file_count: 4,
                validation_report: PlanningValidationReport::default(),
            });
    }

    fn steer_correlation(generation: u64) -> TurnSteerCorrelation {
        TurnSteerCorrelation::new(
            generation,
            crate::core::app::TurnSubmissionCorrelation::new(7),
        )
    }

    fn steer_intent(
        generation: u64,
        input_revision: u64,
        source_input_buffer: &str,
        request: ConversationTurnSteerRequest,
    ) -> PendingTurnSteerUiIntent {
        PendingTurnSteerUiIntent {
            correlation: steer_correlation(generation),
            intent: TurnSteerUiIntent {
                input_revision,
                source_input_buffer: source_input_buffer.to_string(),
                request,
            },
        }
    }

    fn startup_ready_snapshot(can_continue: bool) -> Box<StartupReadySnapshot> {
        Box::new(StartupReadySnapshot::from_diagnostics(StartupDiagnostics {
            cwd: "/tmp/root".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "ok".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/root".to_string(),
            workspace_detail: "ok".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_ok: true,
            initialize_detail: "ok".to_string(),
            account_ok: can_continue,
            account_detail: if can_continue {
                "ok"
            } else {
                "missing account"
            }
            .to_string(),
            warnings: Vec::new(),
            schema_snapshot: "schema".to_string(),
        }))
    }

    fn auto_follow_origin() -> PromptOrigin {
        PromptOrigin::AutoFollow(Box::new(AutoFollowSubmitContext {
            completed_turn_id: "turn-1".to_string(),
            mode_label: "planning queue".to_string(),
            transcript_text: "queued transcript".to_string(),
            debug_detail: None,
            handoff_task: None,
        }))
    }

    fn command(input: &str) -> InlineShellCommandInput {
        InlineShellCommandInput::parse(input).expect("inline shell command should parse")
    }

    fn ready_conversation(app: &NativeTuiApp) -> &ConversationViewModel {
        match &app.conversation_state {
            ConversationState::Ready(conversation) => conversation,
            other => panic!("expected ready conversation, got {other:?}"),
        }
    }

    fn ready_conversation_mut(app: &mut NativeTuiApp) -> &mut ConversationViewModel {
        match &mut app.conversation_state {
            ConversationState::Ready(conversation) => conversation,
            other => panic!("expected ready conversation, got {other:?}"),
        }
    }

    fn single_queue_projection(
        task_id: &str,
        task_title: &str,
        planning_revision: i64,
    ) -> PlanningRuntimeProjection {
        let mut task = sample_queue_head();
        task.task_id = task_id.to_string();
        task.task_title = task_title.to_string();
        PlanningRuntimeProjection::ready_with_queue_projection(
            "context".to_string(),
            "queue".to_string(),
            None,
            Some(task.clone()),
            PriorityQueueProjection {
                next_task: Some(task.clone()),
                active_tasks: vec![task],
                proposed_tasks: Vec::new(),
                skipped_tasks: Vec::new(),
            },
        )
        .with_planning_revision(Some(planning_revision))
    }

    struct GatedPlanningTaskRepository {
        inner: NoopPlanningTaskRepositoryPort,
        count_mutations: AtomicBool,
        gate_next_mutation: AtomicBool,
        mutation_count: AtomicUsize,
        mutation_thread_id: Mutex<Option<ThreadId>>,
        mutation_started: mpsc::SyncSender<()>,
        release_mutation: Mutex<mpsc::Receiver<()>>,
        gate_next_authority_load: AtomicBool,
        authority_load_thread_id: Mutex<Option<ThreadId>>,
        authority_load_started: mpsc::SyncSender<()>,
        release_authority_load: Mutex<mpsc::Receiver<()>>,
    }

    struct GatedPlanningTaskRepositoryControls {
        mutation_started: mpsc::Receiver<()>,
        release_mutation: mpsc::SyncSender<()>,
        authority_load_started: mpsc::Receiver<()>,
        release_authority_load: mpsc::SyncSender<()>,
    }

    impl GatedPlanningTaskRepository {
        fn new() -> (Arc<Self>, GatedPlanningTaskRepositoryControls) {
            let (mutation_started, started_rx) = mpsc::sync_channel(1);
            let (release_tx, release_mutation) = mpsc::sync_channel(1);
            let (authority_load_started, authority_started_rx) = mpsc::sync_channel(1);
            let (release_authority_tx, release_authority_load) = mpsc::sync_channel(1);
            (
                Arc::new(Self {
                    inner: NoopPlanningTaskRepositoryPort,
                    count_mutations: AtomicBool::new(false),
                    gate_next_mutation: AtomicBool::new(false),
                    mutation_count: AtomicUsize::new(0),
                    mutation_thread_id: Mutex::new(None),
                    mutation_started,
                    release_mutation: Mutex::new(release_mutation),
                    gate_next_authority_load: AtomicBool::new(false),
                    authority_load_thread_id: Mutex::new(None),
                    authority_load_started,
                    release_authority_load: Mutex::new(release_authority_load),
                }),
                GatedPlanningTaskRepositoryControls {
                    mutation_started: started_rx,
                    release_mutation: release_tx,
                    authority_load_started: authority_started_rx,
                    release_authority_load: release_authority_tx,
                },
            )
        }

        fn arm(&self) {
            self.mutation_count.store(0, Ordering::SeqCst);
            self.count_mutations.store(true, Ordering::SeqCst);
            self.gate_next_mutation.store(true, Ordering::SeqCst);
        }

        fn mutation_count(&self) -> usize {
            self.mutation_count.load(Ordering::SeqCst)
        }

        fn mutation_thread_id(&self) -> Option<ThreadId> {
            *self
                .mutation_thread_id
                .lock()
                .expect("mutation thread id lock should not be poisoned")
        }

        fn arm_authority_load(&self) {
            self.gate_next_authority_load.store(true, Ordering::SeqCst);
        }

        fn authority_load_thread_id(&self) -> Option<ThreadId> {
            *self
                .authority_load_thread_id
                .lock()
                .expect("authority load thread id lock should not be poisoned")
        }
    }

    impl PlanningTaskRepositoryPort for GatedPlanningTaskRepository {
        fn load_direction_authority_snapshot(
            &self,
            workspace_dir: &str,
        ) -> anyhow::Result<Option<PlanningDirectionAuthoritySnapshot>> {
            self.inner.load_direction_authority_snapshot(workspace_dir)
        }

        fn commit_direction_authority_snapshot(
            &self,
            workspace_dir: &str,
            commit: PlanningDirectionAuthorityCommit<'_>,
        ) -> anyhow::Result<PlanningTaskAuthorityCommitResult> {
            self.inner
                .commit_direction_authority_snapshot(workspace_dir, commit)
        }

        fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> anyhow::Result<()> {
            self.inner.clear_direction_authority_snapshot(workspace_dir)
        }

        fn load_task_authority_snapshot(
            &self,
            workspace_dir: &str,
        ) -> anyhow::Result<Option<PlanningTaskAuthoritySnapshot>> {
            if self.gate_next_authority_load.swap(false, Ordering::SeqCst) {
                *self
                    .authority_load_thread_id
                    .lock()
                    .expect("authority load thread id lock should not be poisoned") =
                    Some(std::thread::current().id());
                let _ = self.authority_load_started.send(());
                self.release_authority_load
                    .lock()
                    .expect("authority load release lock should not be poisoned")
                    .recv()
                    .expect("test should release the queue authority load");
            }
            self.inner.load_task_authority_snapshot(workspace_dir)
        }

        fn commit_task_authority_snapshot(
            &self,
            workspace_dir: &str,
            commit: PlanningTaskAuthorityCommit<'_>,
        ) -> anyhow::Result<PlanningTaskAuthorityCommitResult> {
            self.inner
                .commit_task_authority_snapshot(workspace_dir, commit)
        }

        fn commit_task_authority_mutation_snapshot(
            &self,
            workspace_dir: &str,
            commit: PlanningTaskAuthorityCommit<'_>,
            audit: PlanningTaskAuthorityMutationAudit<'_>,
        ) -> anyhow::Result<PlanningTaskAuthorityCommitResult> {
            if self.count_mutations.load(Ordering::SeqCst) {
                self.mutation_count.fetch_add(1, Ordering::SeqCst);
            }
            if self.gate_next_mutation.swap(false, Ordering::SeqCst) {
                *self
                    .mutation_thread_id
                    .lock()
                    .expect("mutation thread id lock should not be poisoned") =
                    Some(std::thread::current().id());
                let _ = self.mutation_started.send(());
                self.release_mutation
                    .lock()
                    .expect("mutation release lock should not be poisoned")
                    .recv()
                    .expect("test should release the queued mutation");
            }
            self.inner
                .commit_task_authority_mutation_snapshot(workspace_dir, commit, audit)
        }

        fn load_task_authority_mutations(
            &self,
            workspace_dir: &str,
            after_planning_revision: i64,
            through_planning_revision: i64,
        ) -> anyhow::Result<Vec<PlanningTaskAuthorityMutationRecord>> {
            self.inner.load_task_authority_mutations(
                workspace_dir,
                after_planning_revision,
                through_planning_revision,
            )
        }

        fn commit_planning_authority_snapshot(
            &self,
            workspace_dir: &str,
            commit: PlanningAuthoritySnapshotCommit<'_>,
        ) -> anyhow::Result<PlanningTaskAuthorityCommitResult> {
            self.inner
                .commit_planning_authority_snapshot(workspace_dir, commit)
        }

        fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> anyhow::Result<()> {
            self.inner.clear_task_authority_snapshot(workspace_dir)
        }
    }

    fn take_next_queue_mutation_completion(
        app: &mut NativeTuiApp,
    ) -> (QueueMutationCorrelation, QueueMutationResult) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(outcome) = app.core_runtime.poll_pending_input() {
                for event in outcome.events {
                    match event {
                        AppEvent::QueueMutationCompleted {
                            correlation,
                            result,
                        } => return (correlation, *result),
                        event => app.apply_core_event(event),
                    }
                }
            }
            std::thread::yield_now();
        }
        panic!("queue mutation core effect should complete");
    }

    fn apply_next_queue_mutation_completion(app: &mut NativeTuiApp) {
        let (correlation, completion) = take_next_queue_mutation_completion(app);
        app.apply_queue_mutation_completion(correlation, completion);
    }

    fn test_queue_mutation_correlation(
        generation: u64,
        context: queue_overlay_ui::QueueMutationContext,
        kind: queue_overlay_ui::QueueMutationKind,
        expected_planning_revision: i64,
        targets: Vec<QueueMutationTarget>,
        receipt_at_start: Option<PlanningQueueMutationReceipt>,
    ) -> QueueMutationCorrelation {
        QueueMutationCorrelation::new(
            generation,
            QueueMutationIntent {
                workspace_directory: context.workspace_directory,
                active_thread_id: context.active_thread_id,
                kind,
                expected_planning_revision,
                targets,
                receipt_at_start,
            },
        )
    }

    fn record_test_queue_mutation(
        app: &mut NativeTuiApp,
        kind: queue_overlay_ui::QueueMutationKind,
        expected_planning_revision: i64,
        targets: Vec<QueueMutationTarget>,
        receipt_at_start: Option<PlanningQueueMutationReceipt>,
    ) -> QueueMutationCorrelation {
        let correlation = test_queue_mutation_correlation(
            1,
            app.current_queue_mutation_context(),
            kind,
            expected_planning_revision,
            targets,
            receipt_at_start,
        );
        app.apply_queue_mutation_started(correlation.clone());
        correlation
    }

    fn apply_next_queue_overlay_authority_load(app: &mut NativeTuiApp) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            app.poll_core_runtime_inputs(1);
            if !matches!(
                app.queue_overlay_ui_state.authority_screen_model(),
                queue_overlay_ui::QueueOverlayAuthorityScreenModel::Loading { .. }
            ) {
                return;
            }
            std::thread::yield_now();
        }
        panic!("queue authority worker should complete");
    }

    fn begin_test_queue_overlay_authority_load(
        app: &mut NativeTuiApp,
        generation: u64,
    ) -> queue_overlay_ui::QueueOverlayAuthorityLoadRequest {
        let context = app.current_queue_mutation_context();
        app.begin_queue_overlay_authority_load(QueueAuthorityLoadCorrelation::new(
            generation,
            context.workspace_directory,
            context.active_thread_id,
        ))
    }

    fn status_text(app: &NativeTuiApp) -> &str {
        &ready_conversation(app).status_text
    }

    fn poll_core_until_status_contains(app: &mut NativeTuiApp, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !status_text(app).contains(expected) && Instant::now() < deadline {
            app.poll_core_runtime_inputs(16);
            std::thread::yield_now();
        }
        assert!(
            status_text(app).contains(expected),
            "Core completion should update status with `{expected}`"
        );
    }

    #[test]
    fn startup_action_availability_drives_submission_status_copy() {
        let mut app = test_native_tui_app();

        app.startup_state = StartupState::Idle;
        assert_eq!(
            app.shell_action_availability(),
            ShellActionAvailability::Pending
        );
        assert!(!app.shell_action_availability().allows_actions());
        assert_eq!(
            app.submission_blocked_status(auto_follow_origin()),
            "auto-follow paused while startup checks are still running"
        );

        app.startup_state = StartupState::Loading;
        assert_eq!(
            app.submission_blocked_status(PromptOrigin::Manual),
            "startup checks still running; open diagnostics with Ctrl+d"
        );

        app.startup_state = StartupState::Failed("boom".to_string());
        assert_eq!(
            app.submission_blocked_status(auto_follow_origin()),
            "auto-follow paused because startup diagnostics need attention"
        );

        app.startup_state = StartupState::Ready(startup_ready_snapshot(false));
        assert!(!app.can_open_session_list());
        assert_eq!(
            app.shell_action_availability(),
            ShellActionAvailability::Blocked
        );

        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        assert!(app.can_open_session_list());
        assert_eq!(
            app.shell_action_availability(),
            ShellActionAvailability::Ready
        );
        assert!(app.shell_action_availability().allows_actions());
        assert_eq!(
            app.shell_action_availability().status_text(),
            "startup ready"
        );
        assert_eq!(app.submission_blocked_status(PromptOrigin::Manual), "ready");
    }

    #[test]
    fn turn_budget_editor_key_path_preserves_cancels_commits_and_expires_its_draft() {
        let mut app = test_native_tui_app();
        open_simple_review(&mut app);

        assert!(
            app.handle_shell_overlay_key(modified_key(KeyCode::Char('l'), KeyModifiers::CONTROL))
        );
        for _ in 0..3 {
            assert!(app.handle_shell_overlay_key(key(KeyCode::Backspace)));
        }
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Enter)));
        assert_eq!(app.max_auto_turns_edit_buffer(), Some("x"));
        assert_eq!(app.current_max_auto_turns_label(), "off");

        assert!(app.handle_shell_overlay_key(key(KeyCode::Esc)));
        assert_eq!(app.max_auto_turns_edit_buffer(), None);
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);

        assert!(
            app.handle_shell_overlay_key(modified_key(KeyCode::Char('l'), KeyModifiers::CONTROL))
        );
        for _ in 0..3 {
            assert!(app.handle_shell_overlay_key(key(KeyCode::Backspace)));
        }
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('1'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('2'))));
        let status = build_planning_init_overlay_view(&app)
            .status_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(status.contains("value: 12"));

        assert!(app.handle_shell_overlay_key(key(KeyCode::Enter)));
        assert_eq!(app.max_auto_turns_edit_buffer(), None);
        assert_eq!(app.current_max_auto_turns_label(), "12");

        assert!(
            app.handle_shell_overlay_key(modified_key(KeyCode::Char('l'), KeyModifiers::CONTROL))
        );
        assert_eq!(app.max_auto_turns_edit_buffer(), Some("12"));
        app.show_model_selection_overlay();
        assert_eq!(app.shell_overlay, ShellOverlay::ModelSelection);
        assert_eq!(app.max_auto_turns_edit_buffer(), None);

        open_simple_review(&mut app);
        assert!(!app.is_max_auto_turns_editing());
        assert!(
            app.handle_shell_overlay_key(modified_key(KeyCode::Char('l'), KeyModifiers::CONTROL))
        );
        app.close_shell_overlay();
        assert_eq!(app.max_auto_turns_edit_buffer(), None);

        open_simple_review(&mut app);
        assert!(!app.is_max_auto_turns_editing());
    }

    #[test]
    fn close_shell_overlay_resets_overlay_local_buffers() {
        let mut app = test_native_tui_app();

        app.shell_overlay = ShellOverlay::Activity;
        app.progressive_activity_overlay_ui_state
            .reset_for_kind(ProgressiveActivityDetailKind::Output);
        app.progressive_activity_overlay_ui_state
            .select_document(1, Some(8));
        app.progressive_activity_overlay_ui_state
            .set_page_window(4, Some(8));
        app.close_shell_overlay();
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            app.progressive_activity_overlay_ui_state,
            ProgressiveActivityOverlayUiState::default()
        );

        app.shell_overlay = ShellOverlay::Reviews;
        app.begin_reviews_overlay_load(crate::core::app::ReviewCenterLoadCorrelation::new(
            1,
            "/tmp/root",
            None,
        ));
        assert!(matches!(
            app.reviews_overlay_ui_state.screen_model(),
            crate::adapter::inbound::tui::app::reviews_overlay_ui::ReviewsOverlayScreenModel::Loading(_)
        ));
        app.close_shell_overlay();
        assert!(matches!(
            app.reviews_overlay_ui_state.screen_model(),
            crate::adapter::inbound::tui::app::reviews_overlay_ui::ReviewsOverlayScreenModel::Idle
        ));

        for overlay in [
            ShellOverlay::DirectionsMaintenance,
            ShellOverlay::PlanningInit,
            ShellOverlay::ModelSelection,
            ShellOverlay::ViewSelection,
            ShellOverlay::LanguageSelection,
            ShellOverlay::ParallelPeek,
            ShellOverlay::Queue,
        ] {
            app.shell_overlay = overlay;
            app.close_shell_overlay();
            assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        }
    }

    #[test]
    fn reviews_overlay_load_does_not_start_while_approval_owns_focus() {
        let mut app = test_native_tui_app();
        app.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);

        app.show_reviews_overlay();

        assert_eq!(app.shell_overlay, ShellOverlay::Approval);
        assert!(matches!(
            app.reviews_overlay_ui_state.screen_model(),
            crate::adapter::inbound::tui::app::reviews_overlay_ui::ReviewsOverlayScreenModel::Idle
        ));
        assert!(app.rx.try_recv().is_err());
    }

    #[test]
    fn activity_commands_default_validate_alias_and_preserve_approval_focus() {
        let mut app = test_native_tui_app();

        app.execute_inline_shell_command_input(command(":activity"));
        assert_eq!(app.shell_overlay, ShellOverlay::Activity);
        assert_eq!(
            app.progressive_activity_overlay_ui_state.card_filter(),
            None
        );

        app.close_shell_overlay();
        app.execute_inline_shell_command_input(command(":act output"));
        assert_eq!(app.shell_overlay, ShellOverlay::Activity);
        assert_eq!(
            app.progressive_activity_overlay_ui_state.selected_kind(),
            ProgressiveActivityDetailKind::Output
        );
        assert_eq!(
            app.progressive_activity_overlay_ui_state.card_filter(),
            Some(super::ProgressiveActivityCardKind::Command)
        );

        app.close_shell_overlay();
        app.execute_inline_shell_command_input(command(":activity all"));
        assert_eq!(app.shell_overlay, ShellOverlay::Activity);
        assert_eq!(
            app.progressive_activity_overlay_ui_state.card_filter(),
            None
        );

        app.close_shell_overlay();
        app.execute_inline_shell_command_input(command(":activity nope"));
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert!(status_text(&app).contains("activity unchanged; supported values:"));

        app.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);
        assert!(!app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Output));
        assert_eq!(app.shell_overlay, ShellOverlay::Approval);
    }

    #[test]
    fn activity_overlay_keymap_navigates_pages_kinds_home_and_close() {
        let mut app = test_native_tui_app();
        assert!(app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Output));
        app.progressive_activity_overlay_ui_state
            .select_document(1, Some(7));
        app.progressive_activity_overlay_ui_state
            .set_page_window(0, Some(10));

        // Up/Down select cards; with an empty card list the index stays at 0 and page is reset.
        assert!(app.handle_shell_overlay_key(key(KeyCode::Down)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .selected_card_index(),
            0
        );
        app.progressive_activity_overlay_ui_state
            .set_page_window(0, Some(10));
        assert!(app.handle_shell_overlay_key(key(KeyCode::PageDown)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .current_page_start(),
            10
        );
        app.progressive_activity_overlay_ui_state
            .set_page_window(10, Some(20));
        assert!(app.handle_shell_overlay_key(key(KeyCode::PageDown)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .current_page_start(),
            20
        );
        assert!(app.handle_shell_overlay_key(key(KeyCode::PageUp)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .current_page_start(),
            10
        );

        assert!(app.handle_shell_overlay_key(key(KeyCode::Home)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.selected_kind(),
            ProgressiveActivityDetailKind::Output
        );
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .current_document_sequence(),
            None
        );
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .current_page_start(),
            0
        );

        app.progressive_activity_overlay_ui_state
            .select_document(1, Some(8));
        // Output/Command filter cycles next to Patch in the shared filter wheel.
        assert!(app.handle_shell_overlay_key(key(KeyCode::Tab)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.card_filter(),
            Some(super::ProgressiveActivityCardKind::Patch)
        );
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .current_document_sequence(),
            None
        );
        // Filter keys always advance the shared cycle (including BackTab/Left).
        assert!(app.handle_shell_overlay_key(modified_key(KeyCode::BackTab, KeyModifiers::SHIFT)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.card_filter(),
            Some(super::ProgressiveActivityCardKind::Diff)
        );
        assert!(app.handle_shell_overlay_key(key(KeyCode::Left)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.card_filter(),
            Some(super::ProgressiveActivityCardKind::Mcp)
        );
        assert!(app.handle_shell_overlay_key(key(KeyCode::Right)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.card_filter(),
            Some(super::ProgressiveActivityCardKind::Plan)
        );

        assert!(app.handle_shell_overlay_key(key(KeyCode::Esc)));
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            app.progressive_activity_overlay_ui_state,
            ProgressiveActivityOverlayUiState::default()
        );
    }

    #[test]
    fn help_overlay_keymap_scrolls_resets_and_defers_final_clamp_to_rendering() {
        let mut app = test_native_tui_app();
        app.help_scroll_offset = 9;

        app.show_help_overlay();
        assert_eq!(app.shell_overlay, ShellOverlay::Help);
        assert_eq!(app.help_scroll_offset, 0);

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('j'))));
        assert_eq!(app.help_scroll_offset, 1);
        assert!(app.handle_shell_overlay_key(key(KeyCode::PageDown)));
        assert_eq!(app.help_scroll_offset, 6);
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('k'))));
        assert_eq!(app.help_scroll_offset, 5);
        assert!(app.handle_shell_overlay_key(key(KeyCode::PageUp)));
        assert_eq!(app.help_scroll_offset, 0);

        assert!(app.handle_shell_overlay_key(key(KeyCode::End)));
        assert_eq!(app.help_scroll_offset, usize::MAX);
        assert!(app.handle_shell_overlay_key(key(KeyCode::Home)));
        assert_eq!(app.help_scroll_offset, 0);
    }

    #[test]
    fn inline_commands_cover_argument_status_and_stop_paths() {
        let mut app = test_native_tui_app();

        app.set_parallel_mode_enabled_for_test(true);
        app.execute_inline_shell_command_input(command(":sessions"));
        assert_eq!(app.shell_overlay, ShellOverlay::Supersession);
        assert!(status_text(&app).contains("opened supersession control tower"));

        app.set_parallel_mode_enabled_for_test(false);
        app.execute_inline_shell_command_input(command(":help"));
        assert_eq!(app.shell_overlay, ShellOverlay::Help);
        assert!(status_text(&app).contains("opened shell command help"));
        app.execute_inline_shell_command_input(command(":reviews"));
        assert_eq!(app.shell_overlay, ShellOverlay::Reviews);
        assert!(status_text(&app).contains("opened review center inspection"));

        app.execute_inline_shell_command_input(command(":turns 4"));
        assert_eq!(
            ready_conversation(&app)
                .auto_follow_state
                .max_auto_turns_label(),
            "4"
        );
        assert!(status_text(&app).contains("auto-follow enabled"));

        app.execute_inline_shell_command_input(command(":turns off"));
        assert_eq!(
            ready_conversation(&app)
                .auto_follow_state
                .max_auto_turns_label(),
            "off"
        );
        assert!(status_text(&app).contains("auto-follow disabled"));

        app.execute_inline_shell_command_input(command(":turns infinite"));
        assert_eq!(
            ready_conversation(&app)
                .auto_follow_state
                .max_auto_turns_label(),
            "infinite"
        );

        app.execute_inline_shell_command_input(command(":model default"));
        assert_eq!(app.turn_options.model, None);
        assert!(status_text(&app).contains("model reset to app-server default"));

        app.execute_inline_shell_command_input(command(":view unsupported"));
        assert_eq!(app.shell_overlay, ShellOverlay::ViewSelection);
        assert!(status_text(&app).contains("view unchanged"));

        app.execute_inline_shell_command_input(command(":language klingon"));
        assert_eq!(app.shell_overlay, ShellOverlay::LanguageSelection);
        assert!(status_text(&app).contains("language unchanged"));

        app.execute_inline_shell_command_input(command(":think"));
        assert!(status_text(&app).contains("think override unchanged"));

        app.execute_inline_shell_command_input(command(":think unknown"));
        assert!(status_text(&app).contains("supported values"));

        let parallel_workspace = app.planning_workspace_directory();
        app.open_parallel_mode_automation_epoch(parallel_workspace.clone());
        app.set_parallel_mode_enabled_for_test(true);
        let parallel_epoch = app
            .parallel_mode_automation_epoch_id()
            .expect("parallel epoch should be open before stop");
        assert!(
            app.parallel_mode_control_plane
                .automation_epoch_is_active(&parallel_workspace, parallel_epoch)
        );

        app.execute_inline_shell_command_input(command(":stop"));
        assert!(status_text(&app).contains("no active turn is running"));
        assert!(!app.parallel_mode_enabled());
        assert!(app.parallel_mode_automation_epoch_id().is_none());
        assert!(
            !app.parallel_mode_control_plane
                .automation_epoch_is_active(&parallel_workspace, parallel_epoch),
            ":stop must close the automation permit before another dispatch can start"
        );
        assert!(
            ready_conversation(&app)
                .auto_follow_state
                .post_turn_continuation_paused()
        );

        ready_conversation_mut(&mut app)
            .auto_follow_state
            .reset_for_manual_turn();
        assert!(
            ready_conversation(&app)
                .auto_follow_state
                .post_turn_continuation_paused(),
            "manual turns must not re-arm automation after :stop"
        );
        assert!(
            !ready_conversation(&app)
                .auto_follow_state
                .parallel_post_turn_continuation_allowed(),
            "manual turns must not re-arm the parallel continuation path"
        );

        app.execute_inline_shell_command_input(command(":parallel"));
        assert!(
            ready_conversation(&app)
                .auto_follow_state
                .post_turn_continuation_paused(),
            "parallel opt-in must not clear the single-session stop"
        );
        assert!(
            ready_conversation(&app)
                .auto_follow_state
                .parallel_post_turn_continuation_allowed(),
            "explicit parallel opt-in should re-arm only the parallel continuation path"
        );
        app.execute_inline_shell_command_input(command(":parallel off"));
        assert!(
            !ready_conversation(&app)
                .auto_follow_state
                .parallel_post_turn_continuation_allowed()
        );

        app.execute_inline_shell_command_input(command(":turns 2"));
        assert!(
            ready_conversation(&app).auto_follow_state.can_queue_next(),
            "only an explicit positive :turns command should re-arm automation"
        );

        let mut app = test_native_tui_app();
        let turn_submission = app.core_runtime.begin_test_turn_submission();
        app.dispatch_core_input(crate::core::app::CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: crate::core::app::TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        });
        app.execute_inline_shell_command_input(command(":stop"));
        assert!(
            status_text(&app).contains("active app-server sessions"),
            "unexpected stop status: {}",
            status_text(&app)
        );
        app.execute_inline_shell_command_input(command(":stop"));
        assert!(status_text(&app).contains("stop already requested"));
    }

    #[test]
    fn ctrl_c_interrupts_a_running_turn_once_and_keeps_idle_navigation_semantics() {
        let mut app = test_native_tui_app();
        let turn_submission = app.core_runtime.begin_test_turn_submission();
        ready_conversation_mut(&mut app).mark_turn_submitting("/tmp/root".to_string());

        app.handle_ctrl_c();
        assert!(status_text(&app).contains("stop requested"));
        app.dispatch_core_input(crate::core::app::CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: crate::core::app::TurnStreamEvent::TurnStarted {
                turn_id: "turn-ctrl-c".to_string(),
                runtime_request: Box::default(),
            },
        });
        poll_core_until_status_contains(&mut app, "stop synchronized");
        assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Hidden);

        app.handle_ctrl_c();
        assert!(status_text(&app).contains("stop already requested"));

        let _ = app.core_runtime.dispatch_input(
            crate::core::app::CoreInput::ConversationStreamUpdated {
                correlation: turn_submission,
                event: crate::core::app::TurnStreamEvent::Failed {
                    message: "turn stopped".to_string(),
                },
            },
        );
        ready_conversation_mut(&mut app).mark_turn_finished();
        app.handle_ctrl_c();
        assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Hidden);
        assert!(ready_conversation(&app).is_blank_draft());
        app.handle_ctrl_c();
        assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Visible);
    }

    #[test]
    fn prompt_input_wrappers_and_palette_acceptance_route_through_input_reducer() {
        let mut app = test_native_tui_app();

        assert!(!app.insert_input_text(String::new()));

        app.shell_overlay = ShellOverlay::Queue;
        assert!(!app.can_edit_prompt_input());
        assert!(!app.insert_input_text("blocked".to_string()));

        app.shell_overlay = ShellOverlay::Supersession;
        app.set_parallel_mode_enabled_for_test(true);
        assert!(!app.can_edit_prompt_input());

        app.shell_overlay = ShellOverlay::Hidden;
        assert!(app.can_edit_prompt_input());
        assert!(app.insert_input_text("abc".to_string()));
        app.move_input_cursor(InputCursorMovement::LineStart);
        app.push_input_character('z');
        app.move_input_cursor(InputCursorMovement::BufferEnd);
        app.insert_input_newline();
        app.delete_previous_input_word();
        app.pop_input_character();
        app.delete_next_input_character();
        app.clear_prompt_input();
        assert!(ready_conversation(&app).input_buffer.is_empty());

        assert!(!app.move_inline_command_palette_selection(1));
        assert!(!app.dismiss_inline_command_palette());
        assert!(!app.accept_inline_command_palette_selection());

        app.push_input_character(':');
        app.push_input_character('t');
        assert!(app.is_inline_command_palette_active());
        assert!(app.accept_inline_command_palette_selection());
        assert!(ready_conversation(&app).input_buffer.starts_with(":turns"));

        app.clear_prompt_input();
        app.push_input_character(':');
        app.push_input_character('h');
        assert!(app.move_inline_command_palette_selection(0));
        assert!(app.accept_inline_command_palette_selection());
        assert_eq!(app.shell_overlay, ShellOverlay::Help);
    }

    #[test]
    fn queue_overlay_consumes_direct_manipulation_keys_without_editing_prompt() {
        let mut app = test_native_tui_app();
        ready_conversation_mut(&mut app).input_buffer = "keep draft".to_string();
        app.sync_ready_conversation_planning_runtime_projection(
            sample_planning_runtime_projection("context", "queue").with_planning_revision(Some(1)),
        );
        app.shell_overlay = ShellOverlay::Queue;
        app.bind_queue_overlay_authority_for_test(
            1,
            std::collections::BTreeMap::from([
                (
                    "task-1".to_string(),
                    queue_overlay_ui::QueueOverlayAuthorityToken {
                        status: TaskStatus::Ready,
                        updated_at: "2026-04-10T00:00:00Z".to_string(),
                    },
                ),
                (
                    "task-2".to_string(),
                    queue_overlay_ui::QueueOverlayAuthorityToken {
                        status: TaskStatus::Ready,
                        updated_at: "2026-04-10T01:00:00Z".to_string(),
                    },
                ),
            ]),
        );

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('j'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('u'))));

        assert_eq!(ready_conversation(&app).input_buffer, "keep draft");
        assert_eq!(app.pending_queue_mutation_operation_id(), Some(1));
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("Queue change op-1 is waiting for authority acknowledgement.")
        );
        let _completion = take_next_queue_mutation_completion(&mut app);
    }

    #[test]
    fn queue_loading_and_failed_authority_states_block_destructive_input() {
        let mut app = test_native_tui_app();
        app.sync_ready_conversation_planning_runtime_projection(single_queue_projection(
            "task-read-only",
            "Read-only task",
            7,
        ));
        app.shell_overlay = ShellOverlay::Queue;
        let request = begin_test_queue_overlay_authority_load(&mut app, 1);

        app.cancel_selected_queue_task();
        assert!(!app.undo_latest_queue_registration());
        assert_eq!(app.pending_queue_mutation_operation_id(), None);
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("Queue authority is still loading; remove and undo remain disabled.")
        );
        assert!(matches!(app.rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        assert!(
            app.queue_overlay_ui_state
                .apply_authority_load_failed(request.clone(), "database unavailable".to_string(),)
        );
        app.queue_mutation_ui_state.require_authority_refresh();
        app.cancel_selected_queue_task();
        assert!(!app.undo_latest_queue_registration());
        assert_eq!(app.pending_queue_mutation_operation_id(), None);
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("queue authority load-1 failed: database unavailable")
        );
        assert!(matches!(app.rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn queue_action_reloads_a_failed_authority_from_a_stale_context() {
        let mut app = test_native_tui_app();
        app.sync_ready_conversation_planning_runtime_projection(single_queue_projection(
            "task-stale",
            "Stale authority task",
            7,
        ));
        app.shell_overlay = ShellOverlay::Queue;
        let stale_request = begin_test_queue_overlay_authority_load(&mut app, 1);
        assert!(
            app.queue_overlay_ui_state
                .apply_authority_load_failed(stale_request, "database unavailable".to_string())
        );
        ready_conversation_mut(&mut app)
            .sync_draft_workspace("/tmp/queue-stale-failed-context".to_string());

        app.cancel_selected_queue_task();

        assert_eq!(app.pending_queue_mutation_operation_id(), None);
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("Queue authority is still loading; remove and undo remain disabled.")
        );
        assert!(matches!(
            app.queue_overlay_ui_state.authority_screen_model(),
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Loading { .. }
        ));
        let replacement_context = app.current_queue_mutation_context();
        let replacement_correlation = QueueAuthorityLoadCorrelation::new(
            1,
            replacement_context.workspace_directory.clone(),
            replacement_context.active_thread_id.clone(),
        );
        assert_eq!(
            app.queue_overlay_ui_state
                .loading_request(&replacement_correlation)
                .expect("replacement load should bind the current context")
                .correlation
                .workspace_directory,
            "/tmp/queue-stale-failed-context"
        );
    }

    #[test]
    fn queue_close_reopen_ignores_old_authority_completion_before_applying_the_new_one() {
        let mut app = test_native_tui_app();
        app.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);
        let old_request = begin_test_queue_overlay_authority_load(&mut app, 1);
        app.close_shell_overlay();
        app.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);
        let new_request = begin_test_queue_overlay_authority_load(&mut app, 2);
        assert!(new_request.correlation.generation > old_request.correlation.generation);

        assert_eq!(
            app.apply_queue_overlay_authority_loaded(
                old_request.correlation,
                Err(QueueAuthorityLoadError::RuntimeProjectionUnavailable),
            ),
            queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Ignored
        );
        assert!(app.queue_overlay_ui_state.is_loading_request(&new_request));

        assert_eq!(
            app.apply_queue_overlay_authority_loaded(
                new_request.correlation.clone(),
                Err(QueueAuthorityLoadError::AuthorityUnavailable(
                    "database unavailable".to_string(),
                )),
            ),
            queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::Applied
        );
        assert!(matches!(
            app.queue_overlay_ui_state.authority_screen_model(),
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Failed {
                request_id,
                ..
            } if request_id == new_request.correlation.generation
        ));
    }

    #[test]
    fn queue_screen_model_keeps_rows_and_actions_on_one_authority_snapshot() {
        let mut app = test_native_tui_app();
        let authority_projection =
            single_queue_projection("task-authority", "Authority snapshot row", 7);
        app.sync_ready_conversation_planning_runtime_projection(single_queue_projection(
            "task-live",
            "Newer live row",
            8,
        ));
        app.shell_overlay = ShellOverlay::Queue;
        let request = begin_test_queue_overlay_authority_load(&mut app, 1);
        assert!(app.queue_overlay_ui_state.apply_authority_loaded(
            request,
            authority_projection,
            7,
            std::collections::BTreeMap::from([(
                "task-authority".to_string(),
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: TaskStatus::Ready,
                    updated_at: "2026-04-10T00:00:00Z".to_string(),
                },
            )]),
        ));
        app.sync_queue_overlay_selection();

        let view =
            crate::adapter::inbound::tui::app::shell_presentation::build_queue_overlay_view(&app);
        let rows = view
            .queue_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rows.contains("Authority snapshot row"), "{rows}");
        assert!(!rows.contains("Newer live row"), "{rows}");
        assert_eq!(
            app.queue_overlay_ui_state.selected_task_id(),
            Some("task-authority")
        );
        let keys = view
            .key_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(keys.contains("queue authority changed; wait for refresh"));
        assert!(!keys.contains("x/Delete"), "{keys}");
        assert!(!keys.contains("u: undo"), "{keys}");

        app.cancel_selected_queue_task();
        assert_eq!(app.pending_queue_mutation_operation_id(), None);
        assert!(matches!(
            app.queue_overlay_ui_state.authority_screen_model(),
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Loading { .. }
        ));
        apply_next_queue_overlay_authority_load(&mut app);
        assert!(!matches!(
            app.queue_overlay_ui_state.authority_screen_model(),
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Idle
        ));
    }

    #[test]
    fn queue_displayed_selection_matches_the_mutation_request_target() {
        let mut app = test_native_tui_app();
        app.sync_ready_conversation_planning_runtime_projection(single_queue_projection(
            "task-displayed",
            "Displayed authority row",
            7,
        ));
        app.shell_overlay = ShellOverlay::Queue;
        app.bind_queue_overlay_authority_for_test(
            7,
            std::collections::BTreeMap::from([(
                "task-displayed".to_string(),
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: TaskStatus::Ready,
                    updated_at: "2026-04-10T00:00:00Z".to_string(),
                },
            )]),
        );
        let view =
            crate::adapter::inbound::tui::app::shell_presentation::build_queue_overlay_view(&app);
        assert!(
            view.queue_lines
                .iter()
                .any(|line| line.to_string().contains("Displayed authority row"))
        );

        app.cancel_selected_queue_task();
        let (correlation, _) = take_next_queue_mutation_completion(&mut app);
        assert_eq!(correlation.intent.expected_planning_revision, 7);
        assert_eq!(correlation.intent.targets.len(), 1);
        assert_eq!(correlation.intent.targets[0].task_id, "task-displayed");
        assert_eq!(
            correlation.intent.targets[0].expected_status,
            TaskStatus::Ready
        );
        assert_eq!(
            correlation.intent.targets[0].expected_updated_at,
            "2026-04-10T00:00:00Z"
        );
    }

    #[test]
    fn queue_action_reloads_authority_before_using_a_drifted_workspace_context() {
        let mut app = test_native_tui_app();
        app.sync_ready_conversation_planning_runtime_projection(
            sample_planning_runtime_projection("context", "queue").with_planning_revision(Some(7)),
        );
        app.shell_overlay = ShellOverlay::Queue;
        app.bind_queue_overlay_authority_for_test(
            7,
            std::collections::BTreeMap::from([
                (
                    "task-1".to_string(),
                    queue_overlay_ui::QueueOverlayAuthorityToken {
                        status: TaskStatus::Ready,
                        updated_at: "2026-04-10T00:00:00Z".to_string(),
                    },
                ),
                (
                    "task-2".to_string(),
                    queue_overlay_ui::QueueOverlayAuthorityToken {
                        status: TaskStatus::Ready,
                        updated_at: "2026-04-10T01:00:00Z".to_string(),
                    },
                ),
            ]),
        );
        assert_eq!(
            app.queue_overlay_ui_state.selected_task_id(),
            Some("task-1")
        );

        ready_conversation_mut(&mut app)
            .sync_draft_workspace("/tmp/queue-drifted-workspace".to_string());
        app.cancel_selected_queue_task();

        assert_eq!(app.pending_queue_mutation_operation_id(), None);
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("Queue authority is still loading; remove and undo remain disabled.")
        );
        assert!(matches!(
            app.queue_overlay_ui_state.authority_screen_model(),
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Loading { .. }
        ));
        let replacement_context = app.current_queue_mutation_context();
        let replacement_correlation = QueueAuthorityLoadCorrelation::new(
            1,
            replacement_context.workspace_directory.clone(),
            replacement_context.active_thread_id.clone(),
        );
        assert_eq!(
            app.queue_overlay_ui_state
                .loading_request(&replacement_correlation)
                .expect("replacement load should bind the drifted context")
                .correlation
                .workspace_directory,
            "/tmp/queue-drifted-workspace"
        );
    }

    #[test]
    fn queue_overlay_keeps_undo_available_for_large_atomic_batch() {
        let mut app = test_native_tui_app();
        let entries = (0..17)
            .map(|index| PlanningQueueMutationReceiptEntry {
                task_id: format!("task-{index}"),
                task_title: format!("Task {index}"),
                mutation_kind: PlanningQueueMutationKind::Created,
                before_status: None,
                after_status: TaskStatus::Ready,
                after_updated_at: format!("2026-07-15T00:00:{index:02}Z"),
                unchanged_since_mutation: true,
            })
            .collect();
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt =
            Some(PlanningQueueMutationReceipt {
                completed_turn_id: "turn-large".to_string(),
                planning_revision: 17,
                entries,
            });
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(17)),
        );
        app.shell_overlay = ShellOverlay::Queue;
        app.bind_queue_overlay_authority_for_test(17, std::collections::BTreeMap::new());

        let view =
            crate::adapter::inbound::tui::app::shell_presentation::build_queue_overlay_view(&app);
        assert!(
            view.key_lines
                .iter()
                .any(|line| line.to_string().contains("u:"))
        );
        assert!(
            view.key_lines
                .iter()
                .all(|line| !line.to_string().contains("x/Delete"))
        );
    }

    #[test]
    fn unavailable_projection_does_not_discard_a_valid_queue_receipt() {
        let mut app = test_native_tui_app();
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt =
            Some(PlanningQueueMutationReceipt {
                completed_turn_id: "turn-receipt".to_string(),
                planning_revision: 7,
                entries: Vec::new(),
            });
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::invalid("temporary authority read failure"),
        );

        assert!(
            ready_conversation(&app)
                .latest_queue_mutation_receipt
                .is_some()
        );
    }

    #[test]
    fn queue_reopen_reconciles_a_preserved_stale_receipt_instead_of_dropping_it() {
        let (mut app, planning) = test_native_tui_app_with_services();
        let workspace = std::env::temp_dir()
            .join(format!(
                "akra-queue-reopen-receipt-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should follow epoch")
                    .as_nanos()
            ))
            .to_string_lossy()
            .to_string();
        std::fs::create_dir_all(&workspace).expect("queue workspace should exist");
        planning
            .workspace
            .initialize_simple_workspace(&workspace)
            .expect("planning workspace should initialize");
        ready_conversation_mut(&mut app).sync_draft_workspace(workspace.clone());
        let created = planning
            .task_tool
            .run(
                &workspace,
                serde_json::from_str::<PlanningTaskToolRequest>(
                    r#"{"version":1,"op":"create_task","apply":true,"title":"Retry after reopen","status":"ready"}"#,
                )
                .expect("task request should parse"),
            )
            .expect("task should be created");
        let authority = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("queue authority should load");
        let task = authority
            .tasks
            .iter()
            .find(|task| task.id == created.committed_task_ids[0])
            .expect("created task should exist")
            .clone();
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt =
            Some(PlanningQueueMutationReceipt {
                completed_turn_id: "turn-preserved".to_string(),
                planning_revision: authority.planning_revision.saturating_sub(1),
                entries: vec![PlanningQueueMutationReceiptEntry {
                    task_id: task.id,
                    task_title: task.title,
                    mutation_kind: PlanningQueueMutationKind::Created,
                    before_status: None,
                    after_status: task.status,
                    after_updated_at: task.updated_at,
                    unchanged_since_mutation: true,
                }],
            });
        app.queue_mutation_ui_state.require_authority_refresh();

        app.show_queue_overlay();
        apply_next_queue_overlay_authority_load(&mut app);

        let receipt = ready_conversation(&app)
            .latest_queue_mutation_receipt
            .as_ref()
            .expect("unchanged receipt should survive authority refresh");
        assert_eq!(receipt.planning_revision, authority.planning_revision);
        assert!(receipt.created_batch_is_cancellable());
        assert!(!app.queue_mutation_requires_authority_refresh());
        std::fs::remove_dir_all(&workspace).expect("queue workspace should clean up");
    }

    #[test]
    fn queue_overlay_selection_reaches_hidden_and_skipped_ready_tasks() {
        let mut app = test_native_tui_app();
        let queue_task = |rank: usize| PriorityQueueTask {
            rank,
            task_id: format!("task-{rank}"),
            direction_id: "general-workstream".to_string(),
            direction_title: "General".to_string(),
            task_title: format!("Queue task {rank}"),
            status: TaskStatus::Ready,
            combined_priority: 100 - rank as i32,
            updated_at: format!("2026-07-15T00:00:0{rank}Z"),
            rank_reasons: vec!["status=ready".to_string()],
        };
        let active_tasks = (1..=4).map(queue_task).collect::<Vec<_>>();
        let proposal = PriorityQueueTask {
            rank: 1,
            task_id: "proposal-1".to_string(),
            direction_id: "general-workstream".to_string(),
            direction_title: "General".to_string(),
            task_title: "Proposal task".to_string(),
            status: TaskStatus::Proposed,
            combined_priority: 50,
            updated_at: "2026-07-15T00:00:05Z".to_string(),
            rank_reasons: vec!["status=proposed".to_string()],
        };
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::ready_with_queue_projection(
                "context".to_string(),
                "queue ready".to_string(),
                Some("one proposal".to_string()),
                active_tasks.first().cloned(),
                PriorityQueueProjection {
                    next_task: active_tasks.first().cloned(),
                    active_tasks,
                    proposed_tasks: vec![proposal],
                    skipped_tasks: vec![PriorityQueueSkippedTask {
                        task_id: "ready-but-skipped".to_string(),
                        task_title: "Waiting on dependency".to_string(),
                        direction_id: "general-workstream".to_string(),
                        status: TaskStatus::Ready,
                        reason: "dependency task-open is ready".to_string(),
                    }],
                },
            )
            .with_planning_revision(Some(9)),
        );
        app.shell_overlay = ShellOverlay::Queue;
        app.sync_queue_overlay_selection();

        for _ in 0..3 {
            assert!(app.handle_shell_overlay_key(key(KeyCode::Char('j'))));
        }
        assert_eq!(
            app.queue_overlay_ui_state.selected_task_id(),
            Some("task-4")
        );
        let hidden_active_view =
            crate::adapter::inbound::tui::app::shell_presentation::build_queue_overlay_view(&app);
        assert!(
            hidden_active_view
                .queue_lines
                .iter()
                .any(|line| line.to_string().starts_with("> #4 [ready]"))
        );

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('j'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('j'))));
        assert_eq!(
            app.queue_overlay_ui_state.selected_task_id(),
            Some("ready-but-skipped")
        );
        let skipped_view =
            crate::adapter::inbound::tui::app::shell_presentation::build_queue_overlay_view(&app);
        assert!(
            skipped_view
                .note_lines
                .iter()
                .any(|line| line.to_string().contains("> [ready / skipped]"))
        );
    }

    #[test]
    fn queue_overlay_keeps_pause_ahead_of_selected_skip_and_feedback() {
        let mut app = test_native_tui_app();
        let queue_head = sample_queue_head();
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::ready_with_queue_projection(
                "context".to_string(),
                "queue ready".to_string(),
                None,
                Some(queue_head.clone()),
                PriorityQueueProjection {
                    next_task: Some(queue_head.clone()),
                    active_tasks: vec![queue_head],
                    proposed_tasks: Vec::new(),
                    skipped_tasks: vec![PriorityQueueSkippedTask {
                        task_id: "ready-but-skipped".to_string(),
                        task_title: "Dependency wait".to_string(),
                        direction_id: "general-workstream".to_string(),
                        status: TaskStatus::Ready,
                        reason: "dependency task is still open".to_string(),
                    }],
                },
            )
            .with_auto_follow_pause_reason("dependency authority needs operator attention"),
        );
        app.shell_overlay = ShellOverlay::Queue;
        app.sync_queue_overlay_selection();
        let task_ids = app
            .queue_action_tasks()
            .into_iter()
            .map(|task| task.task_id)
            .collect::<Vec<_>>();
        app.queue_overlay_ui_state
            .move_selection(&task_ids, task_ids.len() as isize);
        app.queue_overlay_ui_state
            .set_feedback("task cancellation feedback");

        let view =
            crate::adapter::inbound::tui::app::shell_presentation::build_queue_overlay_view(&app);
        let notes = view
            .note_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        assert_eq!(notes.len(), 2);
        assert!(notes[0].starts_with("pause: dependency authority"));
        assert!(notes[1].contains("> [ready / skipped]"));
        assert!(
            !notes
                .iter()
                .any(|line| line.contains("cancellation feedback"))
        );
    }

    #[test]
    fn queue_overlay_keeps_failure_ahead_of_feedback() {
        let mut app = test_native_tui_app();
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::invalid("planning authority could not be loaded"),
        );
        app.queue_overlay_ui_state
            .set_feedback("previous queue action completed");

        let view =
            crate::adapter::inbound::tui::app::shell_presentation::build_queue_overlay_view(&app);
        let notes = view
            .note_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        assert!(notes[0].starts_with("blocking issue: planning authority"));
        assert!(notes[1].contains("previous queue action completed"));
    }

    #[test]
    fn queue_overlay_opens_before_authority_load_completes() {
        let (repository, controls) = GatedPlanningTaskRepository::new();
        let GatedPlanningTaskRepositoryControls {
            authority_load_started,
            release_authority_load,
            ..
        } = controls;
        let planning = test_planning_services_with_task_repository(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            repository.clone(),
        );
        let (mut app, planning) = test_native_tui_app_with_planning(planning);
        let workspace = std::env::temp_dir()
            .join(format!(
                "akra-queue-authority-load-gate-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should follow epoch")
                    .as_nanos()
            ))
            .to_string_lossy()
            .to_string();
        std::fs::create_dir_all(&workspace).expect("queue workspace should exist");
        planning
            .workspace
            .initialize_simple_workspace(&workspace)
            .expect("planning workspace should initialize");
        ready_conversation_mut(&mut app).sync_draft_workspace(workspace.clone());

        repository.arm_authority_load();
        let terminal_thread_id = std::thread::current().id();
        let (cancel_watchdog, watchdog_cancelled) = mpsc::sync_channel(1);
        let watchdog_release = release_authority_load.clone();
        let watchdog = std::thread::spawn(move || {
            if watchdog_cancelled
                .recv_timeout(Duration::from_secs(2))
                .is_err()
            {
                let _ = watchdog_release.send(());
            }
        });

        let input_started = Instant::now();
        app.show_queue_overlay();
        let input_elapsed = input_started.elapsed();
        authority_load_started
            .recv_timeout(Duration::from_secs(2))
            .expect("background authority load should reach the repository gate");
        assert_eq!(app.shell_overlay, ShellOverlay::Queue);
        assert!(matches!(
            app.queue_overlay_ui_state.authority_screen_model(),
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Loading { .. }
        ));
        assert_ne!(
            repository.authority_load_thread_id(),
            Some(terminal_thread_id)
        );

        if input_elapsed < Duration::from_secs(1) {
            cancel_watchdog
                .send(())
                .expect("fast input return should cancel the deadlock watchdog");
            release_authority_load
                .send(())
                .expect("test should release the background authority load");
        }
        watchdog.join().expect("deadlock watchdog should exit");
        assert!(
            input_elapsed < Duration::from_secs(1),
            "opening the queue blocked the terminal thread for {input_elapsed:?}"
        );
        apply_next_queue_overlay_authority_load(&mut app);
        assert!(matches!(
            app.queue_overlay_ui_state.authority_screen_model(),
            queue_overlay_ui::QueueOverlayAuthorityScreenModel::Ready { .. }
        ));

        std::fs::remove_dir_all(&workspace).expect("queue workspace should clean up");
    }

    #[test]
    fn queue_mutation_returns_before_commit_and_settles_only_the_correlated_operation() {
        let (repository, controls) = GatedPlanningTaskRepository::new();
        let GatedPlanningTaskRepositoryControls {
            mutation_started,
            release_mutation,
            ..
        } = controls;
        let planning = test_planning_services_with_task_repository(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            repository.clone(),
        );
        let (mut app, planning) = test_native_tui_app_with_planning(planning);
        let workspace = std::env::temp_dir()
            .join(format!(
                "akra-queue-mutation-gate-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should follow epoch")
                    .as_nanos()
            ))
            .to_string_lossy()
            .to_string();
        std::fs::create_dir_all(&workspace).expect("queue workspace should exist");
        planning
            .workspace
            .initialize_simple_workspace(&workspace)
            .expect("planning workspace should initialize");
        ready_conversation_mut(&mut app).sync_draft_workspace(workspace.clone());
        let created = planning
            .task_tool
            .run(
                &workspace,
                serde_json::from_str::<PlanningTaskToolRequest>(
                    r#"{"version":1,"op":"create_task","apply":true,"title":"Correlated removal","status":"ready"}"#,
                )
                .expect("create task request should parse"),
            )
            .expect("task should be created");
        let task_id = created.committed_task_ids[0].clone();
        let snapshot = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("queue snapshot should load");
        let task = snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .expect("created task should exist")
            .clone();
        let receipt = PlanningQueueMutationReceipt {
            completed_turn_id: "turn-correlated".to_string(),
            planning_revision: snapshot.planning_revision,
            entries: vec![PlanningQueueMutationReceiptEntry {
                task_id: task.id.clone(),
                task_title: task.title.clone(),
                mutation_kind: PlanningQueueMutationKind::Created,
                before_status: None,
                after_status: task.status,
                after_updated_at: task.updated_at.clone(),
                unchanged_since_mutation: true,
            }],
        };
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt = Some(receipt.clone());
        app.show_queue_overlay();
        apply_next_queue_overlay_authority_load(&mut app);
        assert_eq!(
            app.planning_runtime_projection_snapshot()
                .planning_revision(),
            Some(snapshot.planning_revision)
        );

        repository.arm();
        let terminal_thread_id = std::thread::current().id();
        let (cancel_watchdog, watchdog_cancelled) = mpsc::sync_channel(1);
        let watchdog_release = release_mutation.clone();
        let watchdog = std::thread::spawn(move || {
            if watchdog_cancelled
                .recv_timeout(Duration::from_millis(500))
                .is_err()
            {
                let _ = watchdog_release.send(());
            }
        });
        let input_started = Instant::now();
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        let input_elapsed = input_started.elapsed();
        cancel_watchdog
            .send(())
            .expect("fast input return should cancel the deadlock watchdog");
        watchdog.join().expect("deadlock watchdog should exit");
        assert!(
            input_elapsed < Duration::from_millis(400),
            "queue input blocked the terminal thread for {input_elapsed:?}"
        );
        mutation_started
            .recv_timeout(Duration::from_secs(2))
            .expect("background mutation should reach the gated commit");

        assert_eq!(app.pending_queue_mutation_operation_id(), Some(1));
        assert_ne!(repository.mutation_thread_id(), Some(terminal_thread_id));
        assert_eq!(repository.mutation_count(), 1);
        assert_eq!(
            ready_conversation(&app).latest_queue_mutation_receipt,
            Some(receipt)
        );
        assert_eq!(
            app.planning_runtime_projection_snapshot()
                .planning_revision(),
            Some(snapshot.planning_revision)
        );
        let while_pending = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("pending mutation should leave authority readable");
        assert_eq!(while_pending.planning_revision, snapshot.planning_revision);
        assert_eq!(
            while_pending
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .map(|task| task.status),
            Some(TaskStatus::Ready)
        );

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('u'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Down)));
        assert_eq!(repository.mutation_count(), 1);
        assert_eq!(app.pending_queue_mutation_operation_id(), Some(1));
        app.close_shell_overlay();
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(app.pending_queue_mutation_operation_id(), Some(1));
        assert_eq!(app.queue_receipt_undo_task_count(), None);
        app.show_queue_overlay();
        assert_eq!(app.pending_queue_mutation_operation_id(), Some(1));
        assert_eq!(
            app.planning_runtime_projection_snapshot()
                .planning_revision(),
            Some(snapshot.planning_revision)
        );
        app.close_shell_overlay();
        let mut newer_receipt = ready_conversation(&app)
            .latest_queue_mutation_receipt
            .clone()
            .expect("captured receipt should still be present while pending");
        newer_receipt.completed_turn_id = "turn-newer-receipt".to_string();
        newer_receipt.planning_revision = snapshot.planning_revision + 1;
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt =
            Some(newer_receipt.clone());

        release_mutation
            .send(())
            .expect("gated mutation should be released");
        let (correlation, completion) = take_next_queue_mutation_completion(&mut app);
        let after = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("cancelled queue snapshot should load");
        assert_eq!(app.pending_queue_mutation_operation_id(), Some(1));
        assert_eq!(
            app.planning_runtime_projection_snapshot()
                .planning_revision(),
            Some(snapshot.planning_revision)
        );
        assert_eq!(
            ready_conversation(&app).latest_queue_mutation_receipt,
            Some(newer_receipt.clone())
        );
        assert_eq!(
            after
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .map(|task| task.status),
            Some(TaskStatus::Cancelled)
        );
        app.tui_language = TuiLanguage::Korean;
        app.apply_queue_mutation_completion(correlation, completion);

        assert_eq!(app.pending_queue_mutation_operation_id(), None);
        assert_eq!(repository.mutation_count(), 1);
        assert!(app.core_runtime.poll_pending_input().is_none());
        assert_eq!(
            app.planning_runtime_projection_snapshot()
                .planning_revision(),
            Some(after.planning_revision)
        );
        let preserved_newer_receipt = ready_conversation(&app)
            .latest_queue_mutation_receipt
            .as_ref()
            .expect("newer receipt should be preserved and reconciled");
        assert_eq!(
            preserved_newer_receipt.completed_turn_id,
            "turn-newer-receipt"
        );
        assert_eq!(
            preserved_newer_receipt.planning_revision,
            after.planning_revision
        );
        assert!(!preserved_newer_receipt.created_batch_is_cancellable());
        let feedback = app
            .queue_overlay_ui_state
            .feedback()
            .expect("localized remove feedback should be visible");
        assert!(feedback.contains("선택한 큐 항목 제거"));
        assert_eq!(status_text(&app), feedback);
        assert_eq!(
            ready_conversation(&app)
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some(feedback)
        );
        std::fs::remove_dir_all(&workspace).expect("queue workspace should clean up");
    }

    #[test]
    fn queue_mutation_completion_does_not_cross_conversation_context() {
        let mut app = test_native_tui_app();
        let correlation = record_test_queue_mutation(
            &mut app,
            queue_overlay_ui::QueueMutationKind::RemoveSelected,
            3,
            Vec::new(),
            None,
        );
        let projection_before = app.planning_runtime_projection_snapshot();
        ready_conversation_mut(&mut app).thread_id = "replacement-thread".to_string();
        ready_conversation_mut(&mut app).status_text = "replacement context ready".to_string();

        app.apply_queue_mutation_completion(
            correlation,
            QueueMutationResult {
                mutation: Err("old context mutation failed".to_string()),
                authority: Err(QueueAuthorityLoadError::AuthorityUnavailable(
                    "old context refresh failed".to_string(),
                )),
            },
        );

        assert_eq!(app.pending_queue_mutation_operation_id(), None);
        assert_eq!(status_text(&app), "replacement context ready");
        assert_eq!(
            app.planning_runtime_projection_snapshot(),
            projection_before
        );
        assert_eq!(app.queue_overlay_ui_state.feedback(), None);
        assert!(app.queue_mutation_requires_authority_refresh());
    }

    #[test]
    fn queue_mutation_completion_never_rolls_back_a_newer_visible_revision() {
        let mut app = test_native_tui_app();
        let visible_projection =
            PlanningRuntimeProjection::uninitialized().with_planning_revision(Some(11));
        app.sync_ready_conversation_planning_runtime_projection(visible_projection.clone());
        let correlation = record_test_queue_mutation(
            &mut app,
            queue_overlay_ui::QueueMutationKind::RemoveSelected,
            9,
            Vec::new(),
            None,
        );
        app.tui_language = TuiLanguage::Korean;

        app.apply_queue_mutation_completion(
            correlation,
            QueueMutationResult {
                mutation: Err("stale worker result".to_string()),
                authority: Ok(QueueAuthoritySnapshot {
                    runtime_projection: PlanningRuntimeProjection::uninitialized()
                        .with_planning_revision(Some(10)),
                    planning_revision: 10,
                    tasks: Vec::new(),
                }),
            },
        );

        assert_eq!(
            app.planning_runtime_projection_snapshot(),
            visible_projection
        );
        assert!(app.queue_mutation_requires_authority_refresh());
        assert!(
            app.queue_overlay_ui_state.feedback().is_some_and(
                |feedback| feedback.contains("현재 표시된 계획 리비전보다 오래되었습니다")
            )
        );
    }

    #[test]
    fn queue_mutation_settlement_reconciles_a_receipt_created_while_pending() {
        let mut app = test_native_tui_app();
        let context = app.current_queue_mutation_context();
        let correlation = test_queue_mutation_correlation(
            1,
            context,
            queue_overlay_ui::QueueMutationKind::RemoveSelected,
            4,
            Vec::new(),
            None,
        );
        let receipt_created_while_pending = PlanningQueueMutationReceipt {
            completed_turn_id: "turn-created-while-pending".to_string(),
            planning_revision: 4,
            entries: vec![PlanningQueueMutationReceiptEntry {
                task_id: "task-created-while-pending".to_string(),
                task_title: "Created while pending".to_string(),
                mutation_kind: PlanningQueueMutationKind::Created,
                before_status: None,
                after_status: TaskStatus::Ready,
                after_updated_at: "2026-07-16T00:00:00Z".to_string(),
                unchanged_since_mutation: true,
            }],
        };
        let authority = crate::application::service::planning::PlanningQueueAuthoritySnapshot {
            planning_revision: 5,
            tasks: Vec::new(),
        };

        for settle in [true, false] {
            ready_conversation_mut(&mut app).latest_queue_mutation_receipt =
                Some(receipt_created_while_pending.clone());
            if settle {
                app.settle_correlated_queue_receipt(&correlation, &authority);
            } else {
                app.reconcile_correlated_queue_receipt(&correlation, &authority);
            }

            let reconciled = ready_conversation(&app)
                .latest_queue_mutation_receipt
                .as_ref()
                .expect("receipt created while pending should be preserved");
            assert_eq!(reconciled.completed_turn_id, "turn-created-while-pending");
            assert_eq!(reconciled.planning_revision, 5);
            assert!(!reconciled.created_batch_is_cancellable());
        }
    }

    #[test]
    fn queue_mutation_refresh_failure_preserves_projection_and_receipt_once() {
        let mut app = test_native_tui_app();
        let receipt = PlanningQueueMutationReceipt {
            completed_turn_id: "turn-refresh-failure".to_string(),
            planning_revision: 3,
            entries: vec![PlanningQueueMutationReceiptEntry {
                task_id: "task-refresh-failure".to_string(),
                task_title: "Retry after refresh".to_string(),
                mutation_kind: PlanningQueueMutationKind::Created,
                before_status: None,
                after_status: TaskStatus::Ready,
                after_updated_at: "2026-07-16T00:00:00Z".to_string(),
                unchanged_since_mutation: true,
            }],
        };
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt = Some(receipt.clone());
        let correlation = record_test_queue_mutation(
            &mut app,
            queue_overlay_ui::QueueMutationKind::UndoLatestRegistration,
            3,
            vec![QueueMutationTarget {
                task_id: "task-refresh-failure".to_string(),
                expected_status: TaskStatus::Ready,
                expected_updated_at: "2026-07-16T00:00:00Z".to_string(),
            }],
            Some(receipt.clone()),
        );
        let projection_before = app.planning_runtime_projection_snapshot();
        let completion = QueueMutationResult {
            mutation: Err("guard release failed".to_string()),
            authority: Err(QueueAuthorityLoadError::AuthorityUnavailable(
                "snapshot unavailable".to_string(),
            )),
        };

        app.apply_queue_mutation_completion(correlation.clone(), completion.clone());

        assert_eq!(app.pending_queue_mutation_operation_id(), None);
        assert_eq!(
            app.planning_runtime_projection_snapshot(),
            projection_before
        );
        assert_eq!(
            ready_conversation(&app).latest_queue_mutation_receipt,
            Some(receipt)
        );
        assert!(app.queue_mutation_requires_authority_refresh());
        assert_eq!(app.queue_receipt_undo_task_count(), None);
        assert!(!app.undo_latest_queue_registration());
        assert!(
            app.queue_overlay_ui_state
                .feedback()
                .is_some_and(|feedback| feedback.contains("close and reopen"))
        );
        assert!(status_text(&app).contains("op-1 is unresolved"));
        let message_count = ready_conversation(&app).messages.len();
        app.apply_queue_mutation_completion(correlation, completion);
        assert_eq!(ready_conversation(&app).messages.len(), message_count);
    }

    #[test]
    fn queue_mutation_refresh_failure_uses_the_selected_language_across_surfaces() {
        let mut app = test_native_tui_app();
        let correlation = record_test_queue_mutation(
            &mut app,
            queue_overlay_ui::QueueMutationKind::RemoveSelected,
            3,
            Vec::new(),
            None,
        );
        app.tui_language = TuiLanguage::Korean;

        app.apply_queue_mutation_completion(
            correlation,
            QueueMutationResult {
                mutation: Err("변이 실패".to_string()),
                authority: Err(QueueAuthorityLoadError::RuntimeProjectionUnavailable),
            },
        );

        let feedback = app
            .queue_overlay_ui_state
            .feedback()
            .expect("localized feedback should be visible");
        assert!(feedback.contains("큐 변경 op-1 미확정"));
        assert!(feedback.contains("큐 runtime projection을 사용할 수 없습니다"));
        assert_eq!(status_text(&app), feedback);
        assert_eq!(
            ready_conversation(&app)
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some(feedback)
        );
    }

    #[test]
    fn queue_mutation_input_feedback_uses_the_selected_language() {
        let mut app = test_native_tui_app();
        app.tui_language = TuiLanguage::Korean;
        let correlation = record_test_queue_mutation(
            &mut app,
            queue_overlay_ui::QueueMutationKind::RemoveSelected,
            3,
            Vec::new(),
            None,
        );

        assert!(!app.undo_latest_queue_registration());
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("큐 변경 op-1의 권한 확인을 기다리는 중입니다.")
        );

        assert_eq!(
            app.queue_mutation_ui_state.take_matching(&correlation),
            Some(correlation)
        );
        app.queue_mutation_ui_state.require_authority_refresh();
        assert!(!app.undo_latest_queue_registration());
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("큐 권한을 새로 확인해야 합니다. 큐를 닫았다가 다시 연 뒤 변경하세요.")
        );

        app.queue_mutation_ui_state.record_authority_refresh();
        app.cancel_selected_queue_task();
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("선택한 큐 항목이 변경되었습니다. 큐를 다시 열어 새로 확인하세요.")
        );
    }

    #[test]
    fn queue_overlay_mutations_gate_settlement_invalidate_stale_receipts_and_persist() {
        let (mut app, planning) = test_native_tui_app_with_services();
        let workspace = std::env::temp_dir()
            .join(format!(
                "akra-queue-overlay-undo-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should follow epoch")
                    .as_nanos()
            ))
            .to_string_lossy()
            .to_string();
        std::fs::create_dir_all(&workspace).expect("queue workspace should exist");
        planning
            .workspace
            .initialize_simple_workspace(&workspace)
            .expect("planning workspace should initialize");
        ready_conversation_mut(&mut app).sync_draft_workspace(workspace.clone());
        let created = planning
            .task_tool
            .run(
                &workspace,
                serde_json::from_str::<PlanningTaskToolRequest>(
                    r#"{"version":1,"op":"create_task","apply":true,"title":"Undo this registration","status":"ready"}"#,
                )
                .expect("create task request should parse"),
            )
            .expect("task should be created");
        let task_id = created.committed_task_ids[0].clone();
        let snapshot = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("queue snapshot should load");
        let task = snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .expect("created task should exist")
            .clone();
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt =
            Some(PlanningQueueMutationReceipt {
                completed_turn_id: "turn-queue".to_string(),
                planning_revision: snapshot.planning_revision,
                entries: vec![PlanningQueueMutationReceiptEntry {
                    task_id: task.id.clone(),
                    task_title: task.title.clone(),
                    mutation_kind: PlanningQueueMutationKind::Created,
                    before_status: None,
                    after_status: task.status,
                    after_updated_at: task.updated_at.clone(),
                    unchanged_since_mutation: true,
                }],
            });
        app.show_queue_overlay();
        apply_next_queue_overlay_authority_load(&mut app);

        ready_conversation_mut(&mut app).begin_post_turn_settlement("turn-queue");
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('u'))));
        let blocked = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("settlement gate should preserve queue authority");
        assert_eq!(blocked.planning_revision, snapshot.planning_revision);
        assert_eq!(blocked.tasks[0].status, TaskStatus::Ready);
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("wait for post-turn planning to finish")
        );
        assert!(
            ready_conversation(&app)
                .latest_queue_mutation_receipt
                .is_some()
        );
        ready_conversation_mut(&mut app)
            .auto_follow_state
            .clear_runtime_phase();
        assert!(ready_conversation(&app).has_post_turn_settlement_in_flight());
        assert!(ready_conversation_mut(&mut app).complete_post_turn_settlement("turn-queue"));

        app.close_shell_overlay();
        ready_conversation_mut(&mut app).record_turn_started("turn-active-undo".to_string());
        app.queue_overlay_ui_state
            .bind_receipt_undo_hit_area(Some(Rect::new(2, 4, 14, 1)));
        assert!(
            app.handle_queue_receipt_mouse_event(crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left,),
                column: 2,
                row: 4,
                modifiers: KeyModifiers::NONE,
            })
        );
        app.tui_language = TuiLanguage::Korean;
        apply_next_queue_mutation_completion(&mut app);

        let after = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("cancelled queue snapshot should load");
        assert_eq!(after.tasks[0].status, TaskStatus::Cancelled);
        assert!(
            app.queue_overlay_ui_state
                .feedback()
                .is_some_and(|feedback| feedback.contains("최근 큐 등록 되돌리기"))
        );
        assert!(ready_conversation(&app).messages.iter().any(|message| {
            message
                .text
                .contains("최근 큐 등록 되돌리기: 작업 1개를 취소로 변경 / 리비전")
        }));
        assert!(
            ready_conversation(&app)
                .latest_queue_mutation_receipt
                .is_none()
        );
        ready_conversation_mut(&mut app).mark_turn_finished();
        app.tui_language = TuiLanguage::English;

        let individually_removed = planning
            .task_tool
            .run(
                &workspace,
                serde_json::from_str::<PlanningTaskToolRequest>(
                    r#"{"version":1,"op":"create_task","apply":true,"title":"Individual removal","status":"ready"}"#,
                )
                .expect("individual task request should parse"),
            )
            .expect("individual task should create");
        let individual_task_id = individually_removed.committed_task_ids[0].clone();
        let individual_snapshot = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("individual task snapshot should load");
        let individual_task = individual_snapshot
            .tasks
            .iter()
            .find(|task| task.id == individual_task_id)
            .expect("individual task should exist")
            .clone();
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt =
            Some(PlanningQueueMutationReceipt {
                completed_turn_id: "turn-stale".to_string(),
                planning_revision: individual_snapshot.planning_revision,
                entries: vec![PlanningQueueMutationReceiptEntry {
                    task_id: individual_task.id.clone(),
                    task_title: individual_task.title.clone(),
                    mutation_kind: PlanningQueueMutationKind::Created,
                    before_status: None,
                    after_status: individual_task.status,
                    after_updated_at: individual_task.updated_at.clone(),
                    unchanged_since_mutation: true,
                }],
            });
        app.show_queue_overlay();
        apply_next_queue_overlay_authority_load(&mut app);
        planning
            .task_tool
            .run(
                &workspace,
                serde_json::from_str::<PlanningTaskToolRequest>(
                    r#"{"version":1,"op":"create_task","apply":true,"title":"Concurrent queue change","status":"ready"}"#,
                )
                .expect("concurrent task request should parse"),
            )
            .expect("concurrent task should create");

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        app.tui_language = TuiLanguage::Korean;
        apply_next_queue_mutation_completion(&mut app);
        assert!(
            app.queue_overlay_ui_state
                .feedback()
                .is_some_and(|feedback| feedback.contains("큐 변경 거부"))
        );
        assert!(status_text(&app).contains("op-2 거부"));
        app.tui_language = TuiLanguage::English;
        assert!(
            ready_conversation(&app)
                .latest_queue_mutation_receipt
                .is_some()
        );
        let after_stale_remove = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("stale remove should refresh authority");
        assert_eq!(
            after_stale_remove
                .tasks
                .iter()
                .find(|task| task.id == individual_task_id)
                .map(|task| task.status),
            Some(TaskStatus::Ready)
        );
        let reconciled_receipt = ready_conversation(&app)
            .latest_queue_mutation_receipt
            .as_ref()
            .expect("unchanged receipt targets should stay retryable after a stale rejection");
        assert_eq!(
            reconciled_receipt.planning_revision,
            after_stale_remove.planning_revision
        );
        assert!(reconciled_receipt.created_batch_is_cancellable());

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        apply_next_queue_mutation_completion(&mut app);
        let after_individual_remove = planning
            .queue
            .load_authority_snapshot(&workspace)
            .expect("individual cancellation should persist");
        assert_eq!(
            after_individual_remove
                .tasks
                .iter()
                .find(|task| task.id == individual_task_id)
                .map(|task| task.status),
            Some(TaskStatus::Cancelled)
        );
        ready_conversation_mut(&mut app).latest_queue_mutation_receipt =
            Some(PlanningQueueMutationReceipt {
                completed_turn_id: "turn-stale".to_string(),
                planning_revision: individual_snapshot.planning_revision,
                entries: vec![PlanningQueueMutationReceiptEntry {
                    task_id: individual_task.id,
                    task_title: individual_task.title,
                    mutation_kind: PlanningQueueMutationKind::Created,
                    before_status: None,
                    after_status: individual_task.status,
                    after_updated_at: individual_task.updated_at,
                    unchanged_since_mutation: true,
                }],
            });
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('u'))));
        apply_next_queue_mutation_completion(&mut app);
        assert!(
            app.queue_overlay_ui_state
                .feedback()
                .is_some_and(|feedback| feedback.contains("authority confirmed cancellation"))
        );
        assert!(
            ready_conversation(&app)
                .latest_queue_mutation_receipt
                .is_none()
        );
        std::fs::remove_dir_all(&workspace).expect("queue workspace should clean up");
    }

    #[test]
    fn exit_confirmation_and_shell_overlay_key_routes_are_scoped() {
        let mut app = test_native_tui_app();

        assert_eq!(
            app.handle_exit_confirmation_key(key(KeyCode::Char('y'))),
            None
        );

        app.exit_confirmation_state = ExitConfirmationState::Visible;
        assert_eq!(
            app.handle_exit_confirmation_key(modified_key(
                KeyCode::Char('y'),
                KeyModifiers::CONTROL
            )),
            None
        );
        assert_eq!(
            app.handle_exit_confirmation_key(modified_key(KeyCode::Char('Y'), KeyModifiers::SHIFT)),
            Some(true)
        );
        assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Hidden);
        app.exit_confirmation_state = ExitConfirmationState::Visible;
        assert_eq!(
            app.handle_exit_confirmation_key(key(KeyCode::Char('n'))),
            Some(false)
        );
        assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Hidden);

        app.exit_confirmation_state = ExitConfirmationState::Visible;
        assert_eq!(
            app.handle_exit_confirmation_key(key(KeyCode::Char('x'))),
            Some(false)
        );

        app.shell_overlay = ShellOverlay::Hidden;
        assert!(!app.handle_shell_overlay_key(key(KeyCode::Esc)));

        app.shell_overlay = ShellOverlay::Startup;
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('r'))));
        assert!(matches!(app.startup_state, StartupState::Loading));

        app.shell_overlay = ShellOverlay::Startup;
        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        assert!(
            app.handle_shell_overlay_key(modified_key(KeyCode::Char('o'), KeyModifiers::CONTROL))
        );
        assert_eq!(app.shell_overlay, ShellOverlay::Sessions);

        app.shell_overlay = ShellOverlay::Sessions;
        assert!(app.handle_shell_overlay_key(key(KeyCode::Esc)));
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);

        app.show_model_selection_overlay();
        assert!(app.handle_shell_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            app.model_selection_overlay_ui_state.step(),
            ModelSelectionStep::Effort
        );

        app.show_view_selection_overlay();
        assert!(app.handle_shell_overlay_key(key(KeyCode::Enter)));
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);

        app.show_language_selection_overlay();
        assert!(app.handle_shell_overlay_key(key(KeyCode::Enter)));
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);

        app.handle_ctrl_c();
        assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Visible);

        app.shell_overlay = ShellOverlay::Queue;
        app.handle_ctrl_c();
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    }

    #[test]
    fn approval_overlay_consumes_all_input_and_routes_only_explicit_decisions() {
        let mut app = test_native_tui_app();
        ready_conversation_mut(&mut app).input_buffer = "draft prompt".to_string();
        let turn_submission = arm_pending_approval(
            &mut app,
            ConversationApprovalRequest {
                approval_id: "approval-key".to_string(),
                server_request_id: "server-key".to_string(),
                method: "item/commandExecution/requestApproval".to_string(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: "Command execution requested.".to_string(),
                details: (1..=8).map(|index| format!("Detail {index}")).collect(),
            },
        );

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        assert_eq!(ready_conversation(&app).input_buffer, "draft prompt");
        assert_eq!(app.shell_overlay, ShellOverlay::Approval);

        assert!(app.handle_shell_overlay_key(key(KeyCode::Down)));
        assert_eq!(ready_conversation(&app).approval_detail_scroll_offset, 1);
        assert_eq!(ready_conversation(&app).input_buffer, "draft prompt");

        assert!(app.handle_shell_overlay_key(key(KeyCode::Enter)));
        assert_eq!(ready_conversation(&app).pending_approval_decision(), None);
        assert!(ready_conversation(&app).pending_approval_request.is_some());
        assert_eq!(app.shell_overlay, ShellOverlay::Approval);

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('y'))));
        assert_eq!(
            ready_conversation(&app).status_text,
            "approval decision submitted: accept / waiting for runtime resolution"
        );
        assert!(ready_conversation(&app).pending_approval_request.is_some());
        assert_eq!(
            ready_conversation(&app).pending_approval_decision(),
            Some(crate::domain::conversation::ConversationApprovalDecision::Accept)
        );
        assert_eq!(app.shell_overlay, ShellOverlay::Approval);

        app.apply_core_event(AppEvent::ApprovalDecisionSubmissionCompleted {
            correlation: crate::core::app::ApprovalDecisionCorrelation::new(
                1,
                turn_submission,
                "approval-key",
                crate::domain::conversation::ConversationApprovalDecision::Accept,
            ),
            result: Ok(()),
        });
        assert_eq!(
            ready_conversation(&app).pending_approval_decision(),
            Some(crate::domain::conversation::ConversationApprovalDecision::Accept)
        );

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('n'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Esc)));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('y'))));
        assert_eq!(
            ready_conversation(&app).pending_approval_decision(),
            Some(crate::domain::conversation::ConversationApprovalDecision::Accept)
        );
        assert_eq!(
            ready_conversation(&app).status_text,
            "approval decision submitted: accept / waiting for runtime resolution"
        );
        assert_eq!(app.shell_overlay, ShellOverlay::Approval);

        app.close_shell_overlay();
        assert_eq!(app.shell_overlay, ShellOverlay::Approval);

        assert!(
            app.handle_shell_overlay_key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL,))
        );
        assert!(status_text(&app).contains("stop requested"));
        assert_eq!(
            ready_conversation(&app).pending_approval_decision(),
            Some(crate::domain::conversation::ConversationApprovalDecision::Accept)
        );
        assert_eq!(ready_conversation(&app).input_buffer, "draft prompt");
    }

    #[test]
    fn unavailable_approval_decision_does_not_commit_pending_projection() {
        let mut app = test_native_tui_app();
        ready_conversation_mut(&mut app).pending_approval_request =
            Some(ConversationApprovalRequest {
                approval_id: "approval-unavailable".to_string(),
                server_request_id: "server-unavailable".to_string(),
                method: "item/commandExecution/requestApproval".to_string(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: "Command execution requested.".to_string(),
                details: vec!["Command: cargo test".to_string()],
            });
        app.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('y'))));
        assert_eq!(ready_conversation(&app).pending_approval_decision(), None);
        assert!(ready_conversation(&app).pending_approval_request.is_some());
        assert_eq!(app.shell_overlay, ShellOverlay::Approval);
    }

    #[test]
    fn failed_approval_decision_completion_reopens_retry_before_provider_resolution() {
        let mut app = test_native_tui_app_with_approval_resolution_error("runtime unavailable");
        let _ = arm_pending_approval(
            &mut app,
            ConversationApprovalRequest {
                approval_id: "approval-failure".to_string(),
                server_request_id: "server-failure".to_string(),
                method: "item/commandExecution/requestApproval".to_string(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: "Command execution requested.".to_string(),
                details: vec!["Command: cargo test".to_string()],
            },
        );

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('y'))));
        assert_eq!(
            ready_conversation(&app).pending_approval_decision(),
            Some(crate::domain::conversation::ConversationApprovalDecision::Accept)
        );

        let deadline = Instant::now() + Duration::from_secs(1);
        while ready_conversation(&app)
            .pending_approval_decision()
            .is_some()
            && Instant::now() < deadline
        {
            app.poll_core_runtime_inputs(8);
            std::thread::yield_now();
        }

        assert_eq!(ready_conversation(&app).pending_approval_decision(), None);
        assert_eq!(
            ready_conversation(&app).status_text,
            "approval decision failed: runtime unavailable / retry accept or decline"
        );
        assert_eq!(app.shell_overlay, ShellOverlay::Approval);
    }

    #[test]
    fn selection_overlay_keymaps_cover_navigation_numbers_enter_and_back() {
        let mut app = test_native_tui_app();

        assert!(!app.handle_model_selection_overlay_key(key(KeyCode::Enter)));
        app.show_model_selection_overlay();
        assert!(app.handle_model_selection_overlay_key(key(KeyCode::Down)));
        assert!(app.handle_model_selection_overlay_key(key(KeyCode::Up)));
        assert!(app.handle_model_selection_overlay_key(key(KeyCode::Char('2'))));
        assert_eq!(
            app.model_selection_overlay_ui_state.step(),
            ModelSelectionStep::Effort
        );
        assert!(app.handle_model_selection_overlay_key(key(KeyCode::Backspace)));
        assert_eq!(
            app.model_selection_overlay_ui_state.step(),
            ModelSelectionStep::Model
        );
        assert!(app.handle_model_selection_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            app.model_selection_overlay_ui_state.step(),
            ModelSelectionStep::Effort
        );
        assert!(app.handle_model_selection_overlay_key(key(KeyCode::Enter)));
        assert_eq!(app.turn_options.model.as_deref(), Some("gpt-5.4"));
        assert_eq!(
            app.turn_options.reasoning_effort,
            Some(ConversationReasoningEffort::High)
        );

        assert!(!app.handle_view_selection_overlay_key(key(KeyCode::Enter)));
        app.show_view_selection_overlay();
        assert!(app.handle_view_selection_overlay_key(key(KeyCode::Down)));
        assert!(app.handle_view_selection_overlay_key(key(KeyCode::Up)));
        assert!(app.handle_view_selection_overlay_key(key(KeyCode::Char('3'))));
        assert_eq!(app.conversation_view_mode, ConversationViewMode::Detail);
        app.show_view_selection_overlay();
        assert!(app.handle_view_selection_overlay_key(key(KeyCode::Enter)));

        assert!(!app.handle_language_selection_overlay_key(key(KeyCode::Enter)));
        app.show_language_selection_overlay();
        assert!(app.handle_language_selection_overlay_key(key(KeyCode::Down)));
        assert!(app.handle_language_selection_overlay_key(key(KeyCode::Up)));
        assert!(app.handle_language_selection_overlay_key(key(KeyCode::Char('1'))));
        assert_eq!(app.tui_language, TuiLanguage::English);
        app.show_language_selection_overlay();
        assert!(app.handle_language_selection_overlay_key(key(KeyCode::Enter)));
        assert_eq!(app.tui_language, TuiLanguage::English);
    }

    #[test]
    fn steer_completion_clears_only_the_exact_confirmed_draft() {
        let mut app = test_native_tui_app();
        let request = ConversationTurnSteerRequest {
            thread_id: "thread-steer".to_string(),
            expected_turn_id: "turn-steer".to_string(),
            prompt: "focus the current work".to_string(),
        };
        {
            let conversation = ready_conversation_mut(&mut app);
            conversation.thread_id = request.thread_id.clone();
            conversation.record_turn_started(request.expected_turn_id.clone());
            conversation.input_buffer = request.prompt.clone();
        }
        app.pending_turn_steer = Some(steer_intent(
            1,
            app.prompt_input_revision,
            "focus the current work",
            request.clone(),
        ));

        app.apply_turn_steer_completion(steer_correlation(1), Err("not steerable".to_string()));
        assert_eq!(
            ready_conversation(&app).input_buffer,
            "focus the current work"
        );
        assert!(app.pending_turn_steer.is_none());

        app.pending_turn_steer = Some(steer_intent(
            2,
            app.prompt_input_revision,
            "focus the current work",
            request,
        ));
        app.apply_turn_steer_completion(
            steer_correlation(2),
            Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                turn_id: "turn-steer".to_string(),
            }),
        );
        assert!(ready_conversation(&app).input_buffer.is_empty());
        assert!(app.pending_turn_steer.is_none());
    }

    #[test]
    fn confirmed_steer_runs_through_core_and_preserves_the_draft_on_worker_failure() {
        let mut app = test_native_tui_app();
        let turn_submission = app.core_runtime.begin_test_turn_submission();
        app.dispatch_core_input(crate::core::app::CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: crate::core::app::TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-steer".to_string(),
                title: "Steer test".to_string(),
                cwd: "/tmp/root".to_string(),
                runtime_envelope: Box::default(),
            },
        });
        app.dispatch_core_input(crate::core::app::CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: crate::core::app::TurnStreamEvent::TurnStarted {
                turn_id: "turn-steer".to_string(),
                runtime_request: Box::default(),
            },
        });
        ready_conversation_mut(&mut app).input_buffer = "keep this draft".to_string();

        assert!(app.show_turn_steer_confirmation());
        assert!(app.handle_turn_steer_confirmation_key(key(KeyCode::Enter)));
        assert!(app.pending_turn_steer.is_some());

        let deadline = Instant::now() + Duration::from_secs(1);
        while app.pending_turn_steer.is_some() && Instant::now() < deadline {
            app.poll_core_runtime_inputs(8);
            std::thread::yield_now();
        }

        assert!(app.pending_turn_steer.is_none());
        assert_eq!(ready_conversation(&app).input_buffer, "keep this draft");
        assert!(
            ready_conversation(&app)
                .status_text
                .contains("steer rejected")
        );
    }

    #[test]
    fn accepted_steer_does_not_clear_a_draft_edited_after_confirmation() {
        let mut app = test_native_tui_app();
        let request = ConversationTurnSteerRequest {
            thread_id: "thread-steer".to_string(),
            expected_turn_id: "turn-steer".to_string(),
            prompt: "original draft".to_string(),
        };
        {
            let conversation = ready_conversation_mut(&mut app);
            conversation.thread_id = request.thread_id.clone();
            conversation.record_turn_started(request.expected_turn_id.clone());
            conversation.input_buffer = "newer draft".to_string();
        }
        app.pending_turn_steer = Some(steer_intent(
            1,
            app.prompt_input_revision,
            "original draft",
            request,
        ));

        app.apply_turn_steer_completion(
            steer_correlation(1),
            Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                turn_id: "turn-steer".to_string(),
            }),
        );

        assert_eq!(ready_conversation(&app).input_buffer, "newer draft");
    }

    #[test]
    fn accepted_steer_does_not_clear_the_same_draft_retyped_on_a_new_turn() {
        let mut app = test_native_tui_app();
        let request = ConversationTurnSteerRequest {
            thread_id: "thread-steer".to_string(),
            expected_turn_id: "turn-old".to_string(),
            prompt: "same draft".to_string(),
        };
        {
            let conversation = ready_conversation_mut(&mut app);
            conversation.thread_id = request.thread_id.clone();
            conversation.record_turn_started("turn-new".to_string());
            conversation.input_buffer = request.prompt.clone();
        }
        app.pending_turn_steer = Some(steer_intent(1, 0, "same draft", request));
        app.prompt_input_revision = 1;

        app.apply_turn_steer_completion(
            steer_correlation(1),
            Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                turn_id: "turn-old".to_string(),
            }),
        );

        assert_eq!(ready_conversation(&app).input_buffer, "same draft");
    }

    #[test]
    fn accepted_steer_clears_an_unchanged_draft_after_the_turn_finishes() {
        let mut app = test_native_tui_app();
        let request = ConversationTurnSteerRequest {
            thread_id: "thread-steer".to_string(),
            expected_turn_id: "turn-steer".to_string(),
            prompt: "delivered draft".to_string(),
        };
        {
            let conversation = ready_conversation_mut(&mut app);
            conversation.thread_id = request.thread_id.clone();
            conversation.record_turn_started(request.expected_turn_id.clone());
            conversation.input_buffer = "delivered draft".to_string();
            conversation.mark_turn_finished();
        }
        app.pending_turn_steer = Some(steer_intent(1, 0, "delivered draft", request));

        app.apply_turn_steer_completion(
            steer_correlation(1),
            Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                turn_id: "turn-steer".to_string(),
            }),
        );

        assert!(ready_conversation(&app).input_buffer.is_empty());
    }

    #[test]
    fn stale_steer_completion_cannot_complete_a_new_identical_request() {
        let mut app = test_native_tui_app();
        let request = ConversationTurnSteerRequest {
            thread_id: "thread-steer".to_string(),
            expected_turn_id: "turn-steer".to_string(),
            prompt: "same request".to_string(),
        };
        {
            let conversation = ready_conversation_mut(&mut app);
            conversation.thread_id = request.thread_id.clone();
            conversation.record_turn_started(request.expected_turn_id.clone());
            conversation.input_buffer = "same request".to_string();
        }
        app.pending_turn_steer = Some(steer_intent(2, 0, "same request", request));

        app.apply_turn_steer_completion(
            steer_correlation(1),
            Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                turn_id: "turn-steer".to_string(),
            }),
        );

        assert_eq!(
            app.pending_turn_steer
                .as_ref()
                .map(|pending| pending.correlation.generation),
            Some(2)
        );
        assert_eq!(ready_conversation(&app).input_buffer, "same request");
    }
}
