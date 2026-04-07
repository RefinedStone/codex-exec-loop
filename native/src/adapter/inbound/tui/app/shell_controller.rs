use super::*;

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

impl NativeTuiApp {
    pub(super) fn can_open_session_list(&self) -> bool {
        matches!(
            &self.startup_state,
            StartupState::Ready(diagnostics) if diagnostics.can_continue()
        )
    }

    pub(super) fn shell_action_availability(&self) -> ShellActionAvailability {
        match &self.startup_state {
            StartupState::Ready(diagnostics) if diagnostics.can_continue() => {
                ShellActionAvailability::Ready
            }
            StartupState::Idle | StartupState::Loading => ShellActionAvailability::Pending,
            StartupState::Ready(_) | StartupState::Failed(_) => ShellActionAvailability::Blocked,
        }
    }

    pub(super) fn submission_blocked_status(&self, prompt_origin: PromptOrigin) -> String {
        match (prompt_origin, self.shell_action_availability()) {
            (_, ShellActionAvailability::Ready) => "ready".to_string(),
            (PromptOrigin::Manual, state) => {
                format!("{}; open diagnostics with Ctrl+d", state.status_text())
            }
            (PromptOrigin::AutoFollow, ShellActionAvailability::Pending) => {
                "auto follow-up paused while startup checks are still running".to_string()
            }
            (PromptOrigin::AutoFollow, ShellActionAvailability::Blocked) => {
                "auto follow-up paused because startup diagnostics need attention".to_string()
            }
        }
    }

