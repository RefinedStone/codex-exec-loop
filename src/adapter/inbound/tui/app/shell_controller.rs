use super::*;
use crate::application::service::planning::{
    PlanningQueueCancellationRequest, PlanningQueueCancellationTarget,
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
    fn mark_active_turn_interrupt_requested_once(&mut self) -> bool {
        match &mut self.conversation_state {
            ConversationState::Ready(conversation) => conversation.mark_interrupt_requested_once(),
            ConversationState::Loading | ConversationState::Failed(_) => false,
        }
    }
    pub(super) fn clear_active_turn_interrupt_request(&mut self) {
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.clear_interrupt_request();
        }
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
    pub(super) fn show_queue_overlay(&mut self) {
        if let Err(error) = self.refresh_queue_overlay_authority_binding() {
            self.queue_overlay_ui_state.set_feedback(error);
        }
        self.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);
    }
    pub(super) fn show_reviews_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::ReviewsOverlayShown);
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
            ShellOverlay::Queue => {
                self.queue_overlay_ui_state.reset();
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
        let selected_kind = match argument {
            None => ProgressiveActivityDetailKind::Diff,
            Some(argument) => {
                let Some(selected_kind) = parse_progressive_activity_detail_kind(argument) else {
                    self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: "activity unchanged; supported values: diff, output"
                            .to_string(),
                    });
                    return;
                };
                selected_kind
            }
        };
        self.show_progressive_activity_overlay(selected_kind);
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
        // disable future automation immediately, then ask the service to
        // interrupt any running native sessions.
        let has_running_turn = self.conversation_has_running_turn();
        if has_running_turn && !self.mark_active_turn_interrupt_requested_once() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "stop already requested / waiting for the active app-server turn / auto-follow remains disarmed until :turns re-enables it"
                    .to_string(),
            });
            return;
        }
        let status_text = match self.application.request_stop_all_sessions() {
            Ok(()) if has_running_turn => {
                "stop requested / active app-server sessions will be interrupted / auto-follow disarmed until :turns re-enables it".to_string()
            }
            Ok(()) => "stop requested / no active turn is running / auto-follow disarmed until :turns re-enables it".to_string(),
            Err(error) => {
                self.clear_active_turn_interrupt_request();
                format!(
                    "stop request failed: {error} / auto-follow remains disarmed until :turns re-enables it"
                )
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
        match self.shell_overlay {
            ShellOverlay::Hidden => true,
            ShellOverlay::Supersession => !self.parallel_mode_prompt_input_locked(),
            _ => false,
        }
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
        self.next_turn_steer_request_id = self.next_turn_steer_request_id.wrapping_add(1).max(1);
        self.turn_steer_confirmation = Some(TurnSteerUiIntent {
            request_id: self.next_turn_steer_request_id,
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

        let request_id = intent.request_id;
        let request = intent.request.clone();
        self.pending_turn_steer = Some(intent);
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: self.tui_language.turn_steer_pending_status().to_string(),
        });
        let application = self.application.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = application.steer_turn(request);
            let _ = tx.send(BackgroundMessage::TurnSteerCompleted { request_id, result });
        });
    }

    pub(super) fn apply_turn_steer_completion(
        &mut self,
        request_id: u64,
        result: Result<crate::domain::conversation::ConversationTurnSteerReceipt, String>,
    ) {
        let Some(intent) = self
            .pending_turn_steer
            .as_ref()
            .filter(|intent| intent.request_id == request_id)
            .cloned()
        else {
            return;
        };
        self.pending_turn_steer = None;
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

    fn handle_queue_overlay_key(&mut self, key: event::KeyEvent) -> bool {
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

    fn cancel_selected_queue_task(&mut self) {
        if let Some(reason) = self.queue_mutation_block_reason() {
            self.queue_overlay_ui_state.set_feedback(reason);
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
            let _ = self.refresh_queue_overlay_authority_binding();
            self.queue_overlay_ui_state
                .set_feedback("The selected queue item changed; review the refreshed queue.");
            return;
        };
        self.apply_queue_cancellation(
            PlanningQueueCancellationRequest {
                workspace_directory: self.planning_workspace_directory(),
                expected_planning_revision: planning_revision,
                targets: vec![PlanningQueueCancellationTarget {
                    task_id,
                    expected_status: status,
                    expected_updated_at: updated_at,
                }],
            },
            "Removed selected queue item",
        );
    }

    pub(super) fn undo_latest_queue_registration(&mut self) -> bool {
        if let Some(reason) = self.queue_receipt_undo_block_reason() {
            self.queue_overlay_ui_state.set_feedback(reason);
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
        if self.queue_overlay_ui_state.authority_revision() != Some(receipt.planning_revision) {
            if let Err(feedback) = self.refresh_queue_overlay_authority_binding() {
                self.queue_overlay_ui_state.set_feedback(feedback);
                return false;
            }
            if self.queue_overlay_ui_state.authority_revision() != Some(receipt.planning_revision) {
                self.queue_overlay_ui_state.set_feedback(
                    "The latest registration changed; review the refreshed queue before undoing it.",
                );
                return false;
            }
        }
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
            .map(|entry| PlanningQueueCancellationTarget {
                task_id: entry.task_id.clone(),
                expected_status: entry.after_status,
                expected_updated_at: entry.after_updated_at.clone(),
            })
            .collect::<Vec<_>>();
        self.apply_queue_cancellation(
            PlanningQueueCancellationRequest {
                workspace_directory: self.planning_workspace_directory(),
                expected_planning_revision: receipt.planning_revision,
                targets,
            },
            "Undid latest queue registration",
        )
    }

    fn apply_queue_cancellation(
        &mut self,
        request: PlanningQueueCancellationRequest,
        success_label: &str,
    ) -> bool {
        let queue = self.application.planning().queue().clone();
        match queue.cancel_tasks(request) {
            Ok(result) => {
                let success_message = format!(
                    "{success_label}: {} task(s) marked Cancelled / revision {}",
                    result.committed_task_ids.len(),
                    result.committed_planning_revision
                );
                if let ConversationState::Ready(conversation) = &mut self.conversation_state {
                    conversation.latest_queue_mutation_receipt = None;
                    conversation.status_text = success_message.clone();
                    conversation.append_status_message(success_message.clone());
                }
                let _ = self.refresh_queue_overlay_authority_binding();
                self.queue_overlay_ui_state.set_feedback(success_message);
                true
            }
            Err(error) => {
                let _ = self.refresh_queue_overlay_authority_binding();
                self.queue_overlay_ui_state
                    .set_feedback(format!("Queue change rejected: {error}"));
                false
            }
        }
    }

    fn invalidate_stale_queue_receipt(&mut self) {
        let current_revision = self
            .planning_runtime_projection_snapshot()
            .planning_revision();
        let ConversationState::Ready(conversation) = &mut self.conversation_state else {
            return;
        };
        if current_revision.is_some_and(|current_revision| {
            conversation
                .latest_queue_mutation_receipt
                .as_ref()
                .is_some_and(|receipt| receipt.planning_revision != current_revision)
        }) {
            conversation.latest_queue_mutation_receipt = None;
        }
    }

    pub(super) fn refresh_queue_overlay_authority_binding(&mut self) -> Result<(), String> {
        self.queue_overlay_ui_state.clear_authority_binding();
        for _ in 0..2 {
            self.refresh_ready_conversation_planning_runtime_projection();
            self.invalidate_stale_queue_receipt();
            let projection_revision = self
                .planning_runtime_projection_snapshot()
                .planning_revision()
                .ok_or_else(|| {
                    "Queue authority is unavailable; retry after planning reloads.".to_string()
                })?;
            let action_tasks = self.queue_action_tasks();
            let authority = self
                .application
                .planning()
                .queue()
                .load_authority_snapshot(&self.planning_workspace_directory())
                .map_err(|error| format!("Queue authority is unavailable: {error}"))?;
            if authority.planning_revision != projection_revision {
                continue;
            }
            let tokens = action_tasks
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
                                queue_overlay_ui::QueueOverlayAuthorityToken {
                                    status: authority_task.status,
                                    updated_at: authority_task.updated_at.clone(),
                                },
                            )
                        })
                })
                .collect::<Option<std::collections::BTreeMap<_, _>>>();
            let Some(tokens) = tokens else {
                continue;
            };
            self.queue_overlay_ui_state
                .bind_authority(authority.planning_revision, tokens);
            self.sync_queue_overlay_selection();
            return Ok(());
        }
        self.queue_overlay_ui_state.clear_authority_binding();
        self.sync_queue_overlay_selection();
        Err("Queue is still changing; reopen it to refresh before removing items.".to_string())
    }
    fn handle_progressive_activity_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match (key.code, key.modifiers) {
            (
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right,
                KeyModifiers::NONE | KeyModifiers::SHIFT,
            ) => self.progressive_activity_overlay_ui_state.cycle_kind(),
            (KeyCode::Up | KeyCode::PageUp, KeyModifiers::NONE) => {
                self.progressive_activity_overlay_ui_state
                    .move_to_previous_page();
            }
            (KeyCode::Down | KeyCode::PageDown, KeyModifiers::NONE) => {
                self.progressive_activity_overlay_ui_state
                    .move_to_next_page();
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
            self.dispatch_conversation_runtime(
                ConversationRuntimeEvent::ApprovalDecisionSubmitted {
                    approval_id,
                    decision,
                },
            );
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
    use crate::adapter::inbound::tui::app::test_helpers::{sample_queue_head, test_native_tui_app};
    use crate::application::service::planning::{
        PlanningRuntimeProjection, PlanningTaskToolRequest,
    };
    use crate::core::app::StartupReadySnapshot;
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
    };
    use crate::domain::planning::{
        PlanningQueueMutationKind, PlanningQueueMutationReceipt, PlanningQueueMutationReceiptEntry,
        PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus,
    };
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    fn key(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> event::KeyEvent {
        event::KeyEvent::new(code, modifiers)
    }

    fn steer_intent(
        request_id: u64,
        input_revision: u64,
        source_input_buffer: &str,
        request: ConversationTurnSteerRequest,
    ) -> TurnSteerUiIntent {
        TurnSteerUiIntent {
            request_id,
            input_revision,
            source_input_buffer: source_input_buffer.to_string(),
            request,
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

    fn status_text(app: &NativeTuiApp) -> &str {
        &ready_conversation(app).status_text
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
    fn activity_commands_default_validate_alias_and_preserve_approval_focus() {
        let mut app = test_native_tui_app();

        app.execute_inline_shell_command_input(command(":activity"));
        assert_eq!(app.shell_overlay, ShellOverlay::Activity);
        assert_eq!(
            app.progressive_activity_overlay_ui_state.selected_kind(),
            ProgressiveActivityDetailKind::Diff
        );

        app.close_shell_overlay();
        app.execute_inline_shell_command_input(command(":act output"));
        assert_eq!(app.shell_overlay, ShellOverlay::Activity);
        assert_eq!(
            app.progressive_activity_overlay_ui_state.selected_kind(),
            ProgressiveActivityDetailKind::Output
        );

        app.close_shell_overlay();
        app.execute_inline_shell_command_input(command(":activity all"));
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            status_text(&app),
            "activity unchanged; supported values: diff, output"
        );

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

        assert!(app.handle_shell_overlay_key(key(KeyCode::Down)));
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
        assert!(app.handle_shell_overlay_key(key(KeyCode::Up)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .current_page_start(),
            0
        );

        app.progressive_activity_overlay_ui_state
            .set_page_window(0, Some(10));
        assert!(app.handle_shell_overlay_key(key(KeyCode::PageDown)));
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
        assert!(app.handle_shell_overlay_key(key(KeyCode::Tab)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.selected_kind(),
            ProgressiveActivityDetailKind::Diff
        );
        assert_eq!(
            app.progressive_activity_overlay_ui_state
                .current_document_sequence(),
            None
        );
        assert!(app.handle_shell_overlay_key(modified_key(KeyCode::BackTab, KeyModifiers::SHIFT)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.selected_kind(),
            ProgressiveActivityDetailKind::Output
        );
        assert!(app.handle_shell_overlay_key(key(KeyCode::Left)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.selected_kind(),
            ProgressiveActivityDetailKind::Diff
        );
        assert!(app.handle_shell_overlay_key(key(KeyCode::Right)));
        assert_eq!(
            app.progressive_activity_overlay_ui_state.selected_kind(),
            ProgressiveActivityDetailKind::Output
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

        ready_conversation_mut(&mut app).record_turn_started("turn-1".to_string());
        app.execute_inline_shell_command_input(command(":stop"));
        assert!(status_text(&app).contains("active app-server sessions"));
        assert!(ready_conversation(&app).interrupt_request_pending);
        app.execute_inline_shell_command_input(command(":stop"));
        assert!(status_text(&app).contains("stop already requested"));
    }

    #[test]
    fn ctrl_c_interrupts_a_running_turn_once_and_keeps_idle_navigation_semantics() {
        let mut app = test_native_tui_app();
        ready_conversation_mut(&mut app).mark_turn_submitting("/tmp/root".to_string());

        app.handle_ctrl_c();
        assert!(ready_conversation(&app).interrupt_request_pending);
        ready_conversation_mut(&mut app).record_turn_started("turn-ctrl-c".to_string());
        assert!(ready_conversation(&app).interrupt_request_pending);
        assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Hidden);

        app.handle_ctrl_c();
        assert!(status_text(&app).contains("stop already requested"));
        assert!(ready_conversation(&app).interrupt_request_pending);

        ready_conversation_mut(&mut app).mark_turn_finished();
        assert!(!ready_conversation(&app).interrupt_request_pending);
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
        app.shell_overlay = ShellOverlay::Queue;

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('j'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('u'))));

        assert_eq!(ready_conversation(&app).input_buffer, "keep draft");
        assert_eq!(
            app.queue_overlay_ui_state.feedback(),
            Some("No recent queue registration is available to undo.")
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
        app.queue_overlay_ui_state
            .bind_authority(17, std::collections::BTreeMap::new());
        app.shell_overlay = ShellOverlay::Queue;

        let view =
            crate::adapter::inbound::tui::app::shell_presentation::build_queue_overlay_view(&app);
        assert!(
            view.key_lines
                .iter()
                .any(|line| line.to_string().contains("u:"))
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

        app.invalidate_stale_queue_receipt();

        assert!(
            ready_conversation(&app)
                .latest_queue_mutation_receipt
                .is_some()
        );
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
    fn queue_overlay_mutations_gate_settlement_invalidate_stale_receipts_and_persist() {
        let mut app = test_native_tui_app();
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
        app.application
            .planning()
            .workspace()
            .initialize_simple_workspace(&workspace)
            .expect("planning workspace should initialize");
        ready_conversation_mut(&mut app).sync_draft_workspace(workspace.clone());
        let created = app
            .application
            .planning()
            .task_tool()
            .run(
                &workspace,
                serde_json::from_str::<PlanningTaskToolRequest>(
                    r#"{"version":1,"op":"create_task","apply":true,"title":"Undo this registration","status":"ready"}"#,
                )
                .expect("create task request should parse"),
            )
            .expect("task should be created");
        let task_id = created.committed_task_ids[0].clone();
        let snapshot = app
            .application
            .planning()
            .queue()
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

        ready_conversation_mut(&mut app).begin_post_turn_settlement("turn-queue");
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('u'))));
        let blocked = app
            .application
            .planning()
            .queue()
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

        let after = app
            .application
            .planning()
            .queue()
            .load_authority_snapshot(&workspace)
            .expect("cancelled queue snapshot should load");
        assert_eq!(after.tasks[0].status, TaskStatus::Cancelled);
        assert!(
            app.queue_overlay_ui_state
                .feedback()
                .is_some_and(|feedback| feedback.contains("Undid latest queue registration"))
        );
        assert!(ready_conversation(&app).messages.iter().any(|message| {
            message
                .text
                .contains("Undid latest queue registration: 1 task(s) marked Cancelled / revision")
        }));
        assert!(
            ready_conversation(&app)
                .latest_queue_mutation_receipt
                .is_none()
        );
        ready_conversation_mut(&mut app).mark_turn_finished();

        let individually_removed = app
            .application
            .planning()
            .task_tool()
            .run(
                &workspace,
                serde_json::from_str::<PlanningTaskToolRequest>(
                    r#"{"version":1,"op":"create_task","apply":true,"title":"Individual removal","status":"ready"}"#,
                )
                .expect("individual task request should parse"),
            )
            .expect("individual task should create");
        let individual_task_id = individually_removed.committed_task_ids[0].clone();
        let individual_snapshot = app
            .application
            .planning()
            .queue()
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
        app.application
            .planning()
            .task_tool()
            .run(
                &workspace,
                serde_json::from_str::<PlanningTaskToolRequest>(
                    r#"{"version":1,"op":"create_task","apply":true,"title":"Concurrent queue change","status":"ready"}"#,
                )
                .expect("concurrent task request should parse"),
            )
            .expect("concurrent task should create");

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        assert!(
            app.queue_overlay_ui_state
                .feedback()
                .is_some_and(|feedback| feedback.contains("Queue change rejected"))
        );
        assert!(
            ready_conversation(&app)
                .latest_queue_mutation_receipt
                .is_none()
        );
        let after_stale_remove = app
            .application
            .planning()
            .queue()
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

        assert!(app.handle_shell_overlay_key(key(KeyCode::Char('x'))));
        let after_individual_remove = app
            .application
            .planning()
            .queue()
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
        assert!(
            app.queue_overlay_ui_state
                .feedback()
                .is_some_and(|feedback| feedback.contains("latest registration changed"))
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
        let conversation = ready_conversation_mut(&mut app);
        conversation.input_buffer = "draft prompt".to_string();
        conversation.mark_turn_submitting("/tmp/root".to_string());
        conversation.pending_approval_request = Some(ConversationApprovalRequest {
            approval_id: "approval-key".to_string(),
            server_request_id: "server-key".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "Command execution requested.".to_string(),
            details: (1..=8).map(|index| format!("Detail {index}")).collect(),
        });
        app.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);

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
        assert!(ready_conversation(&app).interrupt_request_pending);
        assert_eq!(
            ready_conversation(&app).pending_approval_decision(),
            Some(crate::domain::conversation::ConversationApprovalDecision::Accept)
        );
        assert_eq!(ready_conversation(&app).input_buffer, "draft prompt");
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

        app.apply_turn_steer_completion(1, Err("not steerable".to_string()));
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
            2,
            Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                turn_id: "turn-steer".to_string(),
            }),
        );
        assert!(ready_conversation(&app).input_buffer.is_empty());
        assert!(app.pending_turn_steer.is_none());
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
            1,
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
            1,
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
            1,
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
            1,
            Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                turn_id: "turn-steer".to_string(),
            }),
        );

        assert_eq!(
            app.pending_turn_steer
                .as_ref()
                .map(|intent| intent.request_id),
            Some(2)
        );
        assert_eq!(ready_conversation(&app).input_buffer, "same request");
    }
}
