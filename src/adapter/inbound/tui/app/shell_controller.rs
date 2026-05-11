use super::*;

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
    pub(super) fn show_queue_overlay(&mut self) {
        self.refresh_ready_conversation_planning_runtime_projection();
        self.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);
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
            InlineShellCommand::Sessions => self.show_session_overlay(),
            InlineShellCommand::Queue => self.handle_queue_shell_command(command_input.argument()),
            InlineShellCommand::Directions => {
                self.handle_directions_shell_command(command_input.argument())
            }
            InlineShellCommand::Turns => self.handle_turns_shell_command(command_input.argument()),
            InlineShellCommand::Stop => self.handle_stop_shell_command(),
            InlineShellCommand::Doctor => self.run_planning_doctor(),
            InlineShellCommand::PlanningInit => {
                self.handle_planning_shell_command(command_input.argument())
            }
            InlineShellCommand::Reset => self.handle_reset_shell_command(command_input.argument()),
            InlineShellCommand::NewDraft => self.open_new_conversation_shell(),
            InlineShellCommand::Help => self.show_help_overlay(),
        }
        let status_text = match command_input.command() {
            InlineShellCommand::Sessions if self.parallel_mode_enabled() => {
                Some("opened supersession control tower".to_string())
            }
            _ => command_input.execution_status(),
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
        self.dispatch_shell_chrome(ShellChromeEvent::HelpOverlayShown);
    }
    fn handle_turns_shell_command(&mut self, argument: Option<&str>) {
        self.dispatch_auto_follow_controls(AutoFollowControlEvent::MaxAutoTurnsUpdated {
            value: argument.unwrap_or_default().to_string(),
        });
    }
    fn handle_stop_shell_command(&mut self) {
        self.dispatch_auto_follow_controls(AutoFollowControlEvent::AutoFollowPaused);
        self.close_parallel_mode_automation_epoch();
        self.invalidate_parallel_mode_supervisor_snapshot();
        // Stop is both a local mode transition and an app-server control request:
        // disable future automation immediately, then ask the service to
        // interrupt any running native sessions.
        let status_text = match self.application.request_stop_all_sessions() {
            Ok(()) if self.conversation_has_running_turn() => {
                "stop requested / active app-server sessions will be interrupted".to_string()
            }
            Ok(()) => "stop requested / no active turn is running".to_string(),
            Err(error) => format!("stop request failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }
    pub(super) fn push_input_character(&mut self, character: char) {
        self.dispatch_conversation_input(ConversationInputEvent::CharacterTyped { character });
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
    pub(super) fn delete_previous_input_word(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::PreviousWordDeleted);
    }
    pub(super) fn clear_prompt_input(&mut self) {
        self.clear_input_buffer();
    }
    pub(super) fn is_shell_overlay_visible(&self) -> bool {
        self.shell_overlay != ShellOverlay::Hidden
    }
    pub(super) fn is_exit_confirmation_visible(&self) -> bool {
        self.exit_confirmation_state == ExitConfirmationState::Visible
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
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
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
        let is_startup_overlay = self.shell_overlay == ShellOverlay::Startup;
        // Text-field handlers get first refusal because their shortcuts must not
        // leak into overlay navigation while the cursor is inside an editor.
        if self.handle_max_auto_turns_editor_key(key) {
            return true;
        }
        if self.handle_session_search_query_editor_key(key) {
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
        if self.shell_overlay == ShellOverlay::DirectionsMaintenance {
            return self.handle_directions_overlay_key(key);
        }
        if self.shell_overlay == ShellOverlay::PlanningInit {
            return self.handle_planning_init_overlay_key(key);
        }

        self.handle_session_overlay_key(key);
        true
    }
    pub(super) fn handle_ctrl_c(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationHidden);
        if self.is_shell_overlay_visible() {
            self.close_shell_overlay();
            return;
        }

        self.dispatch_conversation_intent(ConversationIntentEvent::CtrlCPressed);
    }
}