    pub(super) fn conversation_has_running_turn(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation) if conversation.has_running_turn()
        )
    }

    pub(super) fn sync_draft_shell_workspace(&mut self, workspace_directory: &str) {
        let should_refresh_draft = matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if !conversation.has_active_thread() && conversation.cwd != workspace_directory
        );
        if !should_refresh_draft {
            return;
        }

        self.dispatch_followup_controls(FollowupControlEvent::DraftWorkspaceSynced {
            workspace_directory: workspace_directory.to_string(),
            template_load_result: self.load_followup_template_catalog(workspace_directory),
        });
    }

    pub(super) fn show_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayShown);
    }

    pub(super) fn show_session_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayShown {
            limit: SESSION_PAGE_SIZE,
        });
    }

    pub(super) fn show_followup_template_overlay(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::OverlayShown {
            stop_keyword: self.current_stop_keyword_value(),
        });
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayShown);
    }

    pub(super) fn toggle_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayToggled);
    }

    pub(super) fn toggle_session_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayToggled {
            limit: SESSION_PAGE_SIZE,
        });
    }

    pub(super) fn toggle_followup_template_overlay(&mut self) {
        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            self.show_followup_template_overlay();
            return;
        }
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayToggled);
    }

    pub(super) fn close_shell_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::OverlayClosed);
    }

    pub(super) fn open_new_conversation_shell(&mut self) {
        self.dispatch_conversation_intent(ConversationIntentEvent::NewDraftRequested);
    }

    pub(super) fn execute_inline_shell_command(&mut self, command: InlineShellCommand) {
        match command {
            InlineShellCommand::Diagnostics => self.show_startup_overlay(),
            InlineShellCommand::Sessions => self.show_session_overlay(),
            InlineShellCommand::Templates => self.show_followup_template_overlay(),
            InlineShellCommand::NewDraft => self.open_new_conversation_shell(),
            InlineShellCommand::Help => {}
        }

        if let Some(status_text) = command.execution_status() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: status_text.to_string(),
            });
        }
        self.clear_input_buffer();
    }

    pub(super) fn current_session(&self) -> Option<&SessionSummary> {
        match &self.session_state {
            SessionState::Ready(recent_sessions) => {
                recent_sessions.items.get(self.selected_session_index)
            }
            _ => None,
        }
    }

    pub(super) fn open_conversation_shell(&mut self) {
        self.dispatch_conversation_intent(ConversationIntentEvent::SessionOpenRequested {
            session: self.current_session().cloned(),
        });
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionSelectionMoved { delta });
    }

    pub(super) fn conversation_can_accept_input(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation) if conversation.can_submit_prompt()
        )
    }

    pub(super) fn push_input_character(&mut self, character: char) {
        self.dispatch_conversation_input(ConversationInputEvent::CharacterTyped { character });
    }

    pub(super) fn insert_input_newline(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::NewlineInserted);
    }

    pub(super) fn pop_input_character(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::BackspacePressed);
    }

    pub(super) fn toggle_auto_followup(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowToggled);
    }

    pub(super) fn current_stop_keyword_value(&self) -> String {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .auto_follow_state
                .stop_keyword_value()
                .to_string(),
            ConversationState::Loading | ConversationState::Failed(_) => {
                DEFAULT_AUTO_FOLLOW_STOP_KEYWORD.to_string()
            }
        }
    }

    pub(super) fn is_stop_keyword_editing(&self) -> bool {
        self.followup_overlay_ui_state
            .stop_keyword_editor
            .is_editing
    }

    pub(super) fn start_stop_keyword_edit(&mut self) {
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            self.show_followup_template_overlay();
        }

        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordEditStarted {
            current_value: self.current_stop_keyword_value(),
        });
    }

    pub(super) fn save_stop_keyword_edit(&mut self) {
        if !self.is_stop_keyword_editing() {
            return;
        }

        self.dispatch_followup_controls(FollowupControlEvent::StopKeywordValueUpdated {
            value: self
                .followup_overlay_ui_state
                .stop_keyword_editor
                .buffer
                .clone(),
        });
    }

    pub(super) fn cancel_stop_keyword_edit(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordEditCanceled {
            current_value: self.current_stop_keyword_value(),
        });
    }

    pub(super) fn push_stop_keyword_character(&mut self, character: char) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordCharacterTyped {
            character,
        });
    }

    pub(super) fn pop_stop_keyword_character(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordBackspacePressed);
    }

    pub(super) fn toggle_stop_keyword(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::StopKeywordToggled);
    }

    pub(super) fn toggle_no_file_change_stop(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::NoFileChangeStopToggled);
    }

    pub(super) fn reset_transcript_viewport(&mut self) {
        self.transcript_viewport_state = TranscriptViewportState::default();
    }

    pub(super) fn sync_transcript_viewport_metrics(
        &mut self,
        max_scroll_offset: u16,
        visible_height: u16,
    ) -> u16 {
        self.transcript_viewport_state
            .sync_metrics(max_scroll_offset, visible_height);
        self.transcript_viewport_state.current_scroll_offset()
    }

    pub(super) fn transcript_viewport_status_label(&self) -> String {
        self.transcript_viewport_state.status_label()
    }

    pub(super) fn scroll_transcript_page_up(&mut self) {
        self.transcript_viewport_state.scroll_page_up();
    }

    pub(super) fn scroll_transcript_page_down(&mut self) {
        self.transcript_viewport_state.scroll_page_down();
    }

    pub(super) fn scroll_transcript_to_top(&mut self) {
        self.transcript_viewport_state.scroll_to_top();
    }

    pub(super) fn scroll_transcript_to_tail(&mut self) {
        self.transcript_viewport_state.scroll_to_tail();
    }

    pub(super) fn cycle_auto_followup_template(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::TemplateCycledForward);
    }

    pub(super) fn cycle_auto_followup_template_backward(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::TemplateCycledBackward);
    }

    #[cfg(test)]
    pub(super) fn followup_template_selection(&self) -> Option<usize> {
        match &self.conversation_state {
            ConversationState::Ready(conversation)
                if !conversation
                    .auto_follow_state
                    .template_state
                    .items
                    .is_empty() =>
            {
                Some(conversation.auto_follow_state.selected_template_index())
            }
            _ => None,
        }
    }

    pub(super) fn scroll_followup_template_preview(&mut self, delta: i32) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::PreviewScrolled { delta });
    }

    pub(super) fn current_workspace_directory(&self) -> String {
        match &self.startup_state {
            StartupState::Ready(diagnostics) => diagnostics.workspace_path.clone(),
            _ => std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| ".".to_string()),
        }
    }

    pub(super) fn load_followup_template_catalog(
        &self,
        workspace_directory: &str,
    ) -> FollowupTemplateCatalogLoadResult {
        self.followup_template_service
            .load_catalog(workspace_directory)
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

    pub(super) fn handle_stop_keyword_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::FollowupTemplates || !self.is_stop_keyword_editing()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.save_stop_keyword_edit(),
            KeyCode::Esc => self.cancel_stop_keyword_edit(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_stop_keyword_edit()
            }
            KeyCode::Backspace => self.pop_stop_keyword_character(),
            KeyCode::Char(character)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.push_stop_keyword_character(character);
            }
            _ => {}
        }

        true
    }

    pub(super) fn handle_shell_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay == ShellOverlay::Hidden {
            return false;
        }
        let is_startup_overlay = self.shell_overlay == ShellOverlay::Startup;

        if self.handle_stop_keyword_editor_key(key) {
            return true;
        }

        if key.code == KeyCode::Esc
            || (key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c'))
        {
            self.close_shell_overlay();
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

        if self.shell_overlay == ShellOverlay::FollowupTemplates {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                    self.cycle_auto_followup_template_backward()
                }
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.cycle_auto_followup_template()
                }
                KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => {
                    self.cycle_auto_followup_template()
                }
                KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_auto_followup()
                }
                KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL => {
                    self.start_stop_keyword_edit()
                }
                KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_stop_keyword()
                }
                KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_no_file_change_stop()
                }
                KeyCode::PageUp if key.modifiers.is_empty() => self
                    .scroll_followup_template_preview(
                        -(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                    ),
                KeyCode::PageDown if key.modifiers.is_empty() => self
                    .scroll_followup_template_preview(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => self
                    .scroll_followup_template_preview(
                        -(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                    ),
                KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => self
                    .scroll_followup_template_preview(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                KeyCode::Enter if key.modifiers.is_empty() => self.close_shell_overlay(),
                _ => {}
            }
            return true;
        }

        match key.code {
            KeyCode::Char('r') if key.modifiers.is_empty() => {
                if self.can_open_session_list() {
                    self.dispatch_shell_chrome(ShellChromeEvent::SessionsRequested {
                        limit: SESSION_PAGE_SIZE,
                    });
                }
            }
            KeyCode::Char('n') if key.modifiers.is_empty() => {
                self.open_new_conversation_shell();
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.move_selection(1)
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.open_conversation_shell(),
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.show_startup_overlay()
            }
            _ => {}
        }
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
