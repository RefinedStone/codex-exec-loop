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
            InlineShellCommand::Sessions => self.show_session_overlay(),
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
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::core::app::StartupReadySnapshot;
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    fn key(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> event::KeyEvent {
        event::KeyEvent::new(code, modifiers)
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

        app.execute_inline_shell_command_input(command(":turns 4"));
        assert_eq!(
            ready_conversation(&app)
                .auto_follow_state
                .max_auto_turns_label(),
            "4"
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

        app.execute_inline_shell_command_input(command(":stop"));
        assert!(status_text(&app).contains("no active turn is running"));

        ready_conversation_mut(&mut app).record_turn_started("turn-1".to_string());
        app.execute_inline_shell_command_input(command(":stop"));
        assert!(status_text(&app).contains("active app-server sessions"));
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
}
